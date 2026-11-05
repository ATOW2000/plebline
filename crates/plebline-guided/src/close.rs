//! Closing: ask before deleting signed files that were not taken away,
//! delete the working folder, forget the wallet.

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

use plebline::ask::Ask;
use plebline::project::{self, Project};
use plebline::ui;

/// Deletes the working directory, so nothing about the wallet stays on disk.
///
/// `wallet.json` reveals every address forever, so it goes too, last
/// (every chain step reads it). Next time the seed is asked first, with
/// the network on; the receiving addresses just exported are the way to
/// add coins meanwhile.
///
/// The cost: an unsent payment's coins look free on chain, and the next
/// session, which starts from zero, can re-sign them. The closing warning
/// says so. A signed file with no copy elsewhere is deleted only after
/// asking, default no. A file counts as taken away when every payment in
/// it was copied this session (`copied` holds their txids); splits and
/// groups set aside never are.
pub(crate) fn step_close(
    ceremony: &Project,
    copied: &std::collections::BTreeSet<String>,
    ask: &mut dyn Ask,
) -> Result<()> {
    // An unreadable file is not known to be copied: it stays on the list.
    let taken_away = |path: &std::path::Path| {
        plebline_core::verify::load(path).is_ok_and(|group| {
            !group.txs.is_empty() && group.txs.iter().all(|tx| copied.contains(&tx.txid))
        })
    };
    let mut here_only: Vec<String> = project::signed_files(ceremony)
        .iter()
        .filter(|path| !taken_away(path))
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    here_only.extend(
        project::set_aside_files(ceremony)
            .iter()
            .filter_map(|path| path.strip_prefix(&ceremony.dir).ok())
            .map(|path| path.display().to_string()),
    );
    if !here_only.is_empty() {
        ui::blank();
        if here_only == [project::BATCH] && copied.is_empty() {
            ui::warning(t(
                "Non hai portato via niente, e queste ricariche firmate esistono solo qui: se \
                 cancello, spariscono.",
                "You took nothing away, and these signed refills exist only here: if I delete \
                 them, they are gone.",
            ));
        } else {
            let list = here_only.join(", ");
            ui::warning(&tf!(
                {
                    "Qui ci sono transazioni firmate che in questa seduta non hai portato via: \
                     {list}. Se qualcuna non è ancora stata mandata, esiste solo qui: se \
                     cancello, sparisce, e le sue monete sembreranno libere."
                },
                {
                    "There are signed transactions here that you did not take away in this \
                     session: {list}. If any of them has not been sent yet, it exists only \
                     here: if I delete, it is gone, and its coins will look free."
                }
            ));
        }
        if !ui::confirm(
            ask,
            t(
                "Cancello lo stesso la cartella di lavoro?",
                "Delete the working folder anyway?",
            ),
        )? {
            ui::blank();
            let dir = project::display(&ceremony.dir);
            ui::note(&tf!(
                {
                    "Lascio tutto in {dir}. Portale via appena puoi: finché sono solo lì, se \
                     perdi quella cartella perdi le ricariche."
                },
                {
                    "I leave everything in {dir}. Take them away as soon as you can: while they \
                     are only there, losing that folder means losing the refills."
                }
            ));
            // Payments stay; the address list does not.
            return step_forget_wallet(ceremony);
        }
    }

    let deleted = project::wipe(ceremony)?;

    ui::blank();
    ui::title(t("Non resta niente qui", "Nothing is left here"));
    ui::note(&tf!(
        {
            "Ho cancellato {deleted} file: la parte pubblica del portafoglio, le monete che \
             avevo visto, le ricariche e tutto il resto. Non era roba che apriva niente, ma \
             diceva tutti i tuoi indirizzi e quanto c'è su ognuno."
        },
        {
            "I deleted {deleted} files: the wallet's public half, the coins I had seen, the \
             refills and everything else. None of it opened anything, but it listed every \
             address of yours and how much sits on each."
        }
    ));
    ui::blank();
    ui::note(t(
        "Le tue parole non le ho mai scritte da nessuna parte, e adesso non le ho più nemmeno \
         in memoria. Di questa cerimonia resta solo quello che hai portato via tu.",
        "I never wrote your words anywhere, and now I no longer have them in memory either. \
         All that is left of this ceremony is what you took away.",
    ));

    ui::blank();
    ui::warning(t(
        "Una cosa da sapere prima della prossima volta: finché una ricarica non è stata \
         trasmessa, le monete che spende risultano libere sulla catena, e io non ho più il file \
         che se ne ricordava. Se apri una cerimonia nuova con delle ricariche ancora da mandare, \
         posso firmare di nuovo le stesse monete, e allora una delle due ricariche verrà \
         rifiutata per sempre. Mandale in rete prima di ricominciare.",
        "One thing to know before next time: until a refill has been broadcast, the coins it \
         spends look free on the chain, and I no longer have the file that remembered them. If \
         you open a new ceremony while some refills are still unsent, I can sign the same coins \
         again, and then one of the two refills will be rejected forever. Send them before you \
         start over.",
    ));
    Ok(())
}

