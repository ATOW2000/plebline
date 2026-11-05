//! The first screens: the language, what the program does, and off we go.

use plebline_core::lang::{self, t, Lang};
use plebline_core::Result;

use plebline::ask::{Ask, Question};
use plebline::ui;

pub(crate) fn intro() {
    ui::title(t(
        "Prepariamo le tue ricariche, tutte insieme",
        "Let's prepare your refills, all at once",
    ));
    ui::note(t(
        "Apri il tuo cold storage una volta sola. Ne esci con le ricariche dei prossimi mesi \
         già firmate: le mandi in rete quando vuoi, una alla volta, senza rimettere mai più \
         le tue parole su un computer.",
        "Open your cold storage once. You come out with the coming months' refills already \
         signed: send them whenever you like, one at a time, without ever putting your words \
         on a computer again.",
    ));
    ui::blank();
    ui::note(t(
        "I passi sono sempre gli stessi e li faccio io, in ordine. Ti chiedo solo quello che \
         non posso sapere da solo.",
        "The steps are always the same and I go through them in order. I only ask you what I \
         cannot know by myself.",
    ));
}

/// Asked in both languages: none is chosen yet.
pub(crate) fn step_lang(ask: &mut dyn Ask) -> Result<()> {
    let choice = ask.select(
        Question::new("Lingua / Language"),
        &["Italiano", "English"],
        None,
    )?;
    lang::set(if choice == 0 { Lang::It } else { Lang::En });
    Ok(())
}

/// What PlebLine is, offered (default yes) rather than imposed on repeat
/// users.
pub(crate) fn step_what_it_is(ask: &mut dyn Ask) -> Result<()> {
    ui::blank();
    let wants = ask.confirm(
        Question::new(t(
            "Vuoi due parole su cos'è PlebLine?",
            "Want a few words on what PlebLine is?",
        ))
        .help(t(
            "Meno di un minuto. Se lo sai già, rispondi no e si parte.",
            "Under a minute. If you already know, answer no and we start.",
        )),
        true,
    )?;
    if !wants {
        return Ok(());
    }

    ui::title(t("Cos'è PlebLine", "What PlebLine is"));

    // The site QR goes first, after a line saying what it is for. Kept small
    // so it scans from a screen.
    ui::note(t(
        "Per saperne di più visita il nostro sito: inquadra il QR qui sotto.",
        "To find out more visit our site: scan the QR code below.",
    ));
    match plebline::qr::ascii(SITE) {
        Ok(drawn) => ui::qr(&drawn),
        // Failing to draw this QR does not stop the ceremony.
        Err(reason) => ui::warning(&reason),
    }
    // True until `SITE` holds a real address.
    ui::note(t(
        "Il sito sta arrivando: per ora il codice porta a un segnaposto.",
        "The site is on its way: for now the code leads to a placeholder.",
    ));
    ui::blank();

    ui::note(t(
        "Le tue monete non stanno in un posto preciso: sono scritte in rete, su un registro che \
         tengono in tanti. Quello che hai tu è l'accesso, cioè le parole del tuo wallet. Chi ha \
         quelle parole può spostare le monete da qualunque computer.",
        "Your coins are not kept in any one place: they are recorded on the network, on a \
         ledger many people keep. What you hold is the access, meaning your wallet words. \
         Whoever has those words can move the coins from any computer.",
    ));
    ui::blank();
    ui::note(t(
        "Per questo di solito si tengono due wallet: uno sul telefono, con quello che spendi \
         tutti i giorni, e uno con il resto, lontano da qualsiasi computer connesso. Ogni volta \
         che scrivi su una tastiera le parole di quest'ultimo, per un momento sono esposte.",
        "That is why people usually keep two wallets: one on the phone with what they spend \
         every day, and one with the rest, away from any connected computer. Every time you \
         type the second one's words on a keyboard, they are exposed for a moment.",
    ));
    ui::blank();
    ui::note(t(
        "Con PlebLine le scrivi una volta sola per le ricariche dei prossimi mesi.",
        "With PlebLine you type them once for the refills of the coming months.",
    ));

    // Headings use `info` rather than `title`, because in the GUI a title
    // scrolls to the top and two in a row would hide the first.
    ui::blank();
    ui::info(t("Cosa fa", "What it does"));

    // It signs refills of the everyday wallet; the payments themselves are
    // made from the phone.
    ui::success(t(
        "Firma adesso le ricariche del portafoglio che usi tutti i giorni, quello sul \
         telefono. Quante, di che taglia e ogni quanto lo decidi tu: dipende da quanto spendi \
         nell'anno.",
        "Signs, right now, the refills of the wallet you use every day, the one on your \
         phone. How many, how big and how often is up to you: it depends on what you spend \
         over the year.",
    ));
    ui::success(t(
        "Ogni ricarica diventa un file. Le monete si muovono solo quando mandi quel file in \
         rete, e puoi farlo quando vuoi, anche da un altro computer, senza tirare fuori di \
         nuovo le parole.",
        "Each refill becomes a file. The coins move only when you send that file to the \
         network, which you can do whenever you like, even from another computer, without \
         taking the words out again.",
    ));
    ui::success(t(
        "Poi paghi dal telefono come hai sempre fatto, mentre le parole del cold storage \
         restano sul foglio.",
        "Then you pay from your phone as you always have, while the cold storage words stay on \
         paper.",
    ));

    ui::blank();
    ui::info(t("Cosa non fa", "What it does not do"));
    ui::failure(t(
        "Non nasconde cosa c'è nei file: chi li legge vede quali monete sono tue, quanto muovi \
         e verso quali indirizzi.",
        "It does not hide what is in the files: whoever reads them sees which coins are yours, \
         how much you move and to which addresses.",
    ));
    ui::failure(t(
        "Non impedisce che una ricarica parta prima del tempo. Se succede, i soldi vanno \
         comunque all'indirizzo che hai scelto tu, solo prima.",
        "It does not stop a refill from going out earlier than planned. If that happens, the \
         money still goes to the address you chose, only sooner.",
    ));
    ui::failure(t(
        "Non ti fa cambiare idea dopo: chi riceve e quanto si decidono adesso, e cambiarli \
         vuol dire rifare la cerimonia.",
        "It does not let you change your mind afterwards: who receives and how much are \
         decided now, and changing them means doing the ceremony over.",
    ));

    ui::blank();

    // Stated positively, so it can be checked: the signature covers recipient
    // and amount.
    ui::alert(t(
        "Un file firmato non è una chiave. Chi te lo ruba può fare una cosa sola: mandarlo in \
         rete, cioè far arrivare quelle monete dove avevi deciso tu. Chi riceve e quanto sono \
         dentro la firma: cambiarli la rompe, e la rete rifiuta tutto.",
        "A signed file is not a key. Whoever steals it can do exactly one thing: send it to \
         the network, that is, make those coins arrive where you decided they would. Who \
         receives and how much are inside the signature: change them and it breaks, and the \
         network refuses the lot.",
    ));

    ui::blank();
    ui::note(t(
        "Quello che ti chiederò, in ordine: le tue parole, quante ricariche preparare, a chi e \
         quanto. Il resto lo faccio io, e ti dico sempre cosa sta succedendo.",
        "What I will ask you, in order: your words, how many refills to prepare, to whom and \
         how much. The rest I do myself, and I always tell you what is going on.",
    ));

    step_start(ask)
}

