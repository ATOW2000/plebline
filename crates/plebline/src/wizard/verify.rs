//! The independent check, and the same check run on a file before it is
//! written.

use std::path::PathBuf;

use plebline_core::batch::SignedBatch;
use plebline_core::bitcoin::Network;
use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::utxos::ValidatedUtxoSet;
use plebline_core::verify::{self as verifier, Expectations};
use plebline_core::wallet as wallet_builder;
use plebline_core::Result;

use super::config::{load_config, load_utxos};
use super::print::print_report;
use crate::ui::{self, Wizard};

pub struct VerifyInput {
    pub batch: PathBuf,
    pub utxos: Option<PathBuf>,
    pub config: Option<PathBuf>,
    /// wallet.json, to know which addresses are yours.
    pub wallet: Option<PathBuf>,
}

pub fn verify(network: Network, input: VerifyInput) -> Result<bool> {
    ui::title(t("Verifica indipendente", "Independent check"));
    ui::note(t(
        "Questo controllo non usa chiavi e non richiede rete. Ricalcola tutto dai byte delle \
         transazioni e confronta con quanto il file dichiara.",
        "This check uses no keys and needs no network. It recomputes everything from the \
         transaction bytes and compares it against what the file declares.",
    ));

    let mut wizard = Wizard::new(2);

    wizard.step(t("Lettura del file", "Reading the file"));
    let signed = verifier::load(&input.batch)?;
    let count = signed.txs.len();
    let by = &signed.created_by;
    let checksum = &signed.descriptor_checksum;
    ui::info(&tf!(
        { "{count} transazioni, prodotte da {by}, wallet {checksum}" },
        { "{count} transactions, produced by {by}, wallet {checksum}" }
    ));

    let mut expectations = Expectations {
        network: Some(network),
        ..Default::default()
    };
    if let Some(path) = &input.config {
        let config = load_config(path, network)?;
        expectations.recipients =
            Some(config.entries.iter().map(|e| e.recipient.clone()).collect());
        ui::info(t(
            "So a chi dovevano andare le monete: lo controllo.",
            "I know who the coins were meant for: I will check that.",
        ));
    }
    let coins = match &input.utxos {
        Some(path) => Some(load_utxos(path, network)?),
        None => None,
    };
    if let Some(set) = &coins {
        expectations.spendable = Some(set.utxos.iter().map(|u| u.outpoint).collect());
        ui::info(t(
            "So quali monete avevi: controllo che spenda solo quelle.",
            "I know which coins you had: I will check it spends only those.",
        ));
    }
    if let Some(path) = &input.wallet {
        let file = plebline_core::watch::load(path, network)?;
        expectations.wallet_descriptors = Some((file.external.clone(), file.internal.clone()));
        if let Some(set) = &coins {
            expectations.ownership_depth = Some(deepest_index(set) + OWNERSHIP_MARGIN);
        }
        ui::info(t(
            "So quali indirizzi sono tuoi: posso controllare anche dove torna il resto.",
            "I know which addresses are yours: I can also check where the change comes back.",
        ));
    }

    wizard.step(t("Controlli", "Checks"));
    let report = verifier::verify(&signed, &expectations);
    print_report(&report);

    if report.is_ok() {
        ui::blank();
        if report.warnings().next().is_none() {
            ui::success(t(
                "Tutti i controlli sono passati, e nessuna moneta è spesa da due transazioni.",
                "Every check passed, and no coin is spent by two transactions.",
            ));
        } else {
            ui::warning(t(
                "Nessun controllo è fallito e nessuna moneta è spesa da due transazioni, ma \
                 ci sono avvisi: leggili qui sopra prima di mandarle in rete.",
                "No check failed and no coin is spent by two transactions, but there are \
                 warnings: read them above before broadcasting.",
            ));
        }
        ui::note(t(
            "Puoi mandarle in rete quando vuoi e in qualsiasi ordine: mandarne una non \
             invalida le altre.",
            "You can broadcast them in any order, at any time. Broadcasting one does not \
             invalidate the others.",
        ));
        Ok(true)
    } else {
        ui::blank();
        ui::failure(t(
            "Qualcosa non torna. Non mandare in rete niente di questo file.",
            "Something does not add up. Do not send anything from this file.",
        ));
        Ok(false)
    }
}

/// Search margin beyond the last known index.
const OWNERSHIP_MARGIN: u32 = 200;