pub(crate) fn step_forget_wallet(ceremony: &Project) -> Result<()> {
    if !project::forget_wallet(ceremony)? {
        return Ok(());
    }

    ui::blank();
    ui::title(t(
        "Ho dimenticato il tuo wallet",
        "I have forgotten your wallet",
    ));
    ui::note(t(
        "Ho cancellato il file con cui guardavo la catena. Non conteneva le tue parole e non \
         apriva niente, ma diceva tutti i tuoi indirizzi e quanto c'è su ognuno. Con quel file \
         nessuno può spendere, ma chi lo trova può seguire i tuoi movimenti da lì in avanti.",
        "I have deleted the file I used to look at the chain. It did not hold your words and it \
         opened nothing, but it listed every address of yours and how much sits on each. \
         Nobody can spend with that file, but whoever finds it can follow your payments from \
         then on.",
    ));
    ui::blank();
    ui::note(t(
        "Le ricariche che hai preparato non le ho toccate: si cancellano solo quando lo dici \
         tu, con «Chiudi e cancella».",
        "I have not touched the refills you prepared: they are deleted only when you say so, \
         with \"Close and delete\".",
    ));
    ui::blank();
    ui::warning(t(
        "La prossima volta ti chiederò le parole all'inizio, prima di staccare la rete, perché \
         senza quel file non so dove cercare le tue monete. Vuol dire scriverle con la rete \
         attaccata, due o tre volte l'anno.",
        "Next time I will ask for your words at the start, before unplugging the network, \
         because without that file I do not know where to look for your coins. That means \
         typing them with the network connected, two or three times a year.",
    ));
    ui::blank();
    ui::note(t(
        "Per rimettere monete qui dentro non serve niente di tutto questo: bastano gli \
         indirizzi che ti ho appena dato. Tienili.",
        "To put coins back in here you need none of this: the addresses I just gave you are \
         enough. Keep them.",
    ));
    Ok(())
}

pub(crate) fn shellexpand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => std::path::Path::new(&home).join(rest).display().to_string(),
            None => path.to_string(),
        },
        None => path.to_string(),
    }
}

