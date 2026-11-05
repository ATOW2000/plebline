//! The ceremony's flows: numbered steps, a summary and an explicit
//! confirmation before anything irreversible.

use plebline_core::bdk_wallet::Wallet;
use plebline_core::bitcoin::Network;
use plebline_core::wallet::{self as wallet_builder, SeedWords};
use plebline_core::Result;

use crate::ask::Ask;

mod commitments;
mod config;
mod files;
mod print;
mod public;
mod sign;
mod split;
mod verify;

pub(crate) use commitments::coins_spent;
pub use commitments::{
    commitments_of_open_wallet, committed_outpoints, read_commitments, weigh_commitments,
    Commitments,
};
pub use config::{ask_batch_config, load_or_prompt_config, load_utxos};
pub use files::{
    is_archive_of, is_per_tx_archive, is_per_tx_name, is_temp_name, write_file, write_json,
    write_qr_files, QrContent, PER_TX_PREFIX,
};
pub use print::print_plan;
pub use public::{descriptors, DescriptorsInput};
pub use sign::{sign, SignInput};
pub use split::{split, SplitInput};
pub use verify::{verify, VerifyInput};

/// Where this run's wallet comes from. The caller opens it once and lends
/// it to every flow: each extra seed entry is another chance to leak it.
pub enum WalletAccess<'a> {
    Seed {
        seed: &'a SeedWords,
        passphrase: Option<&'a str>,
    },
    /// Opened by the caller only when the flow reaches its wallet step, so
    /// the files are checked before anything is asked.
    OnDemand(&'a dyn OpenWallet),
}

/// A wallet the flow asks for at its wallet step, not before.
pub trait OpenWallet {
    /// `need_keys`: the flow signs, so a watch-only wallet must refuse.
    fn open(&self, network: Network, need_keys: bool, ask: &mut dyn Ask) -> Result<Wallet>;
}

/// `need_keys`: only signing (refills or a split) needs the private key; the
/// others stay watch-only.
pub fn build_wallet(
    access: &WalletAccess<'_>,
    network: Network,
    need_keys: bool,
    ask: &mut dyn Ask,
) -> Result<Wallet> {
    match access {
        WalletAccess::Seed { seed, passphrase } => {
            wallet_builder::from_seed(seed, *passphrase, network)
        }
        WalletAccess::OnDemand(opener) => opener.open(network, need_keys, ask),
    }
}
