//! `wallet.json`: offline → online handover. Public descriptors only: enough
//! to watch, not to spend, but they reveal every past and future address.

use std::path::Path;
use std::str::FromStr;

use bdk_wallet::bitcoin::Network;
use bdk_wallet::KeychainKind;

use crate::error::{Error, Result};
use crate::lang::t;
use crate::tf;

pub const WATCH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WatchFile {
    pub version: u32,
    pub network: String,
    /// Receive descriptor, with checksum.
    pub external: String,
    /// Change descriptor, with checksum.
    pub internal: String,
    /// The fingerprint shown when comparing a wallet with a UTXO file.
    pub descriptor_checksum: String,
    pub created_by: String,
}

impl WatchFile {
    pub fn parsed_network(&self) -> Result<Network> {
        Network::from_str(&self.network).map_err(|_| Error::NetworkMismatch {
            found: self.network.clone(),
            expected: t("una rete valida", "a valid network").to_string(),
        })
    }
}

pub fn load(path: &Path, expected_network: Network) -> Result<WatchFile> {
    let file: WatchFile = crate::read_json(path)?;

    if file.version != WATCH_SCHEMA_VERSION {
        return Err(Error::SchemaVersion {
            path: path.display().to_string(),
            found: file.version,
            expected: WATCH_SCHEMA_VERSION,
        });
    }

    let network = file.parsed_network()?;
    if network != expected_network {
        return Err(Error::NetworkMismatch {
            found: network.to_string(),
            expected: expected_network.to_string(),
        });
    }

    // The declared checksum names the wallet, on screen and when the words
    // are checked, so it must be the one these descriptors produce.
    let wallet = crate::wallet::from_descriptors(&file.external, &file.internal, network)?;
    let actual = wallet.descriptor_checksum(KeychainKind::External);
    if actual != file.descriptor_checksum {
        let path = path.display();
        let declared = &file.descriptor_checksum;
        return Err(Error::Descriptor(tf!(
            {
                "{path} dichiara il wallet {declared}, ma i suoi descriptor sono del \
                 wallet {actual}"
            },
            {
                "{path} declares wallet {declared}, but its descriptors belong to \
                 wallet {actual}"
            }
        )));
    }
    if !crate::wallet::one_account(&wallet) {
        return Err(Error::Descriptor(format!(
            "{}: {}",
            path.display(),
            not_one_account()
        )));
    }

    Ok(file)
}

/// Public so a program that writes `wallet.json` can refuse with the same
/// words before writing it.
pub fn not_one_account() -> &'static str {
    t(
        "il descriptor del resto non è dello stesso conto di quello di ricezione: il resto \
         potrebbe andare a un altro wallet e sembrare comunque tuo",
        "the change descriptor is not from the same account as the receiving one: change \
         could go to another wallet and still look like yours",
    )
}
