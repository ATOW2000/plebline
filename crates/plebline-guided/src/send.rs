//! Broadcasting a transaction that is already signed.

use plebline_core::lang::t;
use plebline_core::{tf, Result};

use plebline::ask::{Ask, Question};
use plebline::project::{self, Project};
use plebline::ui;

/// After the export, asks whether to send one now. Only the answer is
/// returned: `home` does the sending, and drops the words before the
/// network is plugged back in.
pub(crate) fn step_send(ceremony: &Project, ask: &mut dyn Ask) -> Result<bool> {
    ui::title(t("Ci sei", "You're done"));

    let dir = project::display(&ceremony.dir);
    ui::note(&tf!(
        {
            "Le tue ricariche firmate stanno in {dir}. Non le tocca nessuno finché non sei tu a \
             mandarle in rete, una alla volta, quando ti serve."
        },
        {
            "Your signed refills are in {dir}. Nobody touches them until you send them to the \
             network yourself, one at a time, when you need to."
        }
    ));

    ui::blank();
    let send_now = ask.confirm(
        Question::new(t(
            "Vuoi mandarne una adesso?",
            "Do you want to send one now?",
        ))
        .help(t(
            "serve riattaccare la rete, e prima tolgo le tue parole dalla memoria. Puoi \
                 farlo anche dopo, dal portafoglio",
            "this needs the network back on, and first I remove your words from memory. You \
                 can also do it later, from the wallet",
        )),
        false,
    )?;
    if !send_now {
        ui::blank();
        ui::info(t("Torno al portafoglio.", "Back to the wallet."));
    }
    Ok(send_now)
}

/// Which payment to send: only the user knows.
pub(crate) fn pick_transaction(ceremony: &Project, ask: &mut dyn Ask) -> Result<usize> {
    let batch = plebline_core::verify::load(&ceremony.path(project::BATCH))?;
    let labels: Vec<String> = batch
        .txs
        .iter()
        .map(|tx| {
            let index = plebline_core::batch::number(tx.index);
            let amount: u64 = tx
                .outputs
                .iter()
                .filter(|o| !o.is_change)
                .map(|o| o.value_sat)
                .sum();
            let sats = ui::sats(amount);
            match &tx.label {
                Some(label) => format!("#{index}  {sats}  · {label}"),
                None => format!("#{index}  {sats}"),
            }
        })
        .collect();

    let shown: Vec<&str> = labels.iter().map(String::as_str).collect();
    // The index, never a label lookup with a silent fallback.
    let position = ask.select(
        Question::new(t("Quale mando?", "Which one do I send?")),
        &shown,
        None,
    )?;
    Ok(batch.txs[position].index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};

    /// Broadcasts the file's `index`, not the list position.
    #[test]
    fn i_send_the_transaction_i_chose_not_the_first() {
        in_italian();
        let temp = Temp::new("quale");
        let ceremony = temp.ceremony();
        write_batch(&ceremony, &[3, 7, 11]);

        let mut ask = Scripted::new([Reply::Choice(1)]);
        assert_eq!(
            pick_transaction(&ceremony, &mut ask).expect("scelta"),
            7,
            "la seconda voce dell'elenco è la transazione con indice 7"
        );

        // And the list shown is the file's, in that order, counted from 1.
        let entries = &ask.asked()[0].options;
        assert_eq!(entries.len(), 3);
        assert!(entries[0].starts_with("#4"), "{entries:?}");
        assert!(entries[2].starts_with("#12"), "{entries:?}");
    }

    #[test]
    fn choosing_by_label_finds_the_same_transaction() {
        in_italian();
        let temp = Temp::new("etichetta");
        let ceremony = temp.ceremony();
        write_batch(&ceremony, &[3, 7, 11]);

        let mut ask = Scripted::new([Reply::like("pagamento 11")]);
        assert_eq!(pick_transaction(&ceremony, &mut ask).expect("scelta"), 11);
    }
}
