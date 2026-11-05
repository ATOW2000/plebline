//! The recipient addresses, from the phone, through OnionShare. Asked after
//! the coins are seen, network still up. The words are in memory by then:
//! what arrives from outside is a small text file written by OnionShare, a
//! separate process, read only through `address::parse_list`.

use std::path::{Path, PathBuf};

use plebline::ask::{Ask, Question};
use plebline::{destination, inbox, qr, ui};
use plebline_core::address::{self, Destination};
use plebline_core::lang::t;
use plebline_core::{tf, Result};

use crate::NETWORK;

/// The list sent from the phone, in payment order; empty if the user will
/// type or paste them later, one at a time. `roots`: where OnionShare saves
/// (see [`inbox::default_roots`]).
pub(crate) fn step_phone(roots: &[PathBuf], ask: &mut dyn Ask) -> Result<Vec<Destination>> {
    ui::title(t("Gli indirizzi da ricaricare", "The addresses to refill"));
    ui::note(t(
        "Gli indirizzi a cui mandare le ricariche li hai sul telefono. Il modo più sicuro per \
         portarli qui è OnionShare: un collegamento diretto dentro Tor, senza chiavette e senza \
         nessuno in mezzo. Si fa adesso, perché la rete c'è ancora: fra poco la stacchi.",
        "The addresses to send the refills to are on your phone. The safest way to bring them \
         here is OnionShare: a direct link inside Tor, no USB stick and nobody in between. It \
         happens now, while the network is still up: you unplug it shortly.",
    ));
    ui::note(where_onionshare_is());

    let from_phone = t(
        "Dal telefono, con OnionShare",
        "From the phone, with OnionShare",
    );
    let later = t(
        "Li incollo o li scrivo dopo, uno alla volta",
        "I will paste or type them later, one at a time",
    );
    let how = ask.select(
        Question::new(t(
            "Come mi dai gli indirizzi?",
            "How will you give me the addresses?",
        )),
        &[from_phone, later],
        None,
    )?;
    if how != 0 {
        return Ok(Vec::new());
    }
    if roots.is_empty() {
        ui::warning(t(
            "Non so dov'è la tua cartella personale, quindi non trovo quello che riceve \
             OnionShare: gli indirizzi li chiedo dopo, uno alla volta.",
            "I do not know where your home folder is, so I cannot find what OnionShare \
             receives: I will ask for the addresses later, one at a time.",
        ));
        return Ok(Vec::new());
    }

    instructions();

    let look = t("Li ho mandati: guarda", "I sent them: have a look");
    let give_up = t(
        "Lascia stare, li scrivo dopo",
        "Never mind, I will type them later",
    );
    loop {
        ui::blank();
        let answer = ask.select(
            Question::new(t(
                "Hai mandato gli indirizzi?",
                "Have you sent the addresses?",
            )),
            &[look, give_up],
            None,
        )?;
        if answer != 0 {
            return Ok(Vec::new());
        }

        let Some(text) = pick_message(roots, ask)? else {
            continue;
        };
        let list = match address::parse_list(&text, NETWORK) {
            Ok(list) => list,
            Err(e) => {
                ui::warning(&tf!(
                    {
                        "Il messaggio non va bene, e non ne prendo nemmeno una parte: {e}. \
                         Correggilo sul telefono e mandalo di nuovo."
                    },
                    {
                        "The message is not right, and I will not take even part of it: {e}. \
                         Fix it on the phone and send it again."
                    }
                ));
                continue;
            }
        };

        show(&list);
        let right = ask.confirm(
            Question::new(t(
                "Sono gli indirizzi giusti, in quest'ordine?",
                "Are these the right addresses, in this order?",
            )),
            false,
        )?;
        if right {
            ui::note(t(
                "Adesso puoi fermare OnionShare: non serve più.",
                "You can stop OnionShare now: it is no longer needed.",
            ));
            return Ok(list);
        }
        ui::note(t(
            "Allora mandali di nuovo dal telefono: guardo sempre il messaggio più recente.",
            "Then send them again from the phone: I always look at the most recent message.",
        ));
    }
}

