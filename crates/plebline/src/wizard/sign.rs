//! Signing: the plan rebuilt, shown, confirmed, signed, checked, written.

use std::path::{Path, PathBuf};

use plebline_core::batch::{self, SignedBatch};
use plebline_core::bitcoin::Network;
use plebline_core::lang::t;
use plebline_core::plan::{self, PlannedBatch};
use plebline_core::tf;
use plebline_core::{Error, Result};

use super::commitments::{commitments_of_open_wallet, read_commitments, weigh_commitments};
use super::config::{load_or_prompt_config, load_plan, load_utxos};
use super::files::{archive_existing, write_json, write_per_tx_files, PER_TX_PREFIX};
use super::print::{print_plan, print_report};
use super::verify::verify_signed;
use super::{build_wallet, WalletAccess};
use crate::ask::Ask;
use crate::ui::{self, Wizard};

pub struct SignInput<'a> {
    pub utxos: PathBuf,
    pub config: Option<PathBuf>,
    /// Where to save the definition, if it is asked for on screen.
    pub config_out: Option<PathBuf>,
    pub plan: Option<PathBuf>,
    /// Where to write the rebuilt plan, as a record of what was signed.
    pub plan_out: Option<PathBuf>,
    /// Batches signed in the past, whose inputs must not be reused.
    pub previous: Vec<PathBuf>,
    pub out: PathBuf,
    pub wallet: WalletAccess<'a>,
}

pub fn sign(network: Network, input: SignInput<'_>, ask: &mut dyn Ask) -> Result<bool> {
    ui::title(t("Firma del batch", "Signing the batch"));
    ui::note(t(
        "Da qui in avanti si usano le chiavi private. Nulla viene trasmesso: il risultato \
         sono transazioni firmate, da mandare in rete quando serve.",
        "From here on the private keys are used. Nothing is broadcast: the result is signed \
         transactions, to broadcast when you need them.",
    ));

    let mut wizard = Wizard::new(6);

    wizard.step(t("Controllo delle monete", "Checking the coins"));
    let utxo_set = load_utxos(&input.utxos, network)?;
    // Read now (before the seed), decided at step 4.
    let committed = read_commitments(&utxo_set, &input.previous, &input.out)?;

    wizard.step("Wallet");
    let mut wallet = build_wallet(&input.wallet, network, true, ask)?;
    let committed =
        commitments_of_open_wallet(&wallet, &utxo_set, committed, &input.previous, &input.out)?;
    utxo_set.hydrate(&mut wallet)?;
    ui::success(t(
        "Chiavi pronte, gli UTXO risultano spendibili da questo wallet.",
        "Keys ready, the UTXOs are spendable by this wallet.",
    ));

    wizard.step(t("Cosa vuoi ricaricare", "What you want to refill"));
    let config = load_or_prompt_config(
        input.config.as_deref(),
        network,
        input.config_out.as_deref(),
        ask,
    )?;

    wizard.step(t(
        "Ricostruzione delle transazioni",
        "Rebuilding the transactions",
    ));

    let weighed = weigh_commitments(
        utxo_set.utxos.iter().map(|u| u.outpoint),
        &committed,
        config.entries.len(),
    );
    if let Some((count, free)) = weighed.counts() {
        ui::warning(&tf!(
            {
                "{count} monete sono già state usate in una firma precedente e non verranno \
                 riusate. Restano {free} monete libere."
            },
            {
                "{count} coins were already used in an earlier signing and will not be reused. \
                 {free} coins remain free."
            }
        ));
    }
    // Refuse after showing the numbers.
    if let Some(refusal) = weighed.shortage() {
        return Err(refusal);
    }

    let built = plan::build(&mut wallet, &config, committed.iter().copied())?;
    let planned = plan::to_plan(&built, &config, utxo_set.generated_at, utxo_set.tip_height);

    if let Some(plan_path) = &input.plan {
        let expected = load_plan(plan_path)?;
        compare_plans(&expected, &planned)?;
        ui::success(t(
            "La selezione coincide con il piano approvato, input per input.",
            "The selection matches the approved plan, input by input.",
        ));
    }

    // Written anyway: a keyless record of what was signed.
    if let Some(path) = &input.plan_out {
        write_json(path, &planned)?;
    }

    wizard.step(t("Guarda bene", "Look carefully"));
    print_plan(&planned);
    ui::blank();
    ui::note(t(
        "Sta per essere apposta una firma valida su ognuna di queste transazioni. \
         Chiunque ottenga il file risultante potrà trasmetterle.",
        "A valid signature is about to be placed on each of these transactions. Anyone who gets \
         the resulting file will be able to broadcast them.",
    ));
    ui::blank();
    if !ui::confirm(
        ask,
        t(
            "Confermi e procedi con la firma?",
            "Confirm and proceed with signing?",
        ),
    )? {
        ui::blank();
        ui::info(t(
            "Annullato. Nessuna firma è stata prodotta.",
            "Cancelled. No signature was produced.",
        ));
        return Ok(false);
    }

    wizard.step(t("Firma", "Signing"));
    let signed = batch::sign(&wallet, built, config.fee_rate_sat_vb)?;

    // Verified as a third party would, before anything is written.
    let report = verify_signed(
        network,
        &signed,
        &utxo_set,
        &wallet,
        Some(config.entries.iter().map(|e| e.recipient.clone()).collect()),
    );
    if !report.is_ok() {
        print_report(&report);
        return Err(Error::Verification(
            t(
                "il batch appena firmato non supera la verifica: non è stato scritto su disco",
                "the batch just signed fails verification: it was not written to disk",
            )
            .into(),
        ));
    }
    ui::success(t(
        "Il batch firmato supera tutti i controlli.",
        "The signed batch passes every check.",
    ));

    if let Some(archived) = archive_existing(&input.out)? {
        let archived = archived.display();
        ui::info(&tf!(
            { "Il batch precedente è stato archiviato in {archived}, non cancellato." },
            { "The previous batch was archived to {archived}, not deleted." }
        ));
    }

    write_json(&input.out, &signed)?;
    let out = input.out.display();
    ui::success(&tf!({ "Batch scritto in {out}" }, {
        "Batch written to {out}"
    }));

    let per_tx = write_per_tx_files(&signed, &input.out)?;
    if !per_tx.is_empty() {
        let last = per_tx.len() - 1;
        ui::success(&if last == 0 {
            tf!({ "Scritto anche l'hex della transazione: tx-0.hex" }, {
                "Also written the transaction's hex: tx-0.hex"
            })
        } else {
            tf!(
                { "Scritto anche l'hex di ogni transazione: tx-0.hex … tx-{last}.hex" },
                { "Also written each transaction's hex: tx-0.hex … tx-{last}.hex" }
            )
        });
    }

    print_operational_reminder(&input, &signed);
    Ok(true)
}

