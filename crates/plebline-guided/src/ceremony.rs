//! Opening a ceremony, from what is already in the directory.

use plebline_core::lang::t;
use plebline_core::{tf, Result};

use plebline::project::{self, Project};
use plebline::ui;

/// Every opening starts from zero: the old snapshot, definition and plan
/// go, and the signed files of earlier sessions are set aside, kept but
/// never read again. Their refills may be on a phone, sent or not; their
/// coins look free from here, and the reminder before signing says so.
pub(crate) fn start_fresh(ceremony: &Project) -> Result<()> {
    for file in [project::UTXOS, project::CONFIG, project::PLAN] {
        ceremony.remove_file(file)?;
    }
    let Some(dir) = project::set_aside(ceremony)? else {
        return Ok(());
    };
    let dir = project::display(&dir);
    ui::blank();
    ui::note(&tf!(
        {
            "Riparto da zero. Le ricariche firmate le volte prima le ho spostate in {dir}: \
             restano lì finché non scegli «Chiudi e cancella», ma non le guardo più, e da qui le \
             loro monete sembrano libere."
        },
        {
            "Starting from zero. The refills signed in earlier sessions are now in {dir}: they \
             stay there until you choose \"Close and delete\", but I no longer look at them, and \
             from here their coins look free."
        }
    ));
    Ok(())
}

/// No coins found. Usually after a split broadcast but not yet mined: the
/// old coins are spent, the new ones not counted yet. Said so, not "no
/// UTXO".
pub(crate) fn nothing_found(ceremony: &Project) -> Result<()> {
    ui::blank();
    if ceremony.has(project::SPLIT) {
        ui::title(t(
            "La divisione non è ancora in un blocco",
            "The split is not in a block yet",
        ));
        ui::note(t(
            "È normale, e non hai perso niente: le monete vecchie sono già state spese dalla \
             divisione, e le nuove esistono solo quando la transazione entra in un blocco. Nel \
             frattempo non risultano da nessuna parte.",
            "This is normal, and nothing is lost: the old coins have already been spent by the \
             split, and the new ones only exist once the transaction gets into a block. In the \
             meantime they show up nowhere.",
        ));
        ui::blank();
        ui::info(t(
            "Aspetta ancora un po' e riapri il programma. Se dopo un'ora non è cambiato niente, la \
             commissione della divisione era troppo bassa.",
            "Wait a little longer and open the program again. If nothing has changed after an \
             hour, the split's fee was too low.",
        ));
        return Ok(());
    }

    ui::title(t("Non ho trovato monete", "I found no coins"));
    ui::note(t(
        "Il wallet è quello giusto, ma su questi indirizzi non risulta niente di spendibile. O non \
         ci hai ancora mandato nulla, o quelle monete si spendono con un altro wallet.",
        "The wallet is the right one, but nothing spendable shows up on these addresses. Either \
         you have not sent anything yet, or those coins are spent with a different wallet.",
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;

    /// Last time's groups are set aside, kept, and commit nothing; the
    /// coins are read again; the public half stays.
    #[test]
    fn starting_fresh_sets_earlier_refills_aside_and_keeps_them() {
        in_italian();
        let temp = Temp::new("da-zero");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);
        write_batch(&ceremony, &[0, 1]);
        write_group(&ceremony, "batch-1.json", &[2]);

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        start_fresh(&ceremony).expect("ripartenza");
        plebline::out::take();

        assert!(project::signed_files(&ceremony).is_empty());
        let committed = plebline::wizard::committed_outpoints(
            &project::signed_files(&ceremony),
            &ceremony.path(project::BATCH),
            None,
        )
        .expect("letti");
        assert!(
            committed.is_empty(),
            "nessuna moneta impegnata: {committed:?}"
        );

        let aside = ceremony.path(project::SET_ASIDE).join("1");
        for file in [project::BATCH, "batch-1.json"] {
            assert!(aside.join(file).is_file(), "{file} tenuto");
        }
        assert!(!ceremony.has(project::UTXOS), "le monete si rileggono");
        assert!(ceremony.has(project::WALLET), "la parte pubblica resta");
        assert!(
            notebook.text().contains("Riparto da zero"),
            "{}",
            notebook.text()
        );
    }

    /// Nothing signed before: nothing moved, nothing said.
    #[test]
    fn starting_fresh_with_nothing_signed_says_nothing() {
        in_italian();
        let temp = Temp::new("da-zero-vuota");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        start_fresh(&ceremony).expect("ripartenza");
        plebline::out::take();

        assert!(!ceremony.path(project::SET_ASIDE).exists());
        assert!(!ceremony.has(project::UTXOS));
        assert!(notebook.text().is_empty(), "{}", notebook.text());
    }
}