/// QR content. When the site exists, its address goes here and the line
/// under the code changes. Short on purpose: fewer bytes, fewer modules
/// (31 bytes, 29 squares).
pub(crate) const SITE: &str = "PlebLine: sito in arrivo / soon";

/// A single "Let's start" choice between the explanation and the ceremony,
/// so the user moves on when done reading.
pub(crate) fn step_start(ask: &mut dyn Ask) -> Result<()> {
    ui::blank();
    let entry = t("Cominciamo", "Let's start");
    ask.select(
        Question::new(t("Quando vuoi, si parte.", "Whenever you are ready.")),
        &[entry],
        None,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::Kind;
    use plebline::ask::{Reply, Scripted};

    #[test]
    fn explanation_comes_before_the_start_button() {
        in_italian();
        let mut ask = Scripted::new([Reply::Yes, Reply::like("Cominciamo")]);
        step_what_it_is(&mut ask).expect("la spiegazione arriva in fondo");
        let kinds: Vec<Kind> = ask.asked().iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [Kind::Confirm, Kind::Select]);
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn skipping_the_explanation_skips_the_start_button() {
        in_italian();
        let mut ask = Scripted::new([Reply::No]);
        step_what_it_is(&mut ask).expect("si salta");
        assert_eq!(ask.remaining(), 0, "non deve chiedere altro");
    }

    #[test]
    fn the_opening_qr_code_can_be_drawn() {
        assert!(plebline::qr::ascii(SITE).is_ok(), "{SITE}");
    }

    /// 37 columns with the quiet zone; making it larger should be a
    /// deliberate change.
    #[test]
    fn the_opening_qr_code_stays_small() {
        let drawn = plebline::qr::ascii(SITE).expect("il QR si disegna");
        let width = drawn.lines().map(|r| r.chars().count()).max().unwrap_or(0);
        let height = drawn.lines().count();
        eprintln!("QR: {width} colonne, {height} righe");
        assert!(width <= 40, "{width} colonne: il codice è diventato grande");
    }
}
