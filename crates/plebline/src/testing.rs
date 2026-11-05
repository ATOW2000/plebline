//! Helpers for tests, in this crate and in the ones built on it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use plebline_core::lang::{self, Lang};

use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::hashes::Hash;
use plebline_core::bitcoin::{
    absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness,
};

/// A transaction spending a made-up coin (`nonce` in the first byte of its
/// txid) and paying each `(script, sats)`: where test coins come from.
pub fn paying(nonce: u8, outputs: &[(ScriptBuf, u64)]) -> Transaction {
    let mut previous = [0u8; 32];
    previous[0] = nonce;
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from_byte_array(previous), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: outputs
            .iter()
            .map(|(script, sats)| TxOut {
                value: Amount::from_sat(*sats),
                script_pubkey: script.clone(),
            })
            .collect(),
    }
}

/// The hex of a transaction spending `outpoint`, as a signed file holds it:
/// the coins a file commits are read from these bytes.
pub fn spending_hex(outpoint: &str) -> String {
    let tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::from_str(outpoint).expect("outpoint di prova valido"),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new(),
        }],
    };
    serialize_hex(&tx)
}

/// A directory under the system temp dir, removed on drop. Written by hand
/// to avoid a `tempfile` dependency.
pub struct Temp {
    pub path: PathBuf,
}

impl Temp {
    /// `name` only helps when reading a leftover directory; the counter
    /// keeps every call apart.
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "plebline-test-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("cartella creabile");
        // A home of this test's own: nothing reads the real `~/PlebLine`.
        set_home(path.join("casa"));
        Self { path }
    }

    /// Writes `content` at `relative`, creating the folders on the way.
    pub fn write(&self, relative: &str, content: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("cartella creabile");
        }
        std::fs::write(&path, content).expect("file scrivibile");
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn in_italian() {
    lang::set_for_this_thread(Lang::It);
}

thread_local! {
    static HOME: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Tests give each thread a home of its own, so none of them reads the real
/// `~/PlebLine`, with its signed files, of whoever runs them.
fn set_home(home: PathBuf) {
    HOME.with(|current| *current.borrow_mut() = Some(home));
}

pub(crate) fn home() -> Option<PathBuf> {
    let set = HOME.with(|current| current.borrow().clone());
    // In this crate's own tests nothing falls back on the real home.
    #[cfg(test)]
    let set = set.or_else(|| Some(std::env::temp_dir().join("plebline-prove-senza-casa")));
    set
}

#[cfg(test)]
pub(crate) use own::*;

#[cfg(test)]
mod own {
    use plebline_core::batch::{
        SignedBatch, SignedInput, SignedOutput, SignedTx, BATCH_SCHEMA_VERSION,
    };
    use plebline_core::bitcoin::OutPoint;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::str::FromStr;

    /// Coin `i` of the test batches: txid `i` padded to 64 digits, output 0.
    pub(crate) fn outpoint(i: usize) -> OutPoint {
        OutPoint::from_str(&format!("{i:064}:0")).expect("outpoint valido")
    }

    pub(crate) fn set_of(indexes: &[usize]) -> BTreeSet<OutPoint> {
        indexes.iter().map(|i| outpoint(*i)).collect()
    }

    /// Source without comment lines, so a word in prose does not count.
    fn code_only(source: &str) -> String {
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Source files of the window's crates (this one, `plebline-guided`,
    /// `plebline-gui`) whose code names any of `words`, skipping the file
    /// called `allowed`.
    pub(crate) fn files_naming(words: &[&str], allowed: &str) -> Vec<String> {
        let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut pending: Vec<PathBuf> = ["plebline", "plebline-guided", "plebline-gui"]
            .iter()
            .map(|name| crates.join(name).join("src"))
            .collect();

        let mut found = Vec::new();
        while let Some(dir) = pending.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("{} non leggibile: {e}", dir.display()));
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                if path.file_name().and_then(|n| n.to_str()) == Some(allowed) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("leggibile");
                if words.iter().any(|word| code_only(&source).contains(word)) {
                    found.push(path.display().to_string());
                }
            }
        }
        found.sort();
        found
    }

    /// The code of `src/<file>` in this crate, comment lines left out.
    pub(crate) fn code_of(file: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(file);
        code_only(&std::fs::read_to_string(&path).expect("leggibile"))
    }

    /// Payment `i` spends [`outpoint`]`(i)`.
    pub(crate) fn batch_of(indexes: &[usize]) -> SignedBatch {
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
                    hex: super::spending_hex(&format!("{i:064}:0")),
                    psbt: "cHNidP8=".into(),
                    inputs: vec![SignedInput {
                        outpoint: format!("{i:064}:0"),
                        value_sat: 100_000,
                    }],
                    outputs: vec![SignedOutput {
                        address: "tb1qqqqq".into(),
                        value_sat: 99_000,
                        is_change: false,
                    }],
                    fee_sat: 1_000,
                    fee_rate_sat_vb: 1.0,
                    vsize: 141,
                })
                .collect(),
        }
    }
}
