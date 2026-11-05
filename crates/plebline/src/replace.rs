//! Withdrawing a signed, never-sent payment to reuse its coin.
//!
//! Commitments are the batch files themselves, so freeing a coin means
//! removing that one payment from its batch; the others stay valid. A copy
//! that already left the machine (stick, QR) is still valid, and whichever
//! transaction is broadcast first spends the coin.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use plebline_core::bitcoin::OutPoint;
use plebline_core::plan::BatchConfig;
use plebline_core::verify as verifier;
use plebline_core::{tf, Error, Result};

use crate::project;
use crate::wizard;

/// A signed payment whose coins are all still on hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The batch file holding it.
    pub file: PathBuf,
    pub txid: String,
    pub index: usize,
    pub label: Option<String>,
    /// Leaving the wallet, change excluded.
    pub leaving_sat: u64,
    /// Outputs that do not come back.
    pub recipients: Vec<String>,
    /// Freed by withdrawing it.
    pub coins: Vec<OutPoint>,
}

/// This wallet's payments that can still be withdrawn: from `batch.json`
/// and its archives (not the split), only if every coin is in `on_hand`.
/// Other wallets' batches are skipped by descriptor checksum.
pub fn pending(
    sources: &[PathBuf],
    wallet: &str,
    on_hand: &BTreeSet<OutPoint>,
) -> Result<Vec<Pending>> {
    let mut sources: Vec<PathBuf> = sources.to_vec();
    sources.sort();
    sources.dedup();

    let mut found = Vec::new();
    for path in &sources {
        if !is_group(path) || !path.is_file() {
            continue;
        }
        let batch = verifier::load(path)?;
        if batch.descriptor_checksum != wallet {
            continue;
        }
        for tx in &batch.txs {
            let coins = wizard::coins_spent(path, tx)?;
            if !coins.iter().all(|coin| on_hand.contains(coin)) {
                continue;
            }
            found.push(Pending {
                file: path.clone(),
                txid: tx.txid.clone(),
                index: tx.index,
                label: tx.label.clone(),
                leaving_sat: tx
                    .outputs
                    .iter()
                    .filter(|o| !o.is_change)
                    .map(|o| o.value_sat)
                    .sum(),
                recipients: tx
                    .outputs
                    .iter()
                    .filter(|o| !o.is_change)
                    .map(|o| o.address.clone())
                    .collect(),
                coins,
            });
        }
    }
    Ok(found)
}