/// Tails ships it; elsewhere it must be installed (on a Mac it brings its
/// own Tor, separate from ours).
fn where_onionshare_is() -> &'static str {
    if cfg!(target_os = "macos") {
        t(
            "Sul Mac non c'è già: se non ce l'hai, scaricalo da onionshare.org e installalo, \
             una volta sola.",
            "It is not on a Mac by default: if you do not have it, download it from \
             onionshare.org and install it, once.",
        )
    } else {
        t(
            "Su Tails c'è già, non devi installare niente.",
            "Tails already has it, nothing to install.",
        )
    }
}

fn how_to_open() -> &'static str {
    if cfg!(target_os = "macos") {
        t(
            "1. Apri OnionShare dalla cartella Applicazioni, poi «Ricevi file».",
            "1. Open OnionShare from the Applications folder, then «Receive Files».",
        )
    } else {
        t(
            "1. Apri OnionShare: Applicazioni ▸ Internet ▸ OnionShare, poi «Ricevi file».",
            "1. Open OnionShare: Apps ▸ Internet ▸ OnionShare, then «Receive Files».",
        )
    }
}

/// Orbot plus the usual browser: a browser with its own Tor drops the page
/// on an iPhone as soon as the user switches app to copy the addresses.
fn instructions() {
    ui::blank();
    ui::note(how_to_open());
    ui::note(t(
        "2. Prima di avviare, spunta la casella del servizio pubblico, quella che disattiva la \
         chiave privata, e togli il caricamento dei file: servono solo messaggi di testo.",
        "2. Before starting, tick the public service box, the one that disables the private \
         key, and turn off file uploads: only text messages are needed.",
    ));
    ui::note(t(
        "   La chiave privata qui non serve: l'indirizzo è lungo e casuale, lo vedi solo tu su \
         questo schermo e vive pochi minuti. E quello che arriva lo controlli comunque, \
         indirizzo per indirizzo.",
        "   The private key is not needed here: the address is long and random, only you see \
         it on this screen, and it lives a few minutes. And you check what arrives anyway, \
         address by address.",
    ));
    ui::note(t(
        "3. Avvia la ricezione. Compare un indirizzo lungo che finisce in .onion, con il suo QR.",
        "3. Start receiving. A long address ending in .onion appears, with its QR code.",
    ));
    ui::note(t(
        "4. Sul telefono installa Orbot (dall'App Store o dal Play Store), aprilo e accendilo. \
         Se ti chiede come collegarti, scegli il collegamento diretto, senza bridge.",
        "4. On the phone install Orbot (from the App Store or the Play Store), open it and turn \
         it on. If it asks how to connect, choose the direct connection, no bridge.",
    ));
    ui::warning(t(
        "Usa Orbot, non Tor Browser né Onion Browser: quelli hanno un Tor tutto loro, che \
         sull'iPhone si spegne appena cambi app per andare a copiare gli indirizzi, e la pagina \
         si scollega. Orbot invece resta acceso.",
        "Use Orbot, not Tor Browser or Onion Browser: those have a Tor of their own, which on \
         an iPhone switches off as soon as you change app to go and copy the addresses, and \
         the page disconnects. Orbot stays on.",
    ));
    ui::note(t(
        "5. Con Orbot acceso, inquadra il QR con la fotocamera e apri l'indirizzo nel browser di \
         sempre: Safari, Brave, Chrome. Comincia con http://, senza la s: se il browser lo \
         cambia in https://, la pagina non si apre, e va riscritto com'era.",
        "5. With Orbot on, scan the QR with the camera and open the address in your usual \
         browser: Safari, Brave, Chrome. It starts with http://, no s: if the browser turns it \
         into https://, the page does not open, and it must be typed back as it was.",
    ));
    ui::note(t(
        "6. Nella pagina che si apre incolla gli indirizzi, uno per riga, nell'ordine delle \
         ricariche, e invia.",
        "6. In the page that opens, paste the addresses, one per line, in refill order, and \
         submit.",
    ));
    ui::note(t(
        "Su Android la procedura dovrebbe essere la stessa, con Orbot acceso; non è ancora \
         stata provata.",
        "On Android the steps should be the same, with Orbot on; they have not been tried yet.",
    ));
}