fn compare_plans(expected: &PlannedBatch, actual: &PlannedBatch) -> Result<()> {
    if expected.txs.len() != actual.txs.len() {
        let wanted = expected.txs.len();
        let got = actual.txs.len();
        return Err(Error::Verification(tf!(
            { "il piano approvato ha {wanted} transazioni, la ricostruzione ne ha {got}" },
            { "the approved plan has {wanted} transactions, the rebuild has {got}" }
        )));
    }
    for (a, b) in expected.txs.iter().zip(actual.txs.iter()) {
        let index = batch::number(a.index);
        if a.inputs != b.inputs {
            let planned = a
                .inputs
                .iter()
                .map(|i| i.outpoint.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let chosen = b
                .inputs
                .iter()
                .map(|i| i.outpoint.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::Verification(tf!(
                {
                    "transazione #{index}: il piano prevedeva [{planned}], la ricostruzione ha \
                  scelto [{chosen}]. Il set di UTXO è cambiato rispetto a quando il piano è \
                  stato approvato."
                },
                {
                    "transaction #{index}: the plan expected [{planned}], the rebuild chose \
                  [{chosen}]. The UTXO set changed since the plan was approved."
                }
            )));
        }
        if a.recipient != b.recipient || a.amount_sat != b.amount_sat {
            return Err(Error::Verification(tf!(
                { "transazione #{index}: destinazione o importo diversi dal piano approvato" },
                { "transaction #{index}: recipient or amount differs from the approved plan" }
            )));
        }
    }
    Ok(())
}

fn print_operational_reminder(input: &SignInput<'_>, signed: &SignedBatch) {
    ui::blank();
    ui::title(t("Promemoria operativo", "Operational reminder"));

    let out = input.out.display();
    ui::info(&tf!({ "File prodotto: {out}" }, { "File produced: {out}" }));
    if signed.txs.len() > 1 {
        let last = signed.txs.len().saturating_sub(1);
        ui::info(&tf!(
            { "Hex di ogni transazione: tx-0.hex … tx-{last}.hex, stessa cartella" },
            { "Each transaction's hex: tx-0.hex … tx-{last}.hex, same folder" }
        ));
    }
    ui::note(t(
        "Il file non contiene chiavi private: contiene transazioni già firmate, quindi chi lo \
         possiede può trasmetterle, ma non può cambiarne destinazione o importo.",
        "The file holds no private keys: it holds already-signed transactions, so whoever has \
         it can broadcast them, but cannot change their recipient or amount.",
    ));

    ui::blank();
    ui::info(t(
        "Se una transazione resta bloccata in mempool",
        "If a transaction gets stuck in the mempool",
    ));
    ui::note(t(
        "Non puoi rimpiazzarla: la firma è già stata prodotta e non è modificabile. \
         La leva è il CPFP, cioè spendere l'output ricevuto con una transazione a fee alta, \
         che tira in blocco anche la genitrice. Per questo conviene che la destinazione sia \
         un indirizzo di cui controlli le chiavi.",
        "You cannot replace it: the signature is already made and cannot be changed. The lever \
         is CPFP: spending the received output with a high-fee transaction, which drags the \
         parent into a block with it. That is why the recipient should be an address whose keys \
         you control.",
    ));

    ui::blank();
    ui::info(t("Questo batch resta valido", "This batch stays valid"));
    ui::note(t(
        "Le transazioni qui dentro si possono trasmettere finché le loro monete non vengono \
         spese da un'altra transazione, e in questa seduta il programma non riusa quelle \
         monete. Se perdi il controllo di una copia, l'unico modo di renderla inutile è \
         spendere prima quelle monete.",
        "The transactions in here can be broadcast until their coins are spent by another \
         transaction, and in this session the program does not reuse those coins. If you lose \
         control of a copy, the only way to make it useless is to spend those coins first.",
    ));

    ui::blank();
    let hex_file = input
        .out
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{PER_TX_PREFIX}K.hex"))
        .display()
        .to_string();
    ui::note(&tf!(
        {
            "Da una macchina senza plebline: incolla il contenuto di {hex_file} nel push di un \
             explorer, o passalo a `sendrawtransaction`. È tutto quello che serve per \
             trasmettere; per capire cosa contiene serve invece batch.json."
        },
        {
            "From a machine without plebline: paste the contents of {hex_file} into an \
             explorer's push form, or hand it to `sendrawtransaction`. That is all broadcasting \
             takes; understanding what it contains is what batch.json is for."
        }
    ));
    ui::blank();
}
