//! Offline logic of `plebline`: no network client, checked in CI.
//!
//! Central invariant: no two presigned payments share an input, within a
//! batch and within a session. The selector in [`coin_select`] never sees
//! committed coins, and [`plan::build`] checks the result again in several
//! independent ways.

#![forbid(unsafe_code)]

pub mod address;
pub mod batch;
pub mod coin_select;
pub mod error;
pub mod lang;
pub mod plan;
pub mod signatures;
pub mod split;
pub mod utxos;
pub mod verify;
pub mod wallet;
pub mod watch;

pub use error::{DisjointnessError, Error, Result};

pub use bdk_wallet;
pub use bdk_wallet::bitcoin;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The sum, if it stays within the 21 million bitcoin that can ever exist.
/// Anything above comes from a made-up file, and would overflow later sums.
pub(crate) fn within_all_bitcoin(values: impl IntoIterator<Item = u64>) -> Option<u64> {
    let max = bitcoin::Amount::MAX_MONEY.to_sat();
    values.into_iter().try_fold(0u64, |sum, value| {
        sum.checked_add(value).filter(|s| *s <= max)
    })
}

/// Reads and parses a JSON file; both errors name the file.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let raw = std::fs::read_to_string(path).map_err(Error::io(path))?;
    serde_json::from_str(&raw).map_err(|source| Error::Json {
        path: path.display().to_string(),
        source,
    })
}

/// Made-up transaction ids for the unit tests: `n` in the first byte.
#[cfg(test)]
pub(crate) mod testing {
    use bdk_wallet::bitcoin::hashes::Hash;
    use bdk_wallet::bitcoin::{OutPoint, Txid};

    pub fn txid(n: u8) -> Txid {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        Txid::from_byte_array(bytes)
    }

    pub fn outpoint(n: u8) -> OutPoint {
        OutPoint::new(txid(n), 0)
    }
}