pub(crate) fn copy_into(file: &std::path::Path, dir: &std::path::Path) -> Result<()> {
    let Some(name) = file.file_name() else {
        return Ok(());
    };
    std::fs::copy(file, dir.join(name)).map_err(Error::io(file))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};
    use std::collections::BTreeSet;

    /// The txids of a signed file, as if it had just been taken away.
    fn copied(path: &std::path::Path) -> BTreeSet<String> {
        plebline_core::verify::load(path)
            .expect("gruppo leggibile")
            .txs
            .into_iter()
            .map(|tx| tx.txid)
            .collect()
    }

    /// The latest group was taken away this session, an archive from an
    /// earlier one was not: closing warns and asks, and a "no" keeps it.
    #[test]
    fn an_archive_not_taken_away_is_not_deleted_without_asking() {
        in_italian();
        let temp = Temp::new("chiudi-archivio");
        let ceremony = temp.ceremony();
        write_wallet(&ceremony);
        write_batch(&ceremony, &[0]);
        write_group(&ceremony, "batch-1.json", &[1]);

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        let mut ask = Scripted::new([Reply::No]);
        let closed = step_close(&ceremony, &copied(&ceremony.path(project::BATCH)), &mut ask);
        plebline::out::take();

        closed.expect("chiusura");
        assert_eq!(
            ask.prompts(),
            ["Cancello lo stesso la cartella di lavoro?"],
            "prima di cancellare si chiede"
        );
        assert!(
            ceremony.path("batch-1.json").is_file(),
            "l'archivio non portato via resta"
        );
        assert!(
            notebook
                .text()
                .contains("non hai portato via: batch-1.json"),
            "l'avviso dice quale file: {}",
            notebook.text()
        );
    }

    /// A group copied away, then a second one signed and not copied: the
    /// first is now an archive, and the warning names the second.
    #[test]
    fn after_a_second_group_the_warning_names_the_one_not_taken_away() {
        in_italian();
        let temp = Temp::new("chiudi-secondo-gruppo");
        let ceremony = temp.ceremony();
        write_wallet(&ceremony);
        write_group(&ceremony, "batch-1.json", &[1]);
        write_batch(&ceremony, &[2]);
        let first = copied(&ceremony.path("batch-1.json"));

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        let mut ask = Scripted::new([Reply::No]);
        let closed = step_close(&ceremony, &first, &mut ask);
        plebline::out::take();

        closed.expect("chiusura");
        let screen = notebook.text();
        assert!(
            screen.contains("non hai portato via: batch.json"),
            "l'avviso nomina il gruppo non copiato: {screen}"
        );
        assert!(
            !screen.contains("batch-1.json"),
            "quello è stato copiato: {screen}"
        );
        assert!(
            ceremony.has(project::BATCH),
            "detto no, il gruppo non copiato resta"
        );
        assert_eq!(ask.prompts(), ["Cancello lo stesso la cartella di lavoro?"]);
    }

    /// `batch.json` gone (its last refill replaced), an archive left: asked
    /// too, and a "yes" deletes.
    #[test]
    fn an_archive_alone_goes_only_after_a_yes() {
        in_italian();
        let temp = Temp::new("chiudi-solo-archivio");
        let ceremony = temp.ceremony();
        write_wallet(&ceremony);
        write_group(&ceremony, "batch-1.json", &[1]);

        let mut ask = Scripted::new([Reply::Yes]);
        step_close(&ceremony, &BTreeSet::new(), &mut ask).expect("chiusura");

        assert_eq!(ask.prompts(), ["Cancello lo stesso la cartella di lavoro?"]);
        assert!(
            project::signed_files(&ceremony).is_empty(),
            "con il sì, via"
        );
        assert!(!ceremony.has(project::WALLET));
    }

    /// Refills set aside at an opening are named and asked about: a "no"
    /// keeps them, a "yes" takes them and the folder.
    #[test]
    fn refills_set_aside_go_only_after_a_yes() {
        in_italian();
        let temp = Temp::new("chiudi-precedenti");
        let ceremony = Project {
            name: "prova".to_string(),
            dir: temp.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&ceremony.dir).expect("cartella creabile");
        write_wallet(&ceremony);
        write_batch(&ceremony, &[0]);
        project::set_aside(&ceremony).expect("spostati");

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        let mut ask = Scripted::new([Reply::No]);
        let closed = step_close(&ceremony, &BTreeSet::new(), &mut ask);
        plebline::out::take();

        closed.expect("chiusura");
        assert_eq!(ask.prompts(), ["Cancello lo stesso la cartella di lavoro?"]);
        assert!(
            notebook.text().contains("precedenti/1/batch.json"),
            "l'avviso dice quale file: {}",
            notebook.text()
        );
        assert_eq!(
            project::set_aside_files(&ceremony).len(),
            1,
            "detto no, resta"
        );

        let mut ask = Scripted::new([Reply::Yes]);
        step_close(&ceremony, &BTreeSet::new(), &mut ask).expect("chiusura");
        assert!(
            project::set_aside_files(&ceremony).is_empty(),
            "con il sì, via"
        );
        assert!(!ceremony.dir.exists(), "e la cartella con loro");
    }
}
