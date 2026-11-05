//! Batch signing and the `batch.json` format, self-contained enough for a
//! third party to re-check amounts, fees and disjointness offline.

use bdk_wallet::bitcoin::consensus::encode::serialize_hex;
use bdk_wallet::bitcoin::{Address, Amount, Network, Script};
use bdk_wallet::{SignOptions, Wallet};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::lang::t;
use crate::plan::BuiltTx;
use crate::tf;

pub const BATCH_SCHEMA_VERSION: u32 = 1;

/// The payment number people see: the first is 1. `index`, in files and in
/// the `tx-K.hex` names, starts at 0.
pub fn number(index: usize) -> usize {
    // `index` may come from a hostile file, so overflow must not panic.
    index.saturating_add(1)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedBatch {
    pub version: u32,
    pub network: String,
    pub created_by: String,
    /// Identifies the signing cold storage.
    pub descriptor_checksum: String,
    pub fee_rate_sat_vb: f64,
    pub txs: Vec<SignedTx>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTx {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub txid: String,
    /// Finalized transaction, ready to broadcast.
    pub hex: String,
    /// Same transaction as a PSBT (base64), for other tools.
    pub psbt: String,
    pub inputs: Vec<SignedInput>,
    pub outputs: Vec<SignedOutput>,
    pub fee_sat: u64,
    pub fee_rate_sat_vb: f64,
    pub vsize: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedInput {
    pub outpoint: String,
    /// Lets `verify` recompute the fee without the chain.
    pub value_sat: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedOutput {
    pub address: String,
    pub value_sat: u64,
    /// `true` if the output returns to the signing wallet.
    pub is_change: bool,
}

/// Signs and finalizes the PSBTs built by [`crate::plan::build`] at
/// `fee_rate_sat_vb`.
pub fn sign(wallet: &Wallet, built: Vec<BuiltTx>, fee_rate_sat_vb: f64) -> Result<SignedBatch> {
    let mut txs = Vec::with_capacity(built.len());

    for mut item in built {
        // `trust_witness_utxo` stays off: trusting an unproven witness_utxo amount
        // enables the fee-draining attack. The source transactions are in the wallet.
        let sign_options = SignOptions::default();

        wallet
            .sign(&mut item.psbt, sign_options.clone())
            .map_err(|e| signing_failed(item.index, e))?;

        let finalized = wallet
            .finalize_psbt(&mut item.psbt, sign_options)
            .map_err(|e| signing_failed(item.index, e))?;

        if !finalized {
            let missing = item
                .psbt
                .inputs
                .iter()
                .filter(|i| i.final_script_witness.is_none() && i.final_script_sig.is_none())
                .count();
            return Err(Error::NotFinalized {
                index: item.index,
                missing,
            });
        }

        let tx = item
            .psbt
            .clone()
            .extract_tx()
            .map_err(|e| signing_failed(item.index, e))?;

        // Signing must not change the txid (it would if inputs were non-SegWit).
        let final_txid = tx.compute_txid();
        if final_txid != item.txid {
            return Err(Error::Signing(tf!(
                {
                    "transazione #{}: il txid è cambiato dopo la firma ({} → {}). \
                     Il batch contiene input non SegWit, non supportati.",
                    number(item.index),
                    item.txid,
                    final_txid
                },
                {
                    "transaction #{}: the txid changed after signing ({} → {}). \
                     The batch contains non-SegWit inputs, which are not supported.",
                    number(item.index),
                    item.txid,
                    final_txid
                }
            )));
        }

        let recipient_spk = item.recipient.script_pubkey();
        let outputs = tx
            .output
            .iter()
            .map(|o| SignedOutput {
                address: shown_address(&o.script_pubkey, wallet.network()),
                value_sat: o.value.to_sat(),
                is_change: o.script_pubkey != recipient_spk,
            })
            .collect();

        let vsize = tx.vsize() as u64;
        txs.push(SignedTx {
            index: item.index,
            label: item.label,
            txid: final_txid.to_string(),
            hex: serialize_hex(&tx),
            psbt: item.psbt.to_string(),
            inputs: item
                .inputs
                .iter()
                .map(|(op, v)| SignedInput {
                    outpoint: op.to_string(),
                    value_sat: v.to_sat(),
                })
                .collect(),
            outputs,
            fee_sat: item.fee.to_sat(),
            fee_rate_sat_vb: item.fee.to_sat() as f64 / vsize as f64,
            vsize,
        });
    }

    Ok(SignedBatch {
        version: BATCH_SCHEMA_VERSION,
        network: wallet.network().to_string(),
        created_by: concat!("plebline ", env!("CARGO_PKG_VERSION")).to_string(),
        descriptor_checksum: wallet.descriptor_checksum(bdk_wallet::KeychainKind::External),
        fee_rate_sat_vb,
        txs,
    })
}

/// An output's destination as `batch.json` writes it: the address, or the
/// raw script when it has none. `verify` recomputes it from the bytes, so
/// the text cannot say one thing while the transaction pays another.
pub fn shown_address(script: &Script, network: Network) -> String {
    Address::from_script(script, network)
        .map(|a| a.to_string())
        .unwrap_or_else(|_| format!("script:{}", script.to_hex_string()))
}

fn signing_failed(index: usize, e: impl std::fmt::Display) -> Error {
    let index = number(index);
    Error::Signing(tf!({ "transazione #{index}: {e}" }, {
        "transaction #{index}: {e}"
    }))
}

// Saturating sums: these totals are only for display, and a file may hold
// made-up values.
impl SignedBatch {
    pub fn total_amount(&self) -> Amount {
        Amount::from_sat(
            self.txs
                .iter()
                .flat_map(|t| t.outputs.iter())
                .filter(|o| !o.is_change)
                .fold(0, |sum, o| sum.saturating_add(o.value_sat)),
        )
    }

    pub fn total_fee(&self) -> Amount {
        Amount::from_sat(
            self.txs
                .iter()
                .fold(0, |sum, t| sum.saturating_add(t.fee_sat)),
        )
    }
}

/// The PSBT to hand to another wallet (base64), stripped of what broadcast
/// does not need:
///
/// - `non_witness_utxo`, which makes it too large for a QR; `witness_utxo`
///   stays so the receiver can show the fee;
/// - derivation paths and fingerprints, which reveal the change output.
///
/// Refuses if the inner transaction differs by a byte from the hex.
pub fn shareable_psbt(tx: &SignedTx) -> Result<String> {
    use bdk_wallet::bitcoin::Psbt;
    use std::str::FromStr;

    let index = number(tx.index);
    let refuse = |reason: String| {
        Error::Verification(tf!(
            { "transazione #{index}: non riesco a preparare il PSBT ({reason})" },
            { "transaction #{index}: I cannot prepare the PSBT ({reason})" }
        ))
    };

    let mut psbt = Psbt::from_str(&tx.psbt).map_err(|e| refuse(e.to_string()))?;

    psbt.xpub.clear();
    psbt.proprietary.clear();
    psbt.unknown.clear();

    let previous: Vec<_> = psbt
        .unsigned_tx
        .input
        .iter()
        .map(|i| i.previous_output)
        .collect();
    for (input, outpoint) in psbt.inputs.iter_mut().zip(previous) {
        if input.final_script_witness.is_none() && input.final_script_sig.is_none() {
            return Err(refuse(tf!({ "l'input {outpoint} non è firmato" }, {
                "input {outpoint} is not signed"
            })));
        }
        if input.witness_utxo.is_none() {
            // Keep the spent output before dropping the source transaction.
            let spent = input
                .non_witness_utxo
                .as_ref()
                .and_then(|source| source.output.get(outpoint.vout as usize))
                .cloned()
                .ok_or_else(|| {
                    refuse(tf!({ "manca l'importo dell'input {outpoint}" }, {
                        "the amount of input {outpoint} is missing"
                    }))
                })?;
            input.witness_utxo = Some(spent);
        }
        input.non_witness_utxo = None;
        input.partial_sigs.clear();
        input.bip32_derivation.clear();
        input.tap_key_origins.clear();
        input.proprietary.clear();
        input.unknown.clear();
    }

    for output in psbt.outputs.iter_mut() {
        output.redeem_script = None;
        output.witness_script = None;
        output.bip32_derivation.clear();
        output.tap_internal_key = None;
        output.tap_tree = None;
        output.tap_key_origins.clear();
        output.proprietary.clear();
        output.unknown.clear();
    }

    // Fee rate already checked by `verify`; only the bytes matter here.
    let extracted = psbt.clone().extract_tx_unchecked_fee_rate();
    if serialize_hex(&extracted) != tx.hex {
        return Err(refuse(
            t(
                "la transazione dentro il PSBT non coincide con l'hex firmato",
                "the transaction inside the PSBT does not match the signed hex",
            )
            .into(),
        ));
    }

    Ok(psbt.to_string())
}