/// The newest message, or the one the user picks when there are several:
/// which list to pay is not decided here.
fn pick_message(roots: &[PathBuf], ask: &mut dyn Ask) -> Result<Option<String>> {
    let messages = inbox::messages(roots);
    let chosen = match messages.len() {
        0 => {
            let folders = roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            ui::warning(&tf!(
                {
                    "Non trovo ancora nessun messaggio (ho guardato in {folders}). Guarda su \
                     OnionShare se dice di averlo ricevuto."
                },
                {
                    "I cannot find any message yet (I looked in {folders}). Check whether \
                     OnionShare says it received it."
                }
            ));
            return Ok(None);
        }
        1 => &messages[0],
        _ => {
            ui::note(t(
                "Ho trovato più di un messaggio: scegli tu quale, il più recente è il primo.",
                "I found more than one message: you choose which, the most recent comes first.",
            ));
            let labels: Vec<String> = messages.iter().map(|m| label(&m.path)).collect();
            let options: Vec<&str> = labels.iter().map(String::as_str).collect();
            let index = ask.select(
                Question::new(t("Quale messaggio?", "Which message?")),
                &options,
                None,
            )?;
            &messages[index]
        }
    };
    match &chosen.text {
        Ok(text) => Ok(Some(text.clone())),
        Err(why) => {
            ui::warning(&tf!(
                { "Il messaggio {} non lo leggo: {why}", chosen.path.display() },
                { "I will not read message {}: {why}", chosen.path.display() }
            ));
            Ok(None)
        }
    }
}

/// `…/2026-09-30/113005123456-message.txt` → `2026-09-30 11:30:05`.
fn label(path: &Path) -> String {
    let day = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("?");
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let time = match name.get(0..6) {
        Some(d) if d.chars().all(|c| c.is_ascii_digit()) => {
            format!("{}:{}:{}", &d[0..2], &d[2..4], &d[4..6])
        }
        _ => name.to_string(),
    };
    tf!({ "ricevuto il {day} alle {time}" }, {
        "received on {day} at {time}"
    })
}

