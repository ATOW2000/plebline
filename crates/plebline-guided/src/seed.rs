//! The words, and the public half of the wallet that comes out of them.

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};
use zeroize::Zeroizing;

use plebline::ask::{Ask, Question};
use plebline::project::{self, Project};
use plebline::{ui, wizard};

use crate::{Open, NETWORK};

pub(crate) fn step_seed(ask: &mut dyn Ask) -> Result<Open> {
    ui::title(t("Le tue parole", "Your words"));
    ui::note(t(
        "Sono le parole scritte sul tuo foglio. Restano solo in memoria: non finiscono su \
         disco, e lo schermo su cui le scrivi non ne conserva traccia.",
        "These are the words written on your sheet. They stay in memory only: they never touch \
         the disk, and the screen you type them on keeps no trace.",
    ));
    // Said while the network is still on: the words are needed before the
    // scan, since no `wallet.json` on disk is trusted.
    ui::note(t(
        "In questo momento la rete è ancora attaccata: mi servono adesso perché la parte \
         pubblica del portafoglio la rifaccio da queste, e non da un file che ho trovato sul \
         disco. Restano in memoria fino alla firma, e fra poco ti dico di staccare.",
        "The network is still connected right now: I need them at this point because I rebuild \
         the wallet's public half from them, not from a file I found on the disk. They stay in \
         memory until the signing, and in a moment I will tell you to unplug.",
    ));

    // The user reads the count off their paper; it cannot be deduced.
    // Answered by index.
    const COUNTS: [usize; 5] = [12, 24, 15, 18, 21];

    let labels: Vec<String> = COUNTS.iter().map(usize::to_string).collect();
    let shown: Vec<&str> = labels.iter().map(String::as_str).collect();

    let index = ask.select(
        Question::new(t(
            "Quante parole ci sono sul tuo foglio?",
            "How many words are on your sheet?",
        )),
        &shown,
        Some(5),
    )?;
    let count = COUNTS[index];

    // Checksum checked right away, so a wrong word means retyping, not a
    // failed ceremony.
    let seed = loop {
        let entered = ask.seed(count)?;

        match entered.check() {
            Ok(()) => break entered,
            Err(error) => {
                // Drop the words before showing the message.
                drop(entered);
                ui::blank();
                ui::failure(&error.to_string());
                ui::note(t(
                    "Il numero di controllo non torna: una delle parole non è quella giusta, o \
                     sono in un ordine diverso. Il controllo vale sull'insieme, quindi non è \
                     possibile dire quale sia.",
                    "The check number does not add up: one of the words is not the right one, or \
                     they are in a different order. The check covers the whole set, so it cannot \
                     say which one.",
                ));
                // Not writing them again is a way back, like cancelling.
                if !ask.confirm(
                    Question::new(t("Le riscriviamo?", "Shall we write them again?")),
                    true,
                )? {
                    return Err(Error::Cancelled);
                }
            }
        }
    };

    let typed = Zeroizing::new(ask.password(
        Question::new(t(
            "Passphrase BIP39 (Invio se non ne usi una):",
            "BIP39 passphrase (press Enter if you don't use one):",
        ))
        .help(t(
            "è la parola in più che alcuni aggiungono al seed. Se non sai cosa sia, non ne hai una",
            "it is the extra word some people add to the seed. If you do not know what it is, \
             you do not have one",
        )),
    )?);

    // An empty one is dropped here, and zeroed with it.
    let passphrase = if typed.trim().is_empty() {
        None
    } else {
        Some(typed)
    };

    Ok(Open { seed, passphrase })
}

/// Writes `wallet.json`: the public, watch-only half.
pub(crate) fn step_public_half(ceremony: &Project, open: &Open, ask: &mut dyn Ask) -> Result<()> {
    wizard::descriptors(
        NETWORK,
        wizard::DescriptorsInput {
            out: ceremony.path(project::WALLET),
            wallet: open.access(),
        },
        ask,
    )?;
    Ok(())
}

/// `descriptor_checksum` of the existing `wallet.json`: identifies a wallet,
/// reveals nothing. Unreadable, or not matching its own descriptors, means
/// nothing to compare.
pub(crate) fn wallet_fingerprint(ceremony: &Project) -> Option<String> {
    let path = ceremony.path(project::WALLET);
    let file = plebline_core::watch::load(&path, NETWORK).ok()?;
    Some(file.descriptor_checksum)
}

/// The master key fingerprint in `wallet.json`'s descriptor, `[73c5da0a/…]`:
/// what Sparrow and other wallets show, unlike the descriptor checksum. With
/// it on screen, words typed for the wrong wallet are seen at once.
pub(crate) fn master_fingerprint(ceremony: &Project) -> Option<String> {
    let file = plebline_core::watch::load(&ceremony.path(project::WALLET), NETWORK).ok()?;
    let start = file.external.find('[')? + 1;
    let print = file.external.get(start..start + 8)?;
    print
        .chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| print.to_string())
}

