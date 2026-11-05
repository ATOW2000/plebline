//! Tables shared by several flows: the plan, and the verification report.

use std::collections::BTreeSet;

use comfy_table::Cell;
use plebline_core::lang::t;
use plebline_core::plan::PlannedBatch;
use plebline_core::tf;
use plebline_core::verify::{self as verifier, Severity};

use crate::ui;

pub fn print_plan(planned: &PlannedBatch) {
    let mut table = ui::table(&[
        "#",
        t("Nota", "Label"),
        t("Moneta spesa", "Coin spent"),
        t("A chi", "To whom"),
        t("Quanto", "How much"),
        "Fee",
        "sat/vB",
        t("Torna indietro", "Comes back"),
    ]);
    for tx in &planned.txs {
        let inputs = tx
            .inputs
            .iter()
            .map(|i| ui::short_outpoint(&i.outpoint))
            .collect::<Vec<_>>()
            .join("\n");
        table.add_row(vec![
            ui::right(plebline_core::batch::number(tx.index)),
            Cell::new(tx.label.clone().unwrap_or_default()),
            Cell::new(inputs),
            Cell::new(ui::short_address(&tx.recipient)),
            ui::right(ui::sats(tx.amount_sat)),
            ui::right(ui::sats(tx.fee_sat)),
            ui::right(format!("{:.2}", tx.fee_rate_sat_vb)),
            ui::right(
                tx.change_sat
                    .map(ui::sats)
                    .unwrap_or_else(|| "—".to_string()),
            ),
        ]);
    }
    ui::show(&table);

    let sats = ui::sats(planned.total_amount());
    let btc = ui::btc(planned.total_amount());
    let fees = ui::sats(planned.total_fee());
    ui::info(&tf!(
        { "Totale inviato {sats} ({btc}), fee complessive {fees}." },
        { "Total sent {sats} ({btc}), total fees {fees}." }
    ));

    let all_inputs: Vec<String> = planned
        .txs
        .iter()
        .flat_map(|t| t.inputs.iter().map(|i| i.outpoint.clone()))
        .collect();
    let unique: BTreeSet<&String> = all_inputs.iter().collect();
    let total = all_inputs.len();
    let distinct = unique.len();
    if distinct == total {
        ui::success(&tf!(
            {
                "{total} input in totale, {distinct} distinti: nessuna transazione condivide un \
                 input con un'altra."
            },
            {
                "{total} inputs in total, {distinct} distinct: no transaction shares an input \
                 with another."
            }
        ));
    } else {
        ui::failure(&tf!(
            {
                "{total} input in totale ma solo {distinct} distinti: due transazioni spendono la \
                 stessa moneta. Non firmare."
            },
            {
                "{total} inputs in total but only {distinct} distinct: two transactions spend the \
                 same coin. Do not sign."
            }
        ));
    }
}

pub(super) fn print_report(report: &verifier::Report) {
    let mut table = ui::table(&[
        t("Esito", "Result"),
        t("Cosa ho guardato", "What I checked"),
        t("Cosa ho trovato", "What I found"),
    ]);
    for check in &report.checks {
        // Bare text: the screen escapes what a terminal would obey, colour
        // codes included.
        let marker = match check.severity {
            Severity::Pass => "ok",
            Severity::Warn => t("attenzione", "warning"),
            Severity::Fail => t("FALLITO", "FAILED"),
        };
        table.add_row(vec![
            Cell::new(marker),
            Cell::new(&check.name),
            Cell::new(&check.detail),
        ]);
    }
    ui::show(&table);
}
