//! What the flows read or ask: the coin list, the batch definition, the plan.

use std::path::Path;
use std::str::FromStr;

use plebline_core::batch;
use plebline_core::bitcoin::Network;
use plebline_core::lang::t;
use plebline_core::plan::{BatchConfig, BatchEntry, PlannedBatch};
use plebline_core::tf;
use plebline_core::utxos::{self, ValidatedUtxoSet};
use plebline_core::{Error, Result};

use super::files::write_json;
use crate::ask::{Ask, Question};
use crate::ui;

pub fn load_utxos(path: &Path, network: Network) -> Result<ValidatedUtxoSet> {
    let set = utxos::load(path, network)?;
    if set.utxos.is_empty() {
        return Err(Error::InvalidUtxo {
            outpoint: path.display().to_string(),
            reason: t(
                "il file non contiene alcun UTXO",
                "the file contains no UTXO",
            )
            .into(),
        });
    }
    Ok(set)
}

/// What `ask_batch_config` would never produce is refused here too.
pub(super) fn load_config(path: &Path, network: Network) -> Result<BatchConfig> {
    let config: BatchConfig = plebline_core::read_json(path)?;
    if Network::from_str(&config.network).ok() != Some(network) {
        return Err(Error::NetworkMismatch {
            found: config.network,
            expected: network.to_string(),
        });
    }
    if config.entries.is_empty() {
        let path = path.display();
        return Err(Error::Input(tf!(
            { "{path} non chiede nessuna ricarica" },
            { "{path} asks for no refill" }
        )));
    }
    Ok(config)
}

pub(super) fn load_plan(path: &Path) -> Result<PlannedBatch> {
    plebline_core::read_json(path)
}

/// Loads the batch definition or asks for it; `save_to` keeps the answer
/// so the next step does not ask again.
pub fn load_or_prompt_config(
    path: Option<&Path>,
    network: Network,
    save_to: Option<&Path>,
    ask: &mut dyn Ask,
) -> Result<BatchConfig> {
    if let Some(path) = path {
        let config = load_config(path, network)?;
        let count = config.entries.len();
        let rate = config.fee_rate_sat_vb;
        ui::success(&tf!(
            { "Ricariche richieste: {count}, a {rate} sat/vB." },
            { "Refills requested: {count}, at {rate} sat/vB." }
        ));
        return Ok(config);
    }

    ui::note(t(
        "Nessun file di configurazione: definiamo il batch adesso, una ricarica alla volta.",
        "No configuration file: let's define the batch now, one refill at a time.",
    ));
    let config = ask_batch_config(network, None, &[], ask)?;
    if let Some(path) = save_to {
        write_json(path, &config)?;
        let out = path.display();
        ui::success(&tf!({ "Definizione salvata in {out}" }, {
            "Definition saved to {out}"
        }));
    }
    Ok(config)
}

