//! `wallet.json`: the public half of the wallet.

use std::path::PathBuf;

use plebline_core::bitcoin::Network;
use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::wallet as wallet_builder;
use plebline_core::Result;

use super::files::write_json;
use super::{build_wallet, WalletAccess};
use crate::ask::Ask;
use crate::ui::{self, Wizard};

pub struct DescriptorsInput<'a> {
    pub out: PathBuf,
    pub wallet: WalletAccess<'a>,
}

/// Writes `wallet.json`, the public half: enough to watch, not to spend.
pub fn descriptors(
    network: Network,
    input: DescriptorsInput<'_>,
    ask: &mut dyn Ask,
) -> Result<bool> {
    ui::title(t("Parte pubblica del wallet", "The wallet's public half"));
    ui::note(t(
        "Scrivo il codice di sola lettura del wallet: dice quali indirizzi guardare. \
         Contiene solo chiavi pubbliche: con questo file si può vedere il saldo, non spenderlo.",
        "I write the wallet's read-only code: it says which addresses to watch. It holds \
         public keys only: with it you can see the balance, not spend it.",
    ));

    let mut wizard = Wizard::new(2);

    wizard.step("Wallet");
    let wallet = build_wallet(&input.wallet, network, false, ask)?;
    let (external, internal) = wallet_builder::public_descriptors(&wallet);
    let checksum = wallet.descriptor_checksum(plebline_core::bdk_wallet::KeychainKind::External);
    ui::success(&tf!({ "Wallet {checksum}, rete {network}." }, {
        "Wallet {checksum}, network {network}."
    }));

    wizard.step(t("Scrittura del file", "Writing the file"));
    let file = plebline_core::watch::WatchFile {
        version: plebline_core::watch::WATCH_SCHEMA_VERSION,
        network: network.to_string(),
        external,
        internal,
        descriptor_checksum: checksum,
        created_by: format!("plebline {}", plebline_core::VERSION),
    };
    write_json(&input.out, &file)?;
    let out = input.out.display();
    ui::success(&tf!({ "Scritto {out}" }, { "Written to {out}" }));

    ui::blank();
    ui::warning(t(
        "Chi ottiene questo file vede tutti i tuoi indirizzi, presenti e futuri, e quindi \
         saldo e storia. Non è un seed, ma non è nemmeno da pubblicare.",
        "Whoever gets this file sees every address you have and will ever have, and therefore \
         your balance and history. It is not a seed, but it is not for publishing either.",
    ));

    Ok(true)
}
