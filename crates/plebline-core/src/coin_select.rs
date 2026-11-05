//! Coin selection for the batch. The selector does not see any outpoint
//! already committed, so two transactions cannot pick the same coin.

use core::fmt;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use bdk_wallet::bitcoin::{Amount, FeeRate, OutPoint, Script, TxIn, Weight};
use bdk_wallet::coin_selection::{
    decide_change, CoinSelectionAlgorithm, CoinSelectionResult, InsufficientFunds,
};
use bdk_wallet::WeightedUtxo;
use rand_core::RngCore;

/// Outpoints already spent by a transaction of this batch or of an earlier
/// signed batch.
#[derive(Debug, Clone, Default)]
pub struct CommittedOutpoints(BTreeSet<OutPoint>);

impl CommittedOutpoints {
    /// Pre-committed with inputs of earlier batches, so two ceremonies over the
    /// same `utxos.json` cannot conflict.
    pub fn seeded(outpoints: impl IntoIterator<Item = OutPoint>) -> Self {
        // Duplicates in the starting set are harmless.
        Self(outpoints.into_iter().collect())
    }

    /// Commits the outpoints; `Err` with the duplicates means the invariant is
    /// already broken.
    pub fn commit(
        &mut self,
        outpoints: impl IntoIterator<Item = OutPoint>,
    ) -> Result<(), Vec<OutPoint>> {
        let mut duplicates = Vec::new();
        for op in outpoints {
            if !self.0.insert(op) {
                duplicates.push(op);
            }
        }
        if duplicates.is_empty() {
            Ok(())
        } else {
            Err(duplicates)
        }
    }

    pub fn contains(&self, outpoint: &OutPoint) -> bool {
        self.0.contains(outpoint)
    }

