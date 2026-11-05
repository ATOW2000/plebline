//! Online → offline handover, untrusted: every field is re-checked against
//! the raw source transaction before any key is touched.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use bdk_wallet::bitcoin::consensus::encode::deserialize_hex;
use bdk_wallet::bitcoin::{Amount, Network, OutPoint, ScriptBuf, Transaction, Txid};
use bdk_wallet::{KeychainKind, Wallet};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::lang::t;
use crate::tf;

pub const UTXOS_SCHEMA_VERSION: u32 = 1;

/// The highest derivation index a file may name. Opening the wallet derives
/// every address up to it, one by one (about 75 µs each in the release
/// build: 7.5 s for 100 000). A personal cold storage stays within a few
/// thousand (gap limit 20, BDK lookahead 25); below 2^31 a hostile file
/// could otherwise ask for two billion and stall the program for days.
pub const MAX_DERIVATION_INDEX: u32 = 100_000;

/// `utxos.json`, written by the scan of the chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoSet {
    pub version: u32,
    pub network: String,
    /// Used as `last_seen`: offline, the clock is never read.
    pub generated_at: u64,
    pub tip_height: u32,
    /// Must match the offline wallet's, or the seed is not this cold storage's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_checksum: Option<String>,
    /// Per keychain, the index up to which spks are revealed.
    #[serde(default)]
    pub last_active_indices: BTreeMap<String, u32>,
    pub utxos: Vec<UtxoEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoEntry {
    /// `<txid>:<vout>`
    pub outpoint: String,
    pub value_sat: u64,
    pub script_pubkey: String,
    /// `external` | `internal`
    pub keychain: String,
    pub derivation_index: u32,
    pub confirmations: u32,
    /// Source transaction (hex): rebuilds state offline and proves `value_sat`
    /// and `script_pubkey`.
    pub prev_tx_hex: String,
}

/// Fields derived from the raw transaction, not copied from the JSON.
#[derive(Debug, Clone)]
pub struct ValidatedUtxo {
    pub outpoint: OutPoint,
    pub value: Amount,
    pub script_pubkey: ScriptBuf,
    pub keychain: KeychainKind,
    pub derivation_index: u32,
    pub confirmations: u32,
}

/// The validated set, ready to hydrate an offline wallet.
#[derive(Debug, Clone)]
pub struct ValidatedUtxoSet {
    pub network: Network,
    pub generated_at: u64,
    pub tip_height: u32,
    pub descriptor_checksum: Option<String>,
    pub last_active_indices: BTreeMap<KeychainKind, u32>,
    pub utxos: Vec<ValidatedUtxo>,
    prev_txs: Vec<Transaction>,
}

impl ValidatedUtxoSet {
    pub fn total(&self) -> Amount {
        self.utxos.iter().map(|u| u.value).sum()
    }

    /// Inserts the source transactions and reveals spks so `list_unspent` sees
    /// the UTXOs. They go in unconfirmed (no headers offline); chain position
    /// does not matter for signing.
    ///
    /// A source transaction comes in whole, so the wallet also sees its other
    /// outputs to us. The file does not vouch for those (they may be spent
    /// already): they are locked, and nothing built from this wallet selects
    /// them.
    pub fn hydrate(&self, wallet: &mut Wallet) -> Result<()> {
        for (keychain, index) in &self.last_active_indices {
            let _ = wallet.reveal_addresses_to(*keychain, *index).count();
        }
        // Reveal the UTXOs' own indexes even without `last_active_indices`.
        for utxo in &self.utxos {
            let _ = wallet
                .reveal_addresses_to(utxo.keychain, utxo.derivation_index)
                .count();
        }

        wallet.apply_unconfirmed_txs(
            self.prev_txs
                .iter()
                .cloned()
                .map(|tx| (tx, self.generated_at)),
        );

        // An unrecognized UTXO means the wrong descriptor: say so now.
        let known: std::collections::BTreeSet<OutPoint> =
            wallet.list_unspent().map(|u| u.outpoint).collect();
        for utxo in &self.utxos {
            if !known.contains(&utxo.outpoint) {
                return Err(Error::InvalidUtxo {
                    outpoint: utxo.outpoint.to_string(),
                    reason: t(
                        "il wallet non lo riconosce come proprio: il seed o il descriptor \
                         non corrispondono a quelli usati per l'export",
                        "the wallet does not recognize it as its own: the seed or the \
                         descriptor do not match the ones used for the export",
                    )
                    .into(),
                });
            }
        }

        let listed: std::collections::BTreeSet<OutPoint> =
            self.utxos.iter().map(|u| u.outpoint).collect();
        for outpoint in known.difference(&listed) {
            wallet.lock_outpoint(*outpoint);
        }

        Ok(())
    }
}