/// `batch.json` or `batch-N.json`.
fn is_group(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .is_some_and(|name| name == project::BATCH || wizard::is_archive_of(project::BATCH, &name))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Withdrawn {
    /// `tx-K.hex` copies deleted.
    pub copies_removed: usize,
    /// The batch held only that payment and was removed.
    pub batch_removed: bool,
}

/// Removes payment `txid` from batch `file`, with its `tx-K.hex` copies.
///
/// Copies go first, then the batch: an interruption leaves the coin still
/// committed, never free with a signed payment on disk. The matching line
/// of `batch-config.json` goes too (same position, same recipient) or
/// nothing is touched.
pub fn withdraw(file: &Path, txid: &str) -> Result<Withdrawn> {
    let mut batch = verifier::load(file)?;
    let position = batch
        .txs
        .iter()
        .position(|tx| tx.txid == txid)
        .ok_or_else(|| {
            let file = file.display();
            Error::Refused(tf!(
                { "la ricarica {txid} non è in {file}: non tolgo niente" },
                { "refill {txid} is not in {file}: I remove nothing" }
            ))
        })?;
    let dir = file.parent().unwrap_or(Path::new("."));

    // Checked before touching anything.
    let config_path = dir.join(project::CONFIG);
    let is_current = file.file_name().is_some_and(|n| n == project::BATCH);
    let config = if is_current && config_path.is_file() {
        let mut config: BatchConfig = plebline_core::read_json(&config_path)?;
        let lines_up = config.entries.len() == batch.txs.len()
            && batch.txs[position]
                .outputs
                .iter()
                .any(|o| !o.is_change && o.address == config.entries[position].recipient);
        if !lines_up {
            let path = config_path.display();
            return Err(Error::Refused(tf!(
                {
                    "{path} non corrisponde al gruppo firmato: non so quale riga togliere, \
                     e non tolgo niente"
                },
                {
                    "{path} does not match the signed group: I do not know which line to \
                     remove, and I remove nothing"
                }
            )));
        }
        config.entries.remove(position);
        Some(config)
    } else {
        None
    };

    let removed = batch.txs.remove(position);
    let copies_removed = remove_copies(dir, &removed.hex)?;

    let batch_removed = batch.txs.is_empty();
    if batch_removed {
        remove_file(file)?;
    } else {
        // Written beside the file and renamed into place: never half a batch.
        wizard::write_json(file, &batch)?;
    }

    if let Some(config) = config {
        if config.entries.is_empty() {
            remove_file(&config_path)?;
        } else {
            wizard::write_json(&config_path, &config)?;
        }
    }

    Ok(Withdrawn {
        copies_removed,
        batch_removed,
    })
}

/// Deletes `tx-K.hex` files holding exactly `hex`: by content, since
/// archived groups reuse the numbers.
fn remove_copies(dir: &Path, hex: &str) -> Result<usize> {
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(wizard::is_per_tx_name(&name) || wizard::is_per_tx_archive(&name)) {
            continue;
        }
        let path = entry.path();
        let same = std::fs::read_to_string(&path).is_ok_and(|content| content.trim() == hex);
        if same {
            remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn remove_file(path: &Path) -> Result<()> {
    std::fs::remove_file(path).map_err(Error::io(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{outpoint, set_of, Temp};
    use plebline_core::batch::{
        SignedBatch, SignedInput, SignedOutput, SignedTx, BATCH_SCHEMA_VERSION,
    };
    use plebline_core::plan::BatchEntry;

    fn recipient(i: usize) -> String {
        format!("tb1qdestinatario{i}")
    }

    /// Payment `i` spends coin `i` and pays `recipient(i)`.
    fn batch_of(indexes: &[usize]) -> SignedBatch {
        SignedBatch {
            version: BATCH_SCHEMA_VERSION,
            network: "signet".into(),
            created_by: "test".into(),
            descriptor_checksum: "aaaaaaaa".into(),
            fee_rate_sat_vb: 1.0,
            txs: indexes
                .iter()
                .map(|i| SignedTx {
                    index: *i,
                    label: None,
                    txid: format!("{i:064}"),
                    hex: crate::testing::spending_hex(&format!("{i:064}:0")),
                    psbt: "cHNidP8=".into(),
                    inputs: vec![SignedInput {
                        outpoint: format!("{i:064}:0"),
                        value_sat: 100_000,
                    }],
                    outputs: vec![
                        SignedOutput {
                            address: recipient(*i),
                            value_sat: 60_000,
                            is_change: false,
                        },
                        SignedOutput {
                            address: "tb1qresto".into(),
                            value_sat: 39_000,
                            is_change: true,
                        },
                    ],
                    fee_sat: 1_000,
                    fee_rate_sat_vb: 1.0,
                    vsize: 110,
                })
                .collect(),
        }
    }

    fn config_of(indexes: &[usize]) -> BatchConfig {
        BatchConfig {
            network: "signet".into(),
            fee_rate_sat_vb: 1.0,
            entries: indexes
                .iter()
                .map(|i| BatchEntry {
                    label: None,
                    recipient: recipient(*i),
                    amount_sat: 60_000,
                })
                .collect(),
        }
    }

    fn txid(i: usize) -> String {
        format!("{i:064}")
    }

    #[test]
    fn a_payment_whose_coin_is_on_hand_can_be_given_up() {
        let dir = Temp::new("in-attesa");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1, 2])).expect("scritto");

        let found =
            pending(std::slice::from_ref(&file), "aaaaaaaa", &set_of(&[1, 2])).expect("letti");

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].txid, txid(1));
        assert_eq!(found[0].leaving_sat, 60_000, "il resto non è un pagamento");
        assert_eq!(found[0].recipients, vec![recipient(1)]);
        assert_eq!(found[0].coins, vec![outpoint(1)]);
        assert_eq!(found[0].file, file);
    }

    /// A coin no longer listed: already spent, nothing to withdraw.
    #[test]
    fn a_payment_whose_coin_is_gone_is_not_offered() {
        let dir = Temp::new("atterrato");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1, 2])).expect("scritto");

        let found = pending(&[file], "aaaaaaaa", &set_of(&[2])).expect("letti");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].txid, txid(2));
    }

    #[test]
    fn the_split_and_another_wallets_groups_are_not_offered() {
        let dir = Temp::new("esclusi");
        let split = dir.path.join(project::SPLIT);
        wizard::write_json(&split, &batch_of(&[1])).expect("scritto");
        let theirs = dir.path.join("batch-1.json");
        let mut foreign = batch_of(&[2]);
        foreign.descriptor_checksum = "bbbbbbbb".into();
        wizard::write_json(&theirs, &foreign).expect("scritto");
        let archived = dir.path.join("batch-2.json");
        wizard::write_json(&archived, &batch_of(&[3])).expect("scritto");

        let found = pending(
            &[split, theirs, archived.clone()],
            "aaaaaaaa",
            &set_of(&[1, 2, 3]),
        )
        .expect("letti");

        assert_eq!(found.len(), 1, "solo l'archivio di questo wallet");
        assert_eq!(found[0].file, archived);
    }

    /// One payment goes; the rest still matches the definition.
    #[test]
    fn giving_up_one_payment_keeps_the_others_and_their_definition() {
        let dir = Temp::new("togli-uno");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1, 2, 3])).expect("scritto");
        wizard::write_json(&dir.path.join(project::CONFIG), &config_of(&[1, 2, 3]))
            .expect("scritto");

        let done = withdraw(&file, &txid(2)).expect("tolto");

        assert!(!done.batch_removed);
        let left = verifier::load(&file).expect("il gruppo si rilegge");
        let txids: Vec<_> = left.txs.iter().map(|tx| tx.txid.clone()).collect();
        assert_eq!(txids, vec![txid(1), txid(3)]);
        let config: BatchConfig =
            plebline_core::read_json(&dir.path.join(project::CONFIG)).expect("config");
        let recipients: Vec<_> = config.entries.iter().map(|e| e.recipient.clone()).collect();
        assert_eq!(recipients, vec![recipient(1), recipient(3)]);

        let committed = pending(&[file], "aaaaaaaa", &set_of(&[1, 2, 3])).expect("letti");
        assert!(
            committed.iter().all(|p| p.coins != vec![outpoint(2)]),
            "la moneta 2 adesso è libera"
        );
    }

    /// Copies are matched by content, not number.
    #[test]
    fn only_the_copies_of_that_payment_are_deleted() {
        let dir = Temp::new("copie");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1, 2])).expect("scritto");
        for i in [1, 2] {
            let hex = crate::testing::spending_hex(&format!("{i:064}:0"));
            std::fs::write(dir.path.join(format!("tx-{i}.hex")), format!("{hex}\n"))
                .expect("scritto");
        }
        std::fs::write(dir.path.join("tx-1-1.hex"), "0200999\n").expect("scritto");

        let done = withdraw(&file, &txid(1)).expect("tolto");

        assert_eq!(done.copies_removed, 1);
        assert!(!dir.path.join("tx-1.hex").exists());
        assert!(dir.path.join("tx-2.hex").exists());
        assert!(dir.path.join("tx-1-1.hex").exists());
    }

    #[test]
    fn a_group_left_empty_is_removed_with_its_definition() {
        let dir = Temp::new("vuoto");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1])).expect("scritto");
        wizard::write_json(&dir.path.join(project::CONFIG), &config_of(&[1])).expect("scritto");

        let done = withdraw(&file, &txid(1)).expect("tolto");

        assert!(done.batch_removed);
        assert!(
            !file.exists(),
            "un gruppo vuoto non impegna niente e non serve"
        );
        assert!(!dir.path.join(project::CONFIG).exists());
    }

    /// Definition out of step: nothing touched.
    #[test]
    fn a_definition_that_does_not_line_up_stops_everything() {
        let dir = Temp::new("config-storta");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1, 2])).expect("scritto");
        wizard::write_json(&dir.path.join(project::CONFIG), &config_of(&[2, 1])).expect("scritto");
        std::fs::write(dir.path.join("tx-1.hex"), "02001").expect("scritto");

        let error = withdraw(&file, &txid(1)).expect_err("non corrisponde");

        assert!(matches!(error, Error::Refused(_)), "{error:?}");
        assert_eq!(verifier::load(&file).expect("intatto").txs.len(), 2);
        assert!(dir.path.join("tx-1.hex").exists(), "niente è stato toccato");
    }

    #[test]
    fn an_unknown_payment_removes_nothing() {
        let dir = Temp::new("sconosciuto");
        let file = dir.path.join(project::BATCH);
        wizard::write_json(&file, &batch_of(&[1])).expect("scritto");

        assert!(withdraw(&file, &txid(9)).is_err());
        assert_eq!(verifier::load(&file).expect("intatto").txs.len(), 1);
    }

    #[test]
    fn an_archive_leaves_the_current_definition_alone() {
        let dir = Temp::new("archivio");
        let archived = dir.path.join("batch-1.json");
        wizard::write_json(&archived, &batch_of(&[1, 2])).expect("scritto");
        wizard::write_json(&dir.path.join(project::CONFIG), &config_of(&[5, 6])).expect("scritto");

        withdraw(&archived, &txid(1)).expect("tolto");

        let config: BatchConfig =
            plebline_core::read_json(&dir.path.join(project::CONFIG)).expect("config");
        assert_eq!(config.entries.len(), 2);
        assert_eq!(verifier::load(&archived).expect("letto").txs.len(), 1);
    }
}
