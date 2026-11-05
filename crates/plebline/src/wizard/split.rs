//! Splitting: one transaction to yourself, so more refills can be signed.

use std::collections::BTreeSet;
use std::path::PathBuf;

use comfy_table::Cell;
use plebline_core::bitcoin::{Network, OutPoint};
use plebline_core::lang::t;
use plebline_core::split as split_core;
use plebline_core::tf;
use plebline_core::utxos::{self, ValidatedUtxoSet};
use plebline_core::{Error, Result};

use super::commitments::{commitments_of_open_wallet, read_commitments};
use super::config::load_utxos;
use super::files::{archive_existing, write_json};
use super::print::print_report;
use super::verify::verify_signed;
use super::{build_wallet, WalletAccess};
use crate::ask::{Ask, Question};
use crate::ui::{self, Wizard};

pub struct SplitInput<'a> {
    pub utxos: PathBuf,
    /// Batches signed in the past: their coins stay out of the split.
    pub previous: Vec<PathBuf>,
    pub out: PathBuf,
    pub wallet: WalletAccess<'a>,
    /// `None` asks. The guided path already knows: one coin per payment.
    pub outputs: Option<usize>,
}

/// Splits coins so more payments can be presigned (one input each).
pub fn split(network: Network, input: SplitInput<'_>, ask: &mut dyn Ask) -> Result<bool> {
    ui::title(t("Divisione delle monete", "Splitting the coins"));
    ui::note(t(
        "Una sola transazione, verso te stesso, che ridistribuisce il saldo su più monete. \
         Va trasmessa e confermata prima di firmare le ricariche: finché non è in un blocco, le \
         monete nuove non esistono.",
        "A single transaction, to yourself, that spreads the balance across more coins. It must \
         be broadcast and confirmed before the refills are signed: until it is in a block, the \
         new coins do not exist.",
    ));

    let mut wizard = Wizard::new(4);

    wizard.step(t("Controllo delle monete", "Checking the coins"));
    let utxo_set = load_utxos(&input.utxos, network)?;
    // Spending a committed coin here would cancel that signed payment.
    let committed = read_commitments(&utxo_set, &input.previous, &input.out)?;
    // Before the seed: nothing to split means no words asked.
    free_coins(&utxo_set, &committed)?;

    wizard.step("Wallet");
    let mut wallet = build_wallet(&input.wallet, network, true, ask)?;
    let committed =
        commitments_of_open_wallet(&wallet, &utxo_set, committed, &input.previous, &input.out)?;
    let free = free_coins(&utxo_set, &committed)?;
    utxo_set.hydrate(&mut wallet)?;

    wizard.step(t("In quante monete", "Into how many coins"));
    let have = free.len();
    let total: plebline_core::bitcoin::Amount = free.iter().map(|u| u.value).sum();

    // A default is fine here: this broadcasts now, at today's fees.
    let fee_rate_sat_vb: f64 = ask.range_f64(
        Question::new("Fee rate in sat/vB:").help(t(
            "questa transazione va trasmessa adesso, quindi conta il mercato di adesso",
            "this transaction is broadcast now, so what matters is the fee market of now",
        )),
        1.0..=500.0,
        Some(2.0),
    )?;

    let possible = split_core::max_outputs(total, have, fee_rate_sat_vb);
    if possible < 2 {
        return Err(Error::Refused(
            t(
                "a questa fee il saldo non basta per due monete utilizzabili: prova una fee \
                 più bassa",
                "at this fee the balance is not enough for two usable coins: try a lower fee",
            )
            .into(),
        ));
    }

    let amount = ui::sats(total.to_sat());
    ui::info(&tf!(
        {
            "Hai {have} monete libere, {amount} in totale. Al massimo puoi arrivare a \
             {possible}."
        },
        { "You have {have} free coins, {amount} in total. You can reach {possible} at most." }
    ));
    let floor = split_core::MIN_USEFUL_OUTPUT_SAT;
    ui::note(&tf!(
        {
            "Il limite viene dal fatto che sotto {floor} sat una moneta costa più di quanto \
             valga, e ogni output in più aggiunge fee."
        },
        {
            "The limit comes from the fact that below {floor} sat a coin costs more than it is \
             worth, and every extra output adds fee."
        }
    ));

    let outputs: usize = match input.outputs {
        Some(wanted) => {
            ui::info(&tf!(
                {
                    "La divido in {wanted} monete, quante ne servono per le ricariche che hai chiesto."
                },
                { "I split it into {wanted} coins, as many as the refills you asked for need." }
            ));
            wanted
        }
        None => ask.number_usize(
            Question::new(t(
                "In quante monete la divido?",
                "Into how many coins do I split it?",
            )),
            Some(possible.min(have + 2)),
        )?,
    };

    wizard.step(t("Costruzione e firma", "Building and signing"));
    let config = split_core::SplitConfig {
        network: network.to_string(),
        outputs,
        fee_rate_sat_vb,
    };
    let plan = split_core::build(&mut wallet, &config, committed.iter().copied())?;
    print_split(&plan);

    ui::blank();
    ui::warning(t(
        "Questa transazione spende TUTTE le monete libere. Quelle dentro ricariche già \
         firmate restano fuori, e quelle ricariche restano valide.",
        "This transaction spends ALL the free coins. Those inside refills already signed \
         stay out, and those refills stay valid.",
    ));

    ui::blank();
    if !ui::confirm(
        ask,
        t(
            "Confermi e firmo la divisione?",
            "Confirm and sign the split?",
        ),
    )? {
        ui::blank();
        ui::info(t(
            "Annullato. Nessuna firma è stata prodotta.",
            "Cancelled. No signature was produced.",
        ));
        return Ok(false);
    }

    let signed = split_core::sign(&wallet, plan, &config)?;
    // The same check as a signed batch. No recipients: every output must
    // come back to this wallet.
    let report = verify_signed(network, &signed, &utxo_set, &wallet, None);
    if !report.is_ok() {
        print_report(&report);
        return Err(Error::Verification(
            t(
                "la divisione appena firmata non supera la verifica: non è stata scritta su disco",
                "the split just signed fails verification: it was not written to disk",
            )
            .into(),
        ));
    }
    // A split never sent may already be on a stick: archived, its coins
    // stay committed, as with batches.
    if let Some(archived) = archive_existing(&input.out)? {
        let archived = archived.display();
        ui::info(&tf!(
            { "La divisione precedente è stata archiviata in {archived}, non cancellata." },
            { "The previous split was archived to {archived}, not deleted." }
        ));
    }
    write_json(&input.out, &signed)?;
    let out = input.out.display();
    ui::success(&tf!({ "Divisione firmata, scritta in {out}" }, {
        "Split signed, written to {out}"
    }));

    Ok(true)
}

