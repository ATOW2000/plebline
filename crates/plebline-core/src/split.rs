//! Split transaction: turns a few large coins into many small ones.
//!
//! Each presigned payment needs its own input, so N payments need N coins.
//! One self-transfer, broadcast and confirmed before the ceremony. It spends
//! every free coin; coins inside signed payments are left alone, or sending
//! the split would silently cancel those payments.

use std::collections::BTreeSet;

use bdk_wallet::bitcoin::{absolute::LockTime, Address, Amount, OutPoint, Psbt, Txid};
use bdk_wallet::{KeychainKind, TxOrdering, Wallet};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::lang::t;
use crate::plan::fee_rate_from_sat_per_vb;
use crate::tf;

/// Smallest useful output. Well above dust (294 sat for P2WPKH): a coin
/// barely worth its spending fee is useless.
pub const MIN_USEFUL_OUTPUT_SAT: u64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitConfig {
    pub network: String,
    pub outputs: usize,
    pub fee_rate_sat_vb: f64,
}

pub struct SplitPlan {
    pub psbt: Psbt,
    pub inputs: Vec<(OutPoint, Amount)>,
    pub outputs: Vec<(Address, Amount)>,
    pub fee: Amount,
    pub vsize: u64,
    pub txid: Txid,
}

impl SplitPlan {
    pub fn total_in(&self) -> Amount {
        self.inputs.iter().map(|(_, v)| *v).sum()
    }

    pub fn total_out(&self) -> Amount {
        self.outputs.iter().map(|(_, v)| *v).sum()
    }
}

/// Coins one split can make: enough for any realistic number of refills,
/// and the transaction stays small.
pub const MAX_OUTPUTS: usize = 64;

/// Maximum outputs for a balance and fee rate, shown before trying.
pub fn max_outputs(total: Amount, input_count: usize, fee_rate_sat_vb: f64) -> usize {
    let mut best = 0;
    for n in 1..=MAX_OUTPUTS {
        let fee = estimated_fee(input_count, n, fee_rate_sat_vb);
        let Some(spendable) = total.to_sat().checked_sub(fee) else {
            break;
        };
        if spendable / n as u64 >= MIN_USEFUL_OUTPUT_SAT {
            best = n;
        } else {
            break;
        }
    }
    best
}

/// P2WPKH size estimate (68 vB/input, 31/output, 11 header). Only sizes the
/// outputs; the last one absorbs the real fee.
fn estimated_vsize(inputs: usize, outputs: usize) -> u64 {
    (inputs as u64 * 68) + (outputs as u64 * 31) + 11
}

fn estimated_fee(inputs: usize, outputs: usize, fee_rate_sat_vb: f64) -> u64 {
    (estimated_vsize(inputs, outputs) as f64 * fee_rate_sat_vb).ceil() as u64
}

