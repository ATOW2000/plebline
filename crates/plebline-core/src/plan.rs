//! Building the batch: N mutually independent transactions.
//!
//! Selection is deterministic, so the plan built from the public descriptor
//! and the one rebuilt at signing must match input for input.

use std::collections::BTreeSet;
use std::str::FromStr;

use bdk_wallet::bitcoin::{absolute::LockTime, Address, Amount, FeeRate, OutPoint, Psbt, Txid};
use bdk_wallet::{TxOrdering, Wallet};
use serde::{Deserialize, Serialize};

use crate::coin_select::{CommittedOutpoints, DisjointCoinSelection};
use crate::error::{DisjointnessError, Error, Result};
use crate::lang::t;
use crate::tf;

pub const PLAN_SCHEMA_VERSION: u32 = 1;

/// The payments to build, one transaction per entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConfig {
    pub network: String,
    pub fee_rate_sat_vb: f64,
    pub entries: Vec<BatchEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEntry {
    /// Local note for summaries; never on chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub recipient: String,
    pub amount_sat: u64,
}

/// The plan, serializable into `plan.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedBatch {
    pub version: u32,
    pub network: String,
    pub fee_rate_sat_vb: f64,
    pub source_generated_at: u64,
    pub source_tip_height: u32,
    pub txs: Vec<PlannedTx>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTx {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub recipient: String,
    pub amount_sat: u64,
    pub inputs: Vec<PlannedInput>,
    pub fee_sat: u64,
    pub fee_rate_sat_vb: f64,
    /// Change back to the wallet; `None` means it would have been dust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_sat: Option<u64>,
    pub vsize: u64,
    /// Equals the signed txid: witnesses are not part of it.
    pub txid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedInput {
    pub outpoint: String,
    pub value_sat: u64,
}

impl PlannedBatch {
    pub fn total_amount(&self) -> u64 {
        self.txs.iter().map(|t| t.amount_sat).sum()
    }

    pub fn total_fee(&self) -> u64 {
        self.txs.iter().map(|t| t.fee_sat).sum()
    }

    /// Every outpoint spent by the batch, in transaction order.
    pub fn inputs_by_tx(&self) -> Vec<Vec<String>> {
        self.txs
            .iter()
            .map(|t| t.inputs.iter().map(|i| i.outpoint.clone()).collect())
            .collect()
    }
}

/// A transaction built but not yet signed.
#[derive(Debug)]
pub struct BuiltTx {
    pub index: usize,
    pub label: Option<String>,
    pub recipient: Address,
    pub amount: Amount,
    pub psbt: Psbt,
    pub inputs: Vec<(OutPoint, Amount)>,
    pub fee: Amount,
    pub change: Option<Amount>,
    pub vsize: u64,
    pub txid: Txid,
}

/// Builds the N transactions. Each selector sees none of the inputs used
/// by earlier ones, nor `already_committed` (inputs of past, unbroadcast
/// batches).
pub fn build(
    wallet: &mut Wallet,
    config: &BatchConfig,
    already_committed: impl IntoIterator<Item = OutPoint>,
) -> Result<Vec<BuiltTx>> {
    let network = wallet.network();
    let fee_rate = fee_rate_from_sat_per_vb(config.fee_rate_sat_vb)?;
    let mut committed = CommittedOutpoints::seeded(already_committed);
    let mut built = Vec::with_capacity(config.entries.len());

    for (index, entry) in config.entries.iter().enumerate() {
        let recipient = Address::from_str(&entry.recipient)
            .map_err(|e| Error::BuildTx {
                index,
                reason: tf!(
                    { "indirizzo '{}' non valido: {e}", entry.recipient },
                    { "address '{}' is not valid: {e}", entry.recipient }
                ),
            })?
            .require_network(network)
            .map_err(|_| Error::BuildTx {
                index,
                reason: tf!(
                    { "l'indirizzo '{}' non appartiene alla rete {network}", entry.recipient },
                    { "address '{}' does not belong to network {network}", entry.recipient }
                ),
            })?;

        // Above the total supply BDK's own sums panic, so refuse it here.
        if entry.amount_sat > Amount::MAX_MONEY.to_sat() {
            let shown = entry.amount_sat;
            return Err(Error::BuildTx {
                index,
                reason: tf!(
                    { "{shown} sat sono più di tutti i bitcoin che esisteranno mai" },
                    { "{shown} sat is more than all the bitcoin there will ever be" }
                ),
            });
        }
        let amount = Amount::from_sat(entry.amount_sat);

        let selector = DisjointCoinSelection::for_batch(&committed);
        let violations = selector.violations();
        let forbidden: Vec<OutPoint> = selector.forbidden().iter().copied().collect();

        let psbt = {
            let mut builder = wallet.build_tx().coin_selection(selector);
            builder
                .fee_rate(fee_rate)
                .add_recipient(recipient.script_pubkey(), amount)
                // Reproducible byte for byte at signing time.
                .ordering(TxOrdering::Untouched)
                // Zero, not the height of the tip: a height months old at
                // broadcast would mark the payment as presigned.
                .nlocktime(LockTime::ZERO)
                // The committed coins are also marked unspendable, in case
                // the selector lets one through.
                .unspendable(forbidden);
            builder.finish().map_err(|e| Error::BuildTx {
                index,
                reason: e.to_string(),
            })?
        };

        if !violations.is_clean() {
            return Err(DisjointnessError::RejectedRequired {
                index,
                outpoints: violations.rejected_required(),
            }
            .into());
        }

        let unsigned = psbt.unsigned_tx.clone();
        let spent: Vec<OutPoint> = unsigned.input.iter().map(|i| i.previous_output).collect();

        // Only coins listed in the file may be spent; hydrate locked the others.
        crate::utxos::refuse_unlisted(wallet, spent.iter().copied(), index)?;

        // Commit the inputs the PSBT actually spends; one already used by an
        // earlier transaction stops the batch.
        if let Err(duplicates) = committed.commit(spent.iter().copied()) {
            let outpoint = duplicates[0];
            let previous = built
                .iter()
                .position(|t: &BuiltTx| t.inputs.iter().any(|(op, _)| *op == outpoint));
            return Err(match previous {
                Some(tx_a) => DisjointnessError::SharedInput {
                    outpoint,
                    tx_a,
                    tx_b: index,
                },
                None => DisjointnessError::AlreadySigned { outpoint, index },
            }
            .into());
        }

        let inputs: Vec<(OutPoint, Amount)> =
            spent
                .iter()
                .map(|op| {
                    let value = wallet.get_utxo(*op).map(|u| u.txout.value).ok_or_else(|| {
                        Error::BuildTx {
                            index,
                            reason: tf!({ "input {op} sconosciuto al wallet" }, {
                                "input {op} unknown to the wallet"
                            }),
                        }
                    })?;
                    Ok((*op, value))
                })
                .collect::<Result<_>>()?;

        let input_total: Amount = inputs.iter().map(|(_, v)| *v).sum();
        let output_total: Amount = unsigned.output.iter().map(|o| o.value).sum();
        let fee = input_total
            .checked_sub(output_total)
            .ok_or_else(|| Error::BuildTx {
                index,
                reason: t("output superiori agli input", "outputs exceed the inputs").into(),
            })?;

        let recipient_spk = recipient.script_pubkey();
        let change = unsigned
            .output
            .iter()
            .find(|o| o.script_pubkey != recipient_spk)
            .map(|o| o.value);

        built.push(BuiltTx {
            index,
            label: entry.label.clone(),
            recipient,
            amount,
            fee,
            change,
            vsize: predicted_vsize(wallet, &unsigned, &inputs, index)?,
            inputs,
            txid: unsigned.compute_txid(),
            psbt,
        });
    }

    // Final check on the whole batch, independent of construction.
    let spending: Vec<_> = built
        .iter()
        .map(|t| {
            (
                t.index,
                t.txid,
                t.inputs.iter().map(|(op, _)| *op).collect(),
            )
        })
        .collect();
    check_disjoint(&spending)?;

    Ok(built)
}

/// Signed vsize: unsigned size plus the descriptor's max satisfaction
/// weight and the 2 WU SegWit marker/flag. Otherwise the fee rate reads high.
pub(crate) fn predicted_vsize(
    wallet: &Wallet,
    unsigned: &bdk_wallet::bitcoin::Transaction,
    inputs: &[(OutPoint, Amount)],
    index: usize,
) -> Result<u64> {
    let mut weight = unsigned.weight();
    for (outpoint, _) in inputs {
        let keychain = wallet
            .get_utxo(*outpoint)
            .map(|utxo| utxo.keychain)
            .ok_or_else(|| Error::BuildTx {
                index,
                reason: tf!({ "input {outpoint} sconosciuto al wallet" }, {
                    "input {outpoint} unknown to the wallet"
                }),
            })?;
        let satisfaction = wallet
            .public_descriptor(keychain)
            .max_weight_to_satisfy()
            .map_err(|e| Error::BuildTx {
                index,
                reason: tf!({ "peso di soddisfazione non calcolabile: {e}" }, {
                    "satisfaction weight cannot be computed: {e}"
                }),
            })?;
        weight += satisfaction;
    }
    weight += bdk_wallet::bitcoin::Weight::from_wu(2);
    Ok(weight.to_vbytes_ceil())
}

/// Broadcastable in any order: no shared outpoint, no transaction spending
/// another's output. Each entry is a transaction's `index`, its txid and the
/// coins it spends; errors name the `index`, which after a withdrawal is not
/// the position.
pub fn check_disjoint(
    txs: &[(usize, Txid, Vec<OutPoint>)],
) -> std::result::Result<(), DisjointnessError> {
    let mut owner: std::collections::BTreeMap<OutPoint, usize> = Default::default();
    for (index, _, inputs) in txs {
        let index = *index;
        let mut within: BTreeSet<OutPoint> = BTreeSet::new();
        for op in inputs {
            // Duplicate input within one transaction: a bug anyway.
            if !within.insert(*op) {
                return Err(DisjointnessError::SharedInput {
                    outpoint: *op,
                    tx_a: index,
                    tx_b: index,
                });
            }
            if let Some(previous) = owner.insert(*op, index) {
                return Err(DisjointnessError::SharedInput {
                    outpoint: *op,
                    tx_a: previous,
                    tx_b: index,
                });
            }
        }
    }

    for (op, child) in &owner {
        if let Some((parent, _, _)) = txs.iter().find(|(_, txid, _)| *txid == op.txid) {
            return Err(DisjointnessError::ChainedTx {
                outpoint: *op,
                parent: *parent,
                child: *child,
            });
        }
    }

    Ok(())
}

pub fn to_plan(
    built: &[BuiltTx],
    config: &BatchConfig,
    generated_at: u64,
    tip: u32,
) -> PlannedBatch {
    PlannedBatch {
        version: PLAN_SCHEMA_VERSION,
        network: config.network.clone(),
        fee_rate_sat_vb: config.fee_rate_sat_vb,
        source_generated_at: generated_at,
        source_tip_height: tip,
        txs: built
            .iter()
            .map(|t| PlannedTx {
                index: t.index,
                label: t.label.clone(),
                recipient: t.recipient.to_string(),
                amount_sat: t.amount.to_sat(),
                inputs: t
                    .inputs
                    .iter()
                    .map(|(op, v)| PlannedInput {
                        outpoint: op.to_string(),
                        value_sat: v.to_sat(),
                    })
                    .collect(),
                fee_sat: t.fee.to_sat(),
                fee_rate_sat_vb: t.fee.to_sat() as f64 / t.vsize as f64,
                change_sat: t.change.map(|c| c.to_sat()),
                vsize: t.vsize,
                txid: t.txid.to_string(),
            })
            .collect(),
    }
}

pub fn fee_rate_from_sat_per_vb(sat_per_vb: f64) -> Result<FeeRate> {
    if !sat_per_vb.is_finite() || sat_per_vb < 1.0 {
        return Err(Error::Input(tf!(
            {
                "fee rate {sat_per_vb} sat/vB non valido: il minimo per la propagazione è 1 sat/vB"
            },
            { "fee rate {sat_per_vb} sat/vB is not valid: the minimum to propagate is 1 sat/vB" }
        )));
    }
    // The ceiling the prompts show and verification applies. Above it the
    // fee arithmetic can overflow (a panic, or a wrapped fee in release).
    let max = crate::verify::MAX_SANE_FEE_RATE;
    if sat_per_vb > max {
        return Err(Error::Input(tf!(
            {
                "fee rate {sat_per_vb} sat/vB non plausibile: oltre {max} è quasi sempre un \
                 errore di battitura"
            },
            {
                "fee rate {sat_per_vb} sat/vB is not plausible: above {max} it is almost always a typo"
            }
        )));
    }
    Ok(FeeRate::from_sat_per_kwu(
        (sat_per_vb * 250.0).round() as u64
    ))
}