/// Run on the built transaction: refuses it if it spends a coin that
/// [`ValidatedUtxoSet::hydrate`] locked because the file does not list it,
/// whatever selected that coin.
pub(crate) fn refuse_unlisted(
    wallet: &Wallet,
    inputs: impl IntoIterator<Item = OutPoint>,
    index: usize,
) -> Result<()> {
    for outpoint in inputs {
        if wallet.is_outpoint_locked(outpoint) {
            return Err(Error::BuildTx {
                index,
                reason: tf!(
                    { "spenderebbe {outpoint}, che l'elenco delle monete non contiene" },
                    { "it would spend {outpoint}, which the list of coins does not contain" }
                ),
            });
        }
    }
    Ok(())
}

pub fn load(path: &Path, expected_network: Network) -> Result<ValidatedUtxoSet> {
    let set: UtxoSet = crate::read_json(path)?;
    validate(set, expected_network, &path.display().to_string())
}

pub fn validate(set: UtxoSet, expected_network: Network, path: &str) -> Result<ValidatedUtxoSet> {
    if set.version != UTXOS_SCHEMA_VERSION {
        return Err(Error::SchemaVersion {
            path: path.to_string(),
            found: set.version,
            expected: UTXOS_SCHEMA_VERSION,
        });
    }

    let network = Network::from_str(&set.network).map_err(|_| Error::NetworkMismatch {
        found: set.network.clone(),
        expected: expected_network.to_string(),
    })?;
    if network != expected_network {
        return Err(Error::NetworkMismatch {
            found: network.to_string(),
            expected: expected_network.to_string(),
        });
    }

    let mut last_active_indices = BTreeMap::new();
    for (name, index) in &set.last_active_indices {
        last_active_indices.insert(
            parse_keychain(name, "last_active_indices")?,
            derivable(*index, "last_active_indices")?,
        );
    }

    let mut prev_txs: BTreeMap<Txid, Transaction> = BTreeMap::new();
    let mut utxos = Vec::with_capacity(set.utxos.len());
    let mut seen: std::collections::BTreeSet<OutPoint> = Default::default();

    for entry in &set.utxos {
        let outpoint = OutPoint::from_str(&entry.outpoint).map_err(|e| Error::InvalidUtxo {
            outpoint: entry.outpoint.clone(),
            reason: tf!({ "outpoint malformato ({e})" }, {
                "malformed outpoint ({e})"
            }),
        })?;

        if !seen.insert(outpoint) {
            return Err(Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: t("compare due volte nel file", "appears twice in the file").into(),
            });
        }

        let tx: Transaction =
            deserialize_hex(&entry.prev_tx_hex).map_err(|e| Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: tf!({ "prev_tx_hex non decodificabile ({e})" }, {
                    "prev_tx_hex cannot be decoded ({e})"
                }),
            })?;

        // Recompute the txid, do not trust the field.
        let computed = tx.compute_txid();
        if computed != outpoint.txid {
            return Err(Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: tf!(
                    { "prev_tx_hex ha txid {computed}, non corrisponde all'outpoint" },
                    { "prev_tx_hex has txid {computed}, it does not match the outpoint" }
                ),
            });
        }

        // A consensus rule: no transaction creates more than all bitcoin.
        // Past it, amount sums in the wallet would overflow.
        if crate::within_all_bitcoin(tx.output.iter().map(|o| o.value.to_sat())).is_none() {
            return Err(Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: t(
                    "la transazione che l'ha creata dichiara più bitcoin di quanti ne \
                     esisteranno mai: non è una transazione vera",
                    "the transaction that created it declares more bitcoin than will ever \
                     exist: it is not a real transaction",
                )
                .into(),
            });
        }

        // BDK cannot insert a coinbase as unconfirmed; it would need a real block
        // anchor. Not supported.
        if tx.is_coinbase() {
            return Err(Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: t(
                    "proviene da una transazione coinbase, non ancora supportata. \
                     Spendila una volta verso il cold storage e riesporta gli UTXO",
                    "it comes from a coinbase transaction, not supported yet. \
                     Spend it once to the cold storage and export the UTXOs again",
                )
                .into(),
            });
        }

        let txout = tx
            .output
            .get(outpoint.vout as usize)
            .ok_or_else(|| Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: tf!(
                    {
                        "vout {} fuori intervallo: la transazione ha {} output",
                        outpoint.vout,
                        tx.output.len()
                    },
                    {
                        "vout {} out of range: the transaction has {} outputs",
                        outpoint.vout,
                        tx.output.len()
                    }
                ),
            })?;

        if txout.value.to_sat() != entry.value_sat {
            return Err(Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: tf!(
                    {
                        "value_sat dichiarato {} ma la transazione dice {}",
                        entry.value_sat,
                        txout.value.to_sat()
                    },
                    {
                        "value_sat declared as {} but the transaction says {}",
                        entry.value_sat,
                        txout.value.to_sat()
                    }
                ),
            });
        }

        let declared_spk =
            ScriptBuf::from_hex(&entry.script_pubkey).map_err(|e| Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: tf!({ "script_pubkey non è hex valido ({e})" }, {
                    "script_pubkey is not valid hex ({e})"
                }),
            })?;
        if declared_spk != txout.script_pubkey {
            return Err(Error::InvalidUtxo {
                outpoint: entry.outpoint.clone(),
                reason: t(
                    "script_pubkey dichiarato diverso da quello della transazione",
                    "declared script_pubkey differs from the transaction's",
                )
                .into(),
            });
        }

        utxos.push(ValidatedUtxo {
            outpoint,
            value: txout.value,
            script_pubkey: txout.script_pubkey.clone(),
            keychain: parse_keychain(&entry.keychain, &entry.outpoint)?,
            derivation_index: derivable(entry.derivation_index, &entry.outpoint)?,
            confirmations: entry.confirmations,
        });
        prev_txs.insert(computed, tx);
    }

    // Distinct real coins cannot add up to more than all bitcoin.
    if crate::within_all_bitcoin(utxos.iter().map(|u| u.value.to_sat())).is_none() {
        return Err(Error::InvalidUtxo {
            outpoint: path.to_string(),
            reason: t(
                "insieme le monete valgono più di tutti i bitcoin che esisteranno mai",
                "together the coins are worth more than all the bitcoin there will ever be",
            )
            .into(),
        });
    }

    Ok(ValidatedUtxoSet {
        network,
        generated_at: set.generated_at,
        tip_height: set.tip_height,
        descriptor_checksum: set.descriptor_checksum,
        last_active_indices,
        utxos,
        prev_txs: prev_txs.into_values().collect(),
    })
}