/// The coins no signed refill holds; none is an error.
fn free_coins<'a>(
    set: &'a ValidatedUtxoSet,
    committed: &BTreeSet<OutPoint>,
) -> Result<Vec<&'a utxos::ValidatedUtxo>> {
    let free: Vec<_> = set
        .utxos
        .iter()
        .filter(|u| !committed.contains(&u.outpoint))
        .collect();
    if free.is_empty() {
        return Err(Error::Refused(
            t(
                "nessuna moneta è libera: sono tutte dentro ricariche firmate e non ancora \
                 mandate, e la divisione non le tocca",
                "no coin is free: they are all inside signed refills not sent yet, and the \
                 split does not touch them",
            )
            .into(),
        ));
    }
    Ok(free)
}

fn print_split(plan: &split_core::SplitPlan) {
    let mut table = ui::table(&[
        t("Direzione", "Direction"),
        t("Dove", "Where"),
        t("Importo", "Amount"),
    ]);
    for (outpoint, value) in &plan.inputs {
        table.add_row(vec![
            Cell::new(t("spende", "spends")),
            Cell::new(ui::short_outpoint(&outpoint.to_string())),
            ui::right(ui::sats(value.to_sat())),
        ]);
    }
    for (address, value) in &plan.outputs {
        table.add_row(vec![
            Cell::new(t("crea", "creates")),
            Cell::new(ui::short_address(&address.to_string())),
            ui::right(ui::sats(value.to_sat())),
        ]);
    }
    table.add_row(vec![
        Cell::new("fee"),
        Cell::new(format!(
            "{:.2} sat/vB",
            plan.fee.to_sat() as f64 / plan.vsize as f64
        )),
        ui::right(ui::sats(plan.fee.to_sat())),
    ]);
    ui::show(&table);

    let before = plan.inputs.len();
    let after = plan.outputs.len();
    ui::success(&tf!(
        {
            "Da {before} monete a {after}: potrai prefirmare fino a {after} ricariche \
             indipendenti."
        },
        {
            "From {before} coins to {after}: you will be able to pre-sign up to {after} \
             independent refills."
        }
    ));
}