/// Each address in groups of four and as a QR: scanned back with the phone,
/// it must match the one sent.
fn show(list: &[Destination]) {
    ui::blank();
    ui::success(&tf!(
        { "Mi sono arrivati {} indirizzi.", list.len() },
        { "{} addresses arrived.", list.len() }
    ));
    ui::note(t(
        "Controllali: inquadra ogni QR col telefono e confronta con l'indirizzo di partenza, \
         almeno l'inizio e la fine. Se un indirizzo è sbagliato non compare nessun errore, e il \
         pagamento va a qualcun altro.",
        "Check them: scan each QR with the phone and compare it with the original address, at \
         least the beginning and the end. If an address is wrong no error appears, and the \
         payment goes to someone else.",
    ));
    for (index, destination) in list.iter().enumerate() {
        let text = destination.address.to_string();
        ui::blank();
        let shown = plebline_core::batch::number(index);
        ui::info(&tf!({ "Ricarica #{shown}" }, { "Refill #{shown}" }));
        ui::note(&destination::in_groups(&text));
        if let Some(sat) = destination.amount_sat {
            let amount = ui::sats(sat);
            ui::note(&tf!(
                { "importo proposto: {amount} (te lo richiedo dopo)" },
                { "proposed amount: {amount} (I will ask again later)" }
            ));
        }
        if let Ok(drawn) = qr::ascii(&text) {
            ui::qr(&drawn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};

    fn address(index: u32) -> String {
        recipient(index)
    }

    fn inbox_with(temp: &Temp, messages: &[(&str, String)]) -> std::path::PathBuf {
        let root = temp.path.join("OnionShare");
        for (name, text) in messages {
            let path = root.join("2026-09-30").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        root
    }

    #[test]
    fn later_asks_nothing_else_and_reads_nothing() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("Li incollo")]);
        let list = step_phone(&[PathBuf::from("/non/esiste")], &mut ask).unwrap();
        assert!(list.is_empty());
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn the_list_from_the_phone_comes_back_in_order_once_confirmed() {
        in_italian();
        let temp = Temp::new("phone-ok");
        let text = format!("{}\n{}\n", address(0), address(1));
        let root = inbox_with(&temp, &[("101500000000-message.txt", text)]);

        let mut ask = Scripted::new([
            Reply::like("Dal telefono"),
            Reply::like("Li ho mandati"),
            Reply::Yes,
        ]);
        let list = step_phone(std::slice::from_ref(&root), &mut ask).unwrap();
        let got: Vec<String> = list.iter().map(|d| d.address.to_string()).collect();
        assert_eq!(got, [address(0), address(1)]);
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn a_bad_message_is_refused_whole() {
        in_italian();
        let temp = Temp::new("phone-bad");
        let bad = format!("{}\nnon un indirizzo\n", address(0));
        let root = inbox_with(&temp, &[("101500000000-message.txt", bad)]);

        // Refused, so no confirmation is asked: straight back to "sent?".
        let mut ask = Scripted::new([
            Reply::like("Dal telefono"),
            Reply::like("Li ho mandati"),
            Reply::like("Lascia stare"),
        ]);
        assert!(step_phone(std::slice::from_ref(&root), &mut ask)
            .unwrap()
            .is_empty());
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn with_several_messages_the_user_chooses() {
        in_italian();
        let temp = Temp::new("phone-many");
        let root = inbox_with(
            &temp,
            &[
                ("090000000000-message.txt", format!("{}\n", address(0))),
                ("113005000000-message.txt", format!("{}\n", address(1))),
            ],
        );
        let mut ask = Scripted::new([
            Reply::like("Dal telefono"),
            Reply::like("Li ho mandati"),
            Reply::like("alle 09:00:00"),
            Reply::Yes,
        ]);
        let list = step_phone(std::slice::from_ref(&root), &mut ask).unwrap();
        assert_eq!(list[0].address.to_string(), address(0));
    }

    #[test]
    fn a_no_at_the_check_does_not_take_the_list() {
        in_italian();
        let temp = Temp::new("phone-no");
        let root = inbox_with(
            &temp,
            &[("101500000000-message.txt", format!("{}\n", address(0)))],
        );
        let mut ask = Scripted::new([
            Reply::like("Dal telefono"),
            Reply::like("Li ho mandati"),
            Reply::No,
            Reply::like("Lascia stare"),
        ]);
        assert!(step_phone(std::slice::from_ref(&root), &mut ask)
            .unwrap()
            .is_empty());
    }

    /// On a Mac the words about Tails would be wrong: where it is and how
    /// to open it follow the system.
    #[test]
    fn the_instructions_follow_the_system() {
        in_italian();
        if cfg!(target_os = "macos") {
            assert!(where_onionshare_is().contains("onionshare.org"));
            assert!(how_to_open().contains("cartella Applicazioni"));
        } else {
            assert!(where_onionshare_is().contains("Tails"));
            assert!(how_to_open().contains("Internet"));
        }
    }

    /// Orbot, not a browser with its own Tor; no private key; `http://`, not
    /// `https://`.
    #[test]
    fn the_phone_instructions_say_what_the_trial_on_tails_taught() {
        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        instructions();
        plebline::out::to(Box::new(plebline::out::Terminal));

        let screen = notebook.text();
        for needed in [
            "Orbot",
            "non Tor Browser",
            "servizio pubblico",
            "http://",
            "senza bridge",
        ] {
            assert!(screen.contains(needed), "manca «{needed}»: {screen}");
        }
        assert!(
            !screen.contains("inquadra anche quella"),
            "la chiave privata non si inquadra più: {screen}"
        );
        let warned = notebook
            .lines()
            .into_iter()
            .any(|line| line.kind == plebline::out::Kind::Warning && line.text.contains("Orbot"));
        assert!(
            warned,
            "il consiglio su Orbot è un avvertimento, non una nota"
        );
    }

    /// The message is found where Tails' OnionShare saves it.
    #[test]
    fn a_message_saved_in_downloads_is_found() {
        in_italian();
        let temp = Temp::new("phone-downloads");
        let downloads = temp.path.join("Downloads");
        let path = downloads
            .join("2026-10-07")
            .join("151000000000-message.txt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n", address(0))).unwrap();

        let mut ask = Scripted::new([
            Reply::like("Dal telefono"),
            Reply::like("Li ho mandati"),
            Reply::Yes,
        ]);
        let roots = [downloads, temp.path.join("OnionShare")];
        let list = step_phone(&roots, &mut ask).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].address.to_string(), address(0));
    }

    #[test]
    fn the_addresses_to_check_are_counted_from_one() {
        in_italian();
        let temp = Temp::new("phone-numbers");
        let path = temp
            .path
            .join("2026-10-08")
            .join("150000000000-message.txt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n{}\n", address(0), address(1))).unwrap();

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        let mut ask = Scripted::new([
            Reply::like("Dal telefono"),
            Reply::like("Li ho mandati"),
            Reply::Yes,
        ]);
        step_phone(std::slice::from_ref(&temp.path), &mut ask).unwrap();
        plebline::out::to(Box::new(plebline::out::Terminal));

        let screen = notebook.text();
        assert!(screen.contains("Ricarica #1"), "{screen}");
        assert!(screen.contains("Ricarica #2"), "{screen}");
        assert!(!screen.contains("Ricarica #0"), "{screen}");
    }

    /// Bytes that are not text, or a line that only looks like an address,
    /// give nothing: no panic, no partial list.
    #[test]
    fn a_hostile_message_gives_nothing() {
        in_italian();
        let temp = Temp::new("phone-hostile");
        let day = temp.path.join("OnionShare").join("2026-10-12");
        std::fs::create_dir_all(&day).unwrap();
        let mixed = {
            let a = address(1);
            format!("{}{}", &a[..5], a[5..].to_ascii_uppercase())
        };
        let messages: [(&str, Vec<u8>); 3] = [
            ("090000000000-message.txt", b"\xff\xfe\x00tb1q".to_vec()),
            (
                "100000000000-message.txt",
                format!("{}\n{mixed}\n", address(0)).into_bytes(),
            ),
            (
                "110000000000-message.txt",
                format!("{}\nbitcoin:{}?amount=21000001\n", address(0), address(1)).into_bytes(),
            ),
        ];
        for (name, bytes) in &messages {
            std::fs::write(day.join(name), bytes).unwrap();
        }

        let mut answers = vec![Reply::like("Dal telefono")];
        for when in ["09:00:00", "10:00:00", "11:00:00"] {
            answers.push(Reply::like("Li ho mandati"));
            answers.push(Reply::like(when));
        }
        answers.push(Reply::like("Lascia stare"));
        let mut ask = Scripted::new(answers);
        let list = step_phone(&[temp.path.join("OnionShare")], &mut ask).unwrap();
        assert!(list.is_empty());
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn the_label_shows_day_and_time() {
        let path = Path::new("/x/2026-09-30/113005123456-message.txt");
        in_italian();
        assert_eq!(label(path), "ricevuto il 2026-09-30 alle 11:30:05");
    }
}