/// Spends every free coin into `outputs` fresh addresses of the same
/// wallet. No change: the last output takes the remainder.
/// `already_committed`: inputs of signed, unbroadcast batches; never spent.
pub fn build(
    wallet: &mut Wallet,
    config: &SplitConfig,
    already_committed: impl IntoIterator<Item = OutPoint>,
) -> Result<SplitPlan> {
    let committed: BTreeSet<OutPoint> = already_committed.into_iter().collect();
    if config.outputs < 2 {
        return Err(Error::Input(
            t(
                "dividere in meno di due monete non ha senso",
                "splitting into fewer than two coins makes no sense",
            )
            .into(),
        ));
    }

    // `list_unspent` ignores locks, so the coins hydrate locked (those not in
    // the file) are filtered out here.
    let available: Vec<_> = wallet
        .list_unspent()
        .filter(|u| !committed.contains(&u.outpoint) && !wallet.is_outpoint_locked(u.outpoint))
        .collect();
    if available.is_empty() {
        return Err(Error::Input(
            t(
                "il wallet non ha monete libere da dividere",
                "the wallet has no free coins to split",
            )
            .into(),
        ));
    }

    let total: Amount = available.iter().map(|u| u.txout.value).sum();
    let input_count = available.len();
    let wanted = config.outputs;

    if wanted > MAX_OUTPUTS {
        return Err(Error::Input(tf!(
            { "una divisione fa al massimo {MAX_OUTPUTS} monete, non {wanted}" },
            { "one split makes at most {MAX_OUTPUTS} coins, not {wanted}" }
        )));
    }
    let possible = max_outputs(total, input_count, config.fee_rate_sat_vb);
    if wanted > possible {
        let amount = total.to_sat();
        return Err(Error::Input(tf!(
            {
                "con {amount} sat su {input_count} monete non si arriva a {wanted} output: \
                 al massimo {possible}, perché sotto {MIN_USEFUL_OUTPUT_SAT} sat una moneta \
                 costa più di quanto valga"
            },
            {
                "with {amount} sat across {input_count} coins you cannot reach {wanted} outputs: \
                 at most {possible}, because below {MIN_USEFUL_OUTPUT_SAT} sat a coin costs more \
                 than it is worth"
            }
        )));
    }

    let fee_rate = fee_rate_from_sat_per_vb(config.fee_rate_sat_vb)?;
    let fee_guess = estimated_fee(input_count, wanted, config.fee_rate_sat_vb);
    let per_output = (total.to_sat() - fee_guess) / wanted as u64;

    // Consecutive addresses stay within the gap limit.
    let addresses: Vec<Address> = (0..wanted)
        .map(|_| wallet.reveal_next_address(KeychainKind::External).address)
        .collect();

    let psbt = {
        let mut builder = wallet.build_tx();
        builder
            .fee_rate(fee_rate)
            .ordering(TxOrdering::Untouched)
            .nlocktime(LockTime::ZERO)
            // All free coins: a partial split would leave an unasked-for
            // change coin. `drain_wallet` honours `unspendable`.
            .unspendable(committed.iter().copied().collect())
            .drain_wallet();

        for address in addresses.iter().take(wanted - 1) {
            builder.add_recipient(address.script_pubkey(), Amount::from_sat(per_output));
        }
        builder.drain_to(addresses[wanted - 1].script_pubkey());

        builder.finish().map_err(|e| Error::BuildTx {
            index: 0,
            reason: e.to_string(),
        })?
    };

    let unsigned = psbt.unsigned_tx.clone();
    let inputs: Vec<(OutPoint, Amount)> = unsigned
        .input
        .iter()
        .map(|i| {
            let outpoint = i.previous_output;
            let value = wallet
                .get_utxo(outpoint)
                .map(|u| u.txout.value)
                .ok_or_else(|| Error::BuildTx {
                    index: 0,
                    reason: tf!({ "input {outpoint} sconosciuto al wallet" }, {
                        "input {outpoint} unknown to the wallet"
                    }),
                })?;
            Ok((outpoint, value))
        })
        .collect::<Result<_>>()?;

    // Checked again on the inputs the PSBT ended up with: only listed coins,
    // and none already inside a signed refill.
    crate::utxos::refuse_unlisted(wallet, inputs.iter().map(|(op, _)| *op), 0)?;
    if let Some((outpoint, _)) = inputs.iter().find(|(op, _)| committed.contains(op)) {
        return Err(Error::BuildTx {
            index: 0,
            reason: tf!(
                { "la divisione spenderebbe {outpoint}, già dentro una ricarica firmata" },
                { "the split would spend {outpoint}, already inside a signed refill" }
            ),
        });
    }

    let network = wallet.network();
    let outputs: Vec<(Address, Amount)> = unsigned
        .output
        .iter()
        .map(|o| {
            let address =
                Address::from_script(&o.script_pubkey, network).map_err(|e| Error::BuildTx {
                    index: 0,
                    reason: tf!({ "output non rappresentabile come indirizzo: {e}" }, {
                        "output cannot be represented as an address: {e}"
                    }),
                })?;
            Ok((address, o.value))
        })
        .collect::<Result<_>>()?;

    // Every output must be ours, or it is a payment.
    for (address, _) in &outputs {
        if !wallet.is_mine(address.script_pubkey()) {
            return Err(Error::BuildTx {
                index: 0,
                reason: tf!(
                    {
                        "l'output verso {address} non appartiene a questo wallet: \
                         la divisione manderebbe monete fuori"
                    },
                    {
                        "the output to {address} does not belong to this wallet: \
                         the split would send coins away"
                    }
                ),
            });
        }
    }

    let fee = wallet
        .calculate_fee(&unsigned)
        .map_err(|e| Error::BuildTx {
            index: 0,
            reason: e.to_string(),
        })?;

    // `unsigned.vsize()` has no witnesses yet and would make the fee rate
    // read too high.
    let vsize = crate::plan::predicted_vsize(wallet, &unsigned, &inputs, 0)?;

    Ok(SplitPlan {
        txid: unsigned.compute_txid(),
        vsize,
        psbt,
        inputs,
        outputs,
        fee,
    })
}

/// Signs the split as a one-transaction batch, so verify and broadcast
/// need no special case.
pub fn sign(
    wallet: &Wallet,
    plan: SplitPlan,
    config: &SplitConfig,
) -> Result<crate::batch::SignedBatch> {
    let built = crate::plan::BuiltTx {
        index: 0,
        label: Some(t("divisione delle monete", "coin split").to_string()),
        // `batch::sign` needs a recipient; all outputs are ours anyway.
        recipient: plan.outputs[0].0.clone(),
        amount: plan.outputs[0].1,
        psbt: plan.psbt,
        inputs: plan.inputs,
        fee: plan.fee,
        change: None,
        vsize: plan.vsize,
        txid: plan.txid,
    };

    let mut signed = crate::batch::sign(wallet, vec![built], config.fee_rate_sat_vb)?;
    // Every output comes back to this wallet: none of them is a payment.
    for output in &mut signed.txs[0].outputs {
        output.is_change = true;
    }
    Ok(signed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_drops_as_outputs_grow() {
        let total = Amount::from_sat(1_000_000);
        assert!(max_outputs(total, 2, 1.0) >= 10);
        assert_eq!(max_outputs(Amount::from_sat(25_000), 1, 1.0), 2);
        assert_eq!(max_outputs(Amount::from_sat(5_000), 1, 1.0), 0);
    }

    /// One input, two outputs: 68 + 2 × 31 + 11 = 141 vB.
    #[test]
    fn the_estimate_uses_p2wpkh_sizes() {
        assert_eq!(estimated_fee(1, 2, 2.0), 282);
    }
}