    pub fn snapshot(&self) -> BTreeSet<OutPoint> {
        self.0.clone()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Violations found during selection. `CoinSelectionAlgorithm` can only
/// return `InsufficientFunds`, so they are read after `finish()`; the log is
/// shared because `TxBuilder` takes the selector by value.
///
/// No caller passes required coins today (nothing calls `add_utxo`): this is
/// here so one that did could not slip a committed coin in.
#[derive(Debug, Clone, Default)]
pub struct SelectionViolations(Rc<RefCell<Vec<OutPoint>>>);

impl SelectionViolations {
    fn record(&self, outpoint: OutPoint) {
        self.0.borrow_mut().push(outpoint);
    }

    /// Required outpoints dropped because already committed. Non-empty means
    /// the transaction is not the one asked for.
    pub fn rejected_required(&self) -> Vec<OutPoint> {
        self.0.borrow().clone()
    }

    pub fn is_clean(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

/// Deterministic selector that excludes every committed outpoint.
///
/// Ordering is total (value descending, then outpoint) so `plan` (public
/// descriptor) and `sign` (private key) select the same coins.
#[derive(Clone)]
pub struct DisjointCoinSelection {
    forbidden: BTreeSet<OutPoint>,
    violations: SelectionViolations,
}

impl fmt::Debug for DisjointCoinSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisjointCoinSelection")
            .field("forbidden", &self.forbidden.len())
            .finish()
    }
}

impl DisjointCoinSelection {
    /// Selector that excludes a fixed set of outpoints.
    pub fn excluding(forbidden: BTreeSet<OutPoint>) -> Self {
        Self {
            forbidden,
            violations: SelectionViolations::default(),
        }
    }

    /// Selector that excludes everything already committed in the batch.
    pub fn for_batch(committed: &CommittedOutpoints) -> Self {
        Self::excluding(committed.snapshot())
    }

    /// Keep this before `TxBuilder` consumes the selector.
    pub fn violations(&self) -> SelectionViolations {
        self.violations.clone()
    }

    pub fn forbidden(&self) -> &BTreeSet<OutPoint> {
        &self.forbidden
    }

    fn is_forbidden(&self, utxo: &WeightedUtxo) -> bool {
        self.forbidden.contains(&utxo.utxo.outpoint())
    }
}

impl CoinSelectionAlgorithm for DisjointCoinSelection {
    fn coin_select<R: RngCore>(
        &self,
        required_utxos: Vec<WeightedUtxo>,
        optional_utxos: Vec<WeightedUtxo>,
        fee_rate: FeeRate,
        target_amount: Amount,
        drain_script: &Script,
        _rand: &mut R,
    ) -> Result<CoinSelectionResult, InsufficientFunds> {
        // A forbidden `required` coin is dropped and recorded, so the caller
        // can refuse the transaction.
        let mut required: Vec<WeightedUtxo> = Vec::with_capacity(required_utxos.len());
        for utxo in required_utxos {
            if self.is_forbidden(&utxo) {
                self.violations.record(utxo.utxo.outpoint());
            } else {
                required.push(utxo);
            }
        }

        let mut optional: Vec<WeightedUtxo> = optional_utxos
            .into_iter()
            .filter(|u| !self.is_forbidden(u))
            .collect();

        // Largest first (ideally one input per tx), outpoint as tie-break.
        optional.sort_by(|a, b| {
            let (av, bv) = (a.utxo.txout().value, b.utxo.txout().value);
            bv.cmp(&av)
                .then_with(|| a.utxo.outpoint().cmp(&b.utxo.outpoint()))
        });

        let available: Amount = required
            .iter()
            .chain(optional.iter())
            .map(|u| u.utxo.txout().value)
            .sum();

        let utxos = required
            .into_iter()
            .map(|u| (true, u))
            .chain(optional.into_iter().map(|u| (false, u)));

        select_in_order(utxos, fee_rate, target_amount, drain_script, available)
    }
}

/// A copy of BDK's private `select_sorted_utxos`, except that `available`
/// reports the total spendable instead of the running sum.
fn select_in_order(
    utxos: impl Iterator<Item = (bool, WeightedUtxo)>,
    fee_rate: FeeRate,
    target_amount: Amount,
    drain_script: &Script,
    available: Amount,
) -> Result<CoinSelectionResult, InsufficientFunds> {
    let mut selected_amount = Amount::ZERO;
    let mut total_weight = Weight::ZERO;
    let mut selected = Vec::new();

    for (must_use, weighted_utxo) in utxos {
        if !must_use && selected_amount >= target_amount + fee_rate * total_weight {
            continue;
        }
        total_weight = total_weight
            .checked_add(
                TxIn::default()
                    .segwit_weight()
                    .checked_add(weighted_utxo.satisfaction_weight)
                    .expect("input weight overflow"),
            )
            .expect("total weight overflow");
        selected_amount += weighted_utxo.utxo.txout().value;
        selected.push(weighted_utxo.utxo);
    }

    let fee_amount = fee_rate * total_weight;
    let needed = target_amount + fee_amount;
    if selected_amount < needed {
        return Err(InsufficientFunds { needed, available });
    }

    let excess = decide_change(selected_amount - needed, fee_rate, drain_script);

    Ok(CoinSelectionResult {
        selected,
        fee_amount,
        excess,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::txid;
    use bdk_wallet::bitcoin::{ScriptBuf, TxOut};
    use bdk_wallet::chain::{BlockId, ChainPosition, ConfirmationBlockTime};
    use bdk_wallet::{KeychainKind, LocalOutput, Utxo};
    use rand_core::OsRng;
    use std::str::FromStr;

    fn utxo(n: u8, sats: u64) -> WeightedUtxo {
        let outpoint = OutPoint::new(txid(n), 0);
        WeightedUtxo {
            satisfaction_weight: Weight::from_wu(107),
            utxo: Utxo::Local(LocalOutput {
                outpoint,
                txout: TxOut {
                    value: Amount::from_sat(sats),
                    script_pubkey: ScriptBuf::new(),
                },
                keychain: KeychainKind::External,
                is_spent: false,
                derivation_index: n as u32,
                chain_position: ChainPosition::Confirmed {
                    anchor: ConfirmationBlockTime {
                        block_id: BlockId {
                            height: 100,
                            hash: bdk_wallet::bitcoin::BlockHash::from_str(
                                "0000000000000000000000000000000000000000000000000000000000000000",
                            )
                            .unwrap(),
                        },
                        confirmation_time: 100,
                    },
                    transitively: None,
                },
            }),
        }
    }

    /// Fixed fee, target and change script; tests differ only in coins.
    fn select(
        cs: &DisjointCoinSelection,
        required: Vec<WeightedUtxo>,
        optional: Vec<WeightedUtxo>,
    ) -> Result<CoinSelectionResult, InsufficientFunds> {
        cs.coin_select(
            required,
            optional,
            FeeRate::from_sat_per_vb(1).unwrap(),
            Amount::from_sat(50_000),
            ScriptBuf::new().as_script(),
            &mut OsRng,
        )
    }

    fn selected_outpoints(res: &CoinSelectionResult) -> BTreeSet<OutPoint> {
        res.selected.iter().map(|u| u.outpoint()).collect()
    }

    #[test]
    fn forbidden_outpoints_are_excluded_from_the_optional() {
        let forbidden = BTreeSet::from([OutPoint::new(txid(1), 0)]);
        let cs = DisjointCoinSelection::excluding(forbidden.clone());

        let res = select(&cs, vec![], vec![utxo(1, 100_000), utxo(2, 90_000)]).unwrap();

        let picked = selected_outpoints(&res);
        assert!(
            picked.is_disjoint(&forbidden),
            "ha selezionato un outpoint proibito"
        );
        assert!(picked.contains(&OutPoint::new(txid(2), 0)));
    }

    #[test]
    fn a_forbidden_required_is_dropped_and_recorded() {
        let forbidden = BTreeSet::from([OutPoint::new(txid(1), 0)]);
        let cs = DisjointCoinSelection::excluding(forbidden.clone());
        let violations = cs.violations();

        let res = select(&cs, vec![utxo(1, 100_000)], vec![utxo(2, 90_000)]).unwrap();

        assert!(selected_outpoints(&res).is_disjoint(&forbidden));
        assert_eq!(
            violations.rejected_required(),
            vec![OutPoint::new(txid(1), 0)]
        );
        assert!(!violations.is_clean());
    }

    #[test]
    fn with_no_exclusions_there_are_no_violations() {
        let cs = DisjointCoinSelection::excluding(BTreeSet::new());
        let violations = cs.violations();
        select(&cs, vec![utxo(1, 100_000)], vec![utxo(2, 90_000)]).unwrap();
        assert!(violations.is_clean());
    }

    #[test]
    fn insufficient_funds_when_only_the_forbidden_is_left() {
        let forbidden = BTreeSet::from([OutPoint::new(txid(1), 0)]);
        let cs = DisjointCoinSelection::excluding(forbidden);

        let err = select(&cs, vec![], vec![utxo(1, 100_000), utxo(2, 1_000)]).unwrap_err();

        assert_eq!(err.available, Amount::from_sat(1_000));
    }

    /// Equal values: without the tie-break, `plan` and `sign` could pick
    /// different coins if the wallet listed them in a different order.
    #[test]
    fn equal_values_are_picked_the_same_in_any_input_order() {
        let orders = [
            [9, 3, 7],
            [9, 7, 3],
            [3, 9, 7],
            [3, 7, 9],
            [7, 9, 3],
            [7, 3, 9],
        ];
        for order in orders {
            let cs = DisjointCoinSelection::excluding(BTreeSet::new());
            let coins = order.iter().map(|&n| utxo(n, 60_000)).collect();
            let picked: Vec<OutPoint> = select(&cs, vec![], coins)
                .unwrap()
                .selected
                .iter()
                .map(|u| u.outpoint())
                .collect();
            assert_eq!(picked, vec![OutPoint::new(txid(3), 0)], "ordine {order:?}");
        }
    }

    #[test]
    fn a_coin_cannot_be_committed_twice() {
        let mut committed = CommittedOutpoints::default();
        let op = OutPoint::new(txid(1), 0);
        committed.commit([op]).unwrap();
        assert!(committed.contains(&op));

        let duplicates = committed.commit([op]).unwrap_err();
        assert_eq!(duplicates, vec![op]);
    }

    /// Shows the collision the next test prevents: without exclusions, both
    /// selections take the same coin.
    #[test]
    fn without_exclusion_two_txs_collide() {
        let pool = || vec![utxo(1, 100_000), utxo(2, 100_000)];
        let without_exclusions = || DisjointCoinSelection::excluding(BTreeSet::new());
        let a = select(&without_exclusions(), vec![], pool()).unwrap();
        let b = select(&without_exclusions(), vec![], pool()).unwrap();

        assert!(
            !selected_outpoints(&a).is_disjoint(&selected_outpoints(&b)),
            "il caso di collisione non si riproduce: il test di disgiunzione sarebbe vacuo"
        );
    }

    #[test]
    fn with_the_committed_set_two_txs_are_disjoint() {
        let mut committed = CommittedOutpoints::default();
        let pool = || vec![utxo(1, 100_000), utxo(2, 100_000)];

        let mut sets = Vec::new();
        for _ in 0..2 {
            let cs = DisjointCoinSelection::for_batch(&committed);
            let res = select(&cs, vec![], pool()).unwrap();
            let picked = selected_outpoints(&res);
            committed
                .commit(picked.iter().copied())
                .expect("nessun doppio impegno");
            sets.push(picked);
        }

        assert!(sets[0].is_disjoint(&sets[1]));
        assert_eq!(committed.len(), sets[0].len() + sets[1].len());
    }
}