/// See [`MAX_DERIVATION_INDEX`]. Also keeps the margin added later from
/// overflowing.
fn derivable(index: u32, context: &str) -> Result<u32> {
    if index > MAX_DERIVATION_INDEX {
        return Err(Error::InvalidUtxo {
            outpoint: context.to_string(),
            reason: tf!(
                {
                    "indice di derivazione {index} troppo alto: nessun cold storage personale \
                     usa più di {MAX_DERIVATION_INDEX} indirizzi, e derivarli tutti fermerebbe \
                     il programma"
                },
                {
                    "derivation index {index} is too high: no personal cold storage uses more \
                     than {MAX_DERIVATION_INDEX} addresses, and deriving them all would stall the \
                     program"
                }
            ),
        });
    }
    Ok(index)
}

fn parse_keychain(name: &str, context: &str) -> Result<KeychainKind> {
    match name {
        "external" | "receive" => Ok(KeychainKind::External),
        "internal" | "change" => Ok(KeychainKind::Internal),
        other => Err(Error::InvalidUtxo {
            outpoint: context.to_string(),
            reason: tf!(
                { "keychain sconosciuto '{other}': attesi 'external' o 'internal'" },
                { "unknown keychain '{other}': expected 'external' or 'internal'" }
            ),
        }),
    }
}

pub fn keychain_name(keychain: KeychainKind) -> &'static str {
    match keychain {
        KeychainKind::External => "external",
        KeychainKind::Internal => "internal",
    }
}