/// Asks for the batch: count, fee, then recipient and amount each.
/// `count` skips the first question when already known. `received[i]`, when
/// present, is payment `i`'s recipient (a list sent from the phone): shown,
/// not asked; amount and label are still asked.
pub fn ask_batch_config(
    network: Network,
    count: Option<usize>,
    received: &[plebline_core::address::Destination],
    ask: &mut dyn Ask,
) -> Result<BatchConfig> {
    let count: usize = match count {
        Some(known) => known,
        None => ask.number_usize(
            Question::new(t(
                "Quante ricariche prefirmate?",
                "How many presigned refills?",
            ))
            .invalid(t(
                "Inserisci un numero intero maggiore di zero",
                "Enter an integer greater than zero",
            )),
            None,
        )?,
    };
    if count == 0 {
        return Err(Error::Input(
            t(
                "un batch vuoto non ha senso",
                "an empty batch makes no sense",
            )
            .into(),
        ));
    }

    // The fee advice points to another source: the server this program queries
    // already sees the addresses, and should not be the only source for a
    // number that must hold for months.
    ui::blank();
    ui::note(t(
        "Prima di rispondere: guarda quanto si paga adesso su mempool.space, dal telefono o da \
         un altro computer. È l'unico numero di tutta la cerimonia che conviene prendere da una \
         fonte diversa da questo programma.",
        "Before answering: check what the going rate is right now on mempool.space, from your \
         phone or another computer. It is the one number in the whole ceremony worth taking from \
         a source other than this program.",
    ));
    ui::note(t(
        "Queste ricariche devono restare valide per mesi, quindi conviene stare sopra al \
         prezzo di oggi: una firmata troppo bassa resta ferma, e per rifarla dovresti riaprire \
         il cold storage.",
        "These refills have to stay valid for months, so it pays to stay above today's \
         price: one signed too low sits there, and redoing it would mean opening the cold \
         storage again.",
    ));

    // No default: this fee must hold for months, the user must choose it.
    let fee_rate_sat_vb: f64 = ask.range_f64(
        Question::new("Fee rate in sat/vB:").help(t(
            "vale per tutte le ricariche del batch. Sceglilo alto abbastanza da restare \
                 plausibile fra sei mesi: una tx bloccata si sblocca con CPFP, non abbassando \
                 la fee",
            "it applies to every refill in the batch. Pick it high enough to still be \
                 plausible in six months: a stuck tx is freed with CPFP, not by lowering the fee",
        )),
        1.0..=500.0,
        None,
    )?;

    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        ui::blank();
        let shown = batch::number(index);
        ui::info(&tf!({ "Ricarica #{shown}" }, { "Refill #{shown}" }));
        let destination = match received.get(index) {
            Some(sent) => {
                ui::note(t(
                    "  Indirizzo di destinazione, arrivato dal telefono:",
                    "  Destination address, sent from the phone:",
                ));
                ui::note(&format!(
                    "  {}",
                    crate::destination::in_groups(&sent.address.to_string())
                ));
                sent.clone()
            }
            // `address` accepts paste, BIP 21 URI, or a QR image from the stick.
            None => ask.address(
                Question::new(t("  Indirizzo di destinazione:", "  Destination address:"))
                    .help(crate::destination::help_text()),
                network,
            )?,
        };
        let recipient = destination.address.to_string();
        // An amount from a QR is only the default; the user can change it.
        let amount_sat = ask.number_u64(
            Question::new(t("  Importo in sat:", "  Amount in sat:")),
            destination.amount_sat,
        )?;
        let label = ask.text(
            Question::new(t("  Nota (facoltativa):", "  Label (optional):")),
            destination.label.as_deref(),
        )?;
        entries.push(BatchEntry {
            label: if label.trim().is_empty() {
                None
            } else {
                Some(label)
            },
            recipient,
            amount_sat,
        });
    }

    Ok(BatchConfig {
        network: network.to_string(),
        fee_rate_sat_vb,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Temp;

    /// Addresses sent from the phone fill the first payments in order and
    /// are not asked again; the rest are asked; amounts are always asked.
    #[test]
    fn addresses_from_the_phone_fill_the_first_payments() {
        use crate::ask::{Reply, Scripted};
        crate::testing::in_italian();
        let from_phone = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        let typed = "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7";
        let received = plebline_core::address::parse_list(
            &format!("bitcoin:{from_phone}?amount=0.0002"),
            Network::Signet,
        )
        .unwrap();

        let mut ask = Scripted::new([
            Reply::number(10.0),
            // Payment 0: amount proposed by the phone, taken; no label.
            Reply::number(""),
            Reply::text(""),
            // Payment 1: typed here.
            Reply::text(typed),
            Reply::number(5_000u64),
            Reply::text(""),
        ]);
        let config = ask_batch_config(Network::Signet, Some(2), &received, &mut ask).unwrap();

        assert_eq!(config.entries[0].recipient, from_phone);
        assert_eq!(config.entries[0].amount_sat, 20_000);
        assert_eq!(config.entries[1].recipient, typed);
        assert_eq!(ask.remaining(), 0);
        let address_questions = ask
            .prompts()
            .iter()
            .filter(|p| p.contains("Indirizzo di destinazione"))
            .count();
        assert_eq!(address_questions, 1, "{:?}", ask.prompts());
    }

    /// What the questions would never produce is refused from a file too.
    #[test]
    fn a_definition_for_another_network_or_for_nothing_is_refused() {
        let dir = Temp::new("definizione");
        let path = dir.path.join(crate::project::CONFIG);
        let config = |network: &str, entries: Vec<BatchEntry>| BatchConfig {
            network: network.into(),
            fee_rate_sat_vb: 2.0,
            entries,
        };
        let entry = BatchEntry {
            label: None,
            recipient: "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".into(),
            amount_sat: 10_000,
        };

        write_json(&path, &config("signet", vec![entry.clone()])).expect("scritto");
        load_config(&path, Network::Signet).expect("buona");

        write_json(&path, &config("bitcoin", vec![entry.clone()])).expect("scritto");
        assert!(load_config(&path, Network::Signet).is_err(), "altra rete");

        write_json(&path, &config("signet", Vec::new())).expect("scritto");
        assert!(
            load_config(&path, Network::Signet).is_err(),
            "nessuna ricarica"
        );

        std::fs::write(&path, "{\"network\": \"signet\", \"entries\": [").expect("scrivibile");
        assert!(load_config(&path, Network::Signet).is_err(), "troncata");
    }
}