/// Warns when the directory held another wallet: not an error, but later
/// verification failures would otherwise be unexplained.
pub(crate) fn warn_if_other_wallet(ceremony: &Project, previous: Option<&str>) {
    let Some(previous) = previous else { return };
    let Some(current) = wallet_fingerprint(ceremony) else {
        return;
    };
    if previous == current {
        return;
    }

    ui::blank();
    ui::warning(&tf!(
        {
            "In questa cartella c'era un altro portafoglio ({previous}); le parole che hai appena \
             scritto ne aprono un altro ({current})."
        },
        {
            "This folder held a different wallet ({previous}); the words you have just entered open \
             another one ({current})."
        }
    ));
    ui::note(t(
        "Vado avanti con quello che hai scritto tu: è l'unica cosa che decide di chi è questa \
         cerimonia. Ma i file rimasti qui dentro (monete viste, ricariche già firmate) sono di \
         quell'altro, e non valgono per questo.",
        "I go on with what you have entered: it is the only thing that decides whose ceremony \
         this is. But the files left in here (coins seen, refills already signed) belong to \
         that other one, and do not apply to this one.",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::Kind;
    use plebline::ask::{Reply, Scripted};
    use plebline_core::bitcoin::Network;

    #[test]
    fn twelve_words_and_no_passphrase() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("12"), Reply::seed(SEED12), Reply::text("")]);
        let open = step_seed(&mut ask).expect("wallet aperto");
        assert!(open.passphrase.is_none());
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn a_typed_passphrase_is_kept() {
        in_italian();
        let mut ask = Scripted::new([
            Reply::like("12"),
            Reply::seed(SEED12),
            Reply::text("la parola in più"),
        ]);
        let open = step_seed(&mut ask).expect("wallet aperto");
        assert_eq!(
            open.passphrase.as_deref().map(String::as_str),
            Some("la parola in più")
        );
    }

    /// The chosen word count reaches the typing (12 of 24 would open another
    /// wallet silently).
    #[test]
    fn the_chosen_count_reaches_the_word_entry() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("24"), Reply::seed(SEED24), Reply::text("")]);
        assert!(step_seed(&mut ask).is_ok());
    }

    #[test]
    #[should_panic(expected = "il flusso ne ha chieste 24")]
    fn choosing_twenty_four_does_not_become_twelve() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("24"), Reply::seed(SEED12)]);
        let _ = step_seed(&mut ask);
    }

    #[test]
    fn the_words_are_asked_before_the_passphrase() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("12"), Reply::seed(SEED12), Reply::text("")]);
        let _ = step_seed(&mut ask).expect("wallet aperto");
        let kinds: Vec<Kind> = ask.asked().iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [Kind::Select, Kind::Seed, Kind::Password]);
    }

    #[test]
    fn without_the_words_there_is_no_going_on() {
        in_italian();
        // Esc during assisted entry.
        let mut ask = Scripted::new([Reply::like("12"), Reply::Cancelled]);
        // `Open` has no `Debug` on purpose: it holds the passphrase.
        let error = match step_seed(&mut ask) {
            Ok(_) => panic!("senza le parole non si deve aprire nessun wallet"),
            Err(error) => error,
        };
        // A way back, not an alarm on the words.
        assert!(matches!(error, Error::Cancelled), "{error}");
    }

    #[test]
    fn the_public_half_lands_on_disk_and_holds_no_seed() {
        in_italian();
        let temp = Temp::new("pubblica");
        let ceremony = temp.ceremony();

        let mut ask = Scripted::new([Reply::like("12"), Reply::seed(SEED12), Reply::text("")]);
        let open = step_seed(&mut ask).expect("wallet aperto");
        step_public_half(&ceremony, &open, &mut ask).expect("scritto");

        assert!(ceremony.has(project::WALLET));
        let written = std::fs::read_to_string(ceremony.path(project::WALLET)).expect("leggibile");
        // xpub/xprv on mainnet, tpub/tprv elsewhere.
        let (public, private) = match NETWORK {
            Network::Bitcoin => ("xpub", "xprv"),
            _ => ("tpub", "tprv"),
        };
        assert!(written.contains(public), "servono i descriptor: {written}");
        // Never the words, never a spending key.
        assert!(
            !written.contains(SEED12),
            "le parole del seed sono nel file"
        );
        assert!(
            !written.contains(private),
            "questo è un descriptor che può spendere: {written}"
        );
    }

    /// A `wallet.json` from another seed is rewritten from the typed words,
    /// and the difference is announced.
    #[test]
    fn a_wallet_already_in_the_folder_is_rewritten_and_the_difference_is_told() {
        in_italian();
        let temp = Temp::new("non-mio");
        let ceremony = temp.ceremony();

        let other = test_wallet(SEED24);
        let file = plebline_core::watch::WatchFile {
            version: plebline_core::watch::WATCH_SCHEMA_VERSION,
            network: NETWORK.to_string(),
            external: plebline_core::wallet::public_descriptors(&other).0,
            internal: plebline_core::wallet::public_descriptors(&other).1,
            descriptor_checksum: fingerprint(&other),
            created_by: "una prova di agosto".into(),
        };
        std::fs::write(
            ceremony.path(project::WALLET),
            serde_json::to_string_pretty(&file).expect("serializzabile"),
        )
        .expect("scrivibile");

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut ask = Scripted::new([Reply::like("12"), Reply::seed(SEED12), Reply::text("")]);
        let open = step_seed(&mut ask).expect("parole");
        let previous = wallet_fingerprint(&ceremony);
        step_public_half(&ceremony, &open, &mut ask).expect("riscritto");
        warn_if_other_wallet(&ceremony, previous.as_deref());
        plebline::out::take();

        assert_eq!(
            previous.as_deref(),
            Some(fingerprint(&other).as_str()),
            "in partenza in cartella c'era l'altro"
        );
        assert_eq!(
            wallet_fingerprint(&ceremony).as_deref(),
            Some(fingerprint(&test_wallet(SEED12)).as_str()),
            "il file deve dire il portafoglio delle parole appena scritte"
        );

        let said = notebook.text();
        assert!(
            said.contains("c'era un altro portafoglio"),
            "la differenza va detta, non lasciata scoprire alla verifica: {said}"
        );
    }

    #[test]
    fn if_the_wallet_is_the_same_no_warning_is_given() {
        in_italian();
        let temp = Temp::new("stesso");
        let ceremony = temp.ceremony();
        write_wallet(&ceremony);

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut ask = Scripted::new([Reply::like("12"), Reply::seed(SEED12), Reply::text("")]);
        let open = step_seed(&mut ask).expect("parole");
        let previous = wallet_fingerprint(&ceremony);
        step_public_half(&ceremony, &open, &mut ask).expect("riscritto");
        warn_if_other_wallet(&ceremony, previous.as_deref());
        plebline::out::take();

        assert!(
            !notebook.text().contains("c'era un altro portafoglio"),
            "stesso portafoglio, nessun avviso"
        );
    }

    /// A wrong word means retyping, not ending the ceremony.
    #[test]
    fn words_that_do_not_check_out_get_retyped() {
        in_italian();
        let mut ask = Scripted::new([
            Reply::like("12"),
            Reply::seed(SEED12_BROKEN),
            Reply::Yes,
            Reply::seed(SEED12),
            Reply::text(""),
        ]);

        step_seed(&mut ask).expect("il secondo tentativo torna");
        assert_eq!(ask.remaining(), 0);
    }

    /// No wallet is opened on words that fail the checksum.
    #[test]
    fn refusing_to_retype_goes_back() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("12"), Reply::seed(SEED12_BROKEN), Reply::No]);

        let error = match step_seed(&mut ask) {
            Ok(_) => panic!("un seed che non torna non deve aprire niente"),
            Err(error) => error,
        };
        assert!(matches!(error, Error::Cancelled), "{error}");
    }

    /// The name a `wallet.json` declares is the one typed words are checked
    /// against: one that is not its descriptors' own names nothing.
    #[test]
    fn a_wallet_json_that_lies_about_its_name_names_nothing() {
        let temp = Temp::new("nome-falso");
        let ceremony = temp.ceremony();
        let mine = write_wallet(&ceremony);
        assert_eq!(wallet_fingerprint(&ceremony), Some(fingerprint(&mine)));
        assert!(master_fingerprint(&ceremony).is_some());

        let other = test_wallet(SEED24);
        let (external, internal) = plebline_core::wallet::public_descriptors(&other);
        let file = plebline_core::watch::WatchFile {
            version: plebline_core::watch::WATCH_SCHEMA_VERSION,
            network: NETWORK.to_string(),
            external,
            internal,
            // Another wallet's descriptors under this wallet's name.
            descriptor_checksum: fingerprint(&mine),
            created_by: "test".into(),
        };
        std::fs::write(
            ceremony.path(project::WALLET),
            serde_json::to_string_pretty(&file).expect("serializzabile"),
        )
        .expect("scrivibile");

        assert_eq!(wallet_fingerprint(&ceremony), None);
        assert_eq!(master_fingerprint(&ceremony), None);

        std::fs::write(ceremony.path(project::WALLET), b"{\"version\": 1,").expect("scrivibile");
        assert_eq!(wallet_fingerprint(&ceremony), None);
    }
}