/// The check before a signed file is written: only listed coins spent, and
/// every output to an expected recipient or back to the signing wallet,
/// which knows its addresses.
pub(super) fn verify_signed(
    network: Network,
    signed: &SignedBatch,
    utxo_set: &ValidatedUtxoSet,
    wallet: &plebline_core::bdk_wallet::Wallet,
    recipients: Option<Vec<String>>,
) -> verifier::Report {
    verifier::verify(
        signed,
        &Expectations {
            network: Some(network),
            recipients,
            spendable: Some(utxo_set.utxos.iter().map(|u| u.outpoint).collect()),
            wallet_descriptors: Some(wallet_builder::public_descriptors(wallet)),
            ownership_depth: Some(deepest_index(utxo_set) + OWNERSHIP_MARGIN),
        },
    )
}

fn deepest_index(set: &ValidatedUtxoSet) -> u32 {
    let declared = set.last_active_indices.values().copied().max().unwrap_or(0);
    let used = set
        .utxos
        .iter()
        .map(|u| u.derivation_index)
        .max()
        .unwrap_or(0);
    declared.max(used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plebline_core::bitcoin::OutPoint;
    use plebline_core::split as split_core;
    use plebline_core::utxos;
    use plebline_core::wallet::SeedWords;

    /// The check before writing a split: on the listed coin it passes; on a
    /// coin the file does not list (with the lock `hydrate` puts on it
    /// lifted) it fails, so the split is not written.
    #[test]
    fn a_split_on_a_coin_the_file_does_not_list_fails_the_check_before_writing() {
        use plebline_core::bdk_wallet::KeychainKind;
        use plebline_core::bitcoin::consensus::encode::serialize_hex;

        let network = Network::Regtest;
        let seed = SeedWords::from_interactive_input(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon about",
        )
        .expect("seed valido");
        let mut wallet = wallet_builder::from_seed(&seed, None, network).expect("wallet");
        let spk = |index| {
            wallet
                .peek_address(KeychainKind::External, index)
                .address
                .script_pubkey()
        };
        let parent = crate::testing::paying(9, &[(spk(0), 200_000), (spk(1), 500_000)]);
        let txid = parent.compute_txid();
        let set = utxos::validate(
            utxos::UtxoSet {
                version: 1,
                network: network.to_string(),
                generated_at: 1_700_000_000,
                tip_height: 300,
                descriptor_checksum: Some(wallet.descriptor_checksum(KeychainKind::External)),
                last_active_indices: [("external".to_string(), 10)].into_iter().collect(),
                utxos: vec![utxos::UtxoEntry {
                    outpoint: format!("{txid}:0"),
                    value_sat: 200_000,
                    script_pubkey: parent.output[0].script_pubkey.to_hex_string(),
                    keychain: "external".into(),
                    derivation_index: 0,
                    confirmations: 144,
                    prev_tx_hex: serialize_hex(&parent),
                }],
            },
            network,
            "utxos.json",
        )
        .expect("il file è coerente");
        set.hydrate(&mut wallet).expect("idratato");
        let unlisted = OutPoint::new(txid, 1);
        let config = split_core::SplitConfig {
            network: network.to_string(),
            outputs: 2,
            fee_rate_sat_vb: 2.0,
        };

        let honest = split_core::build(&mut wallet, &config, []).expect("costruita");
        let honest = split_core::sign(&wallet, honest, &config).expect("firmata");
        let report = verify_signed(network, &honest, &set, &wallet, None);
        assert!(
            report.is_ok(),
            "sulla moneta elencata la divisione passa: {:?}",
            report.failures().collect::<Vec<_>>()
        );

        wallet.unlock_outpoint(unlisted);
        let plan = split_core::build(&mut wallet, &config, []).expect("costruita");
        assert!(
            plan.inputs.iter().any(|(op, _)| *op == unlisted),
            "senza il lucchetto la divisione prende anche {unlisted}"
        );
        let signed = split_core::sign(&wallet, plan, &config).expect("firmata");
        let report = verify_signed(network, &signed, &set, &wallet, None);
        let failed: Vec<verifier::CheckKind> = report.failures().map(|c| c.kind).collect();
        assert_eq!(
            failed,
            [verifier::CheckKind::Origin],
            "la verifica prima di scrivere deve rifiutare {unlisted}"
        );
    }
}
