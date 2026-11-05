//! The loop around the wallet page: reached after the words and the coins,
//! and back to after every operation, until "close and delete". The user
//! picks each operation; no file picks it. The page itself is in `wallet_page`.

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

use plebline::ask::{Ask, Question};
use plebline::chain::Chain;
use plebline::out::Cancel;
use plebline::project::{self, Project};
use plebline::{online, ui};

use crate::ceremony::nothing_found;
use crate::close::step_close;
use crate::export::step_export;
use crate::road::{self, Point};
use crate::seed::{step_seed, wallet_fingerprint};
use crate::send::{pick_transaction, step_send};
use crate::sign::{
    ask_payment_count, check_before_send, do_split, explain_split, step_check, step_sign,
};
use crate::snapshot::{ask_server, step_chain, step_unplug};
use crate::wallet_page::{self, Action};
use crate::{phone, Open, NETWORK};

/// What one operation hands to the next.
pub(crate) struct Session {
    /// The words, while in memory. They are dropped before the network is
    /// plugged back in.
    pub(crate) open: Option<Open>,
    pub(crate) server: Option<String>,
    /// The network as the user last left it.
    pub(crate) online: bool,
    /// At the wallet again, after an operation.
    pub(crate) returned: bool,
    /// The txids of the signed payments taken away (stick or phone) in
    /// this session: a later group archives the copied one under another
    /// name, so the name alone would point at the wrong file.
    pub(crate) copied: std::collections::BTreeSet<String>,
}

/// Every change to the words or the network goes through here, so the step
/// line the window draws always matches the ceremony's state.
impl Session {
    fn remember(&mut self, open: Open) {
        self.open = Some(open);
        road::words(true);
    }

    /// Whether there were words to forget.
    fn forget(&mut self) -> bool {
        let had = self.open.take().is_some();
        road::words(false);
        had
    }

    fn unplugged(&mut self) {
        self.online = false;
        road::unplugged();
    }

    fn plugged_back(&mut self) {
        self.online = true;
        road::plugged_back();
    }
}

/// Step points for new refills, given the current session.
fn refills_road(session: &Session) -> Vec<Point> {
    let mut tail = Vec::new();
    if session.open.is_none() {
        tail.push(Point::Words);
    }
    if session.online {
        tail.push(Point::Addresses);
    }
    tail.push(Point::Count);
    tail.extend(signing_road(session));
    tail.extend([Point::Check, Point::Export, Point::Send]);
    tail
}

fn split_road(session: &Session) -> Vec<Point> {
    let mut tail = Vec::new();
    if session.open.is_none() {
        tail.push(Point::Words);
    }
    tail.push(Point::Parts);
    tail.extend(signing_road(session));
    tail.push(Point::Send);
    tail
}

/// Unplugging (if the network is still on), then signing.
fn signing_road(session: &Session) -> Vec<Point> {
    let mut tail = Vec::new();
    if session.online {
        tail.push(Point::Unplug);
    }
    tail.push(Point::Sign);
    tail
}

pub(crate) fn run(
    ceremony: &Project,
    session: &mut Session,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<()> {
    loop {
        // At the wallet, the points ahead show the usual next operation.
        road::operation(&refills_road(session));
        road::at(Point::Wallet);
        // Nothing before the wallet page: cancelling here stops, files stay.
        road::cancel(Cancel::Stop);
        let action = if ceremony.has(project::UTXOS) {
            wallet_page::step_home(ceremony, session, ask, chain)?
        } else {
            empty(ceremony, session, ask)?
        };
        road::cancel(Cancel::Home);
        let done = match action {
            Action::Refills { free } => refills(ceremony, session, free, ask, chain),
            Action::Split { free } => {
                road::operation(&split_road(session));
                split(ceremony, session, free, None, ask, chain)
            }
            Action::Send => {
                road::operation(&[Point::Send]);
                road::at(Point::Send);
                send_one(ceremony, session, ask, chain)
            }
            Action::Reread => reread(ceremony, session, ask, chain),
            Action::Close => {
                session.forget();
                match step_close(ceremony, &session.copied, ask) {
                    Err(Error::Cancelled) => Err(Error::Cancelled),
                    closed => return closed,
                }
            }
        };
        // A cancel leaves the operation where it got to and comes back here.
        // The session already says where words and network are: only its
        // methods change them, and only after the user has done it.
        match done {
            Ok(()) => {}
            Err(Error::Cancelled) => {
                ui::blank();
                ui::note(t(
                    "Annullato: torno al portafoglio. Quello che è già scritto resta dov'è.",
                    "Cancelled: back to the wallet. What is already written stays where it is.",
                ));
            }
            Err(error) => return Err(error),
        }
        session.returned = true;
    }
}

/// No snapshot: no coins on the chain yet, or a split just spent them.
fn empty(ceremony: &Project, session: &Session, ask: &mut dyn Ask) -> Result<Action> {
    nothing_found(ceremony)?;
    let mut choices = Vec::new();
    if session.online {
        choices.push(wallet_page::reread_choice());
    }
    choices.push(wallet_page::close_choice());
    wallet_page::pick_one(choices, ask)
}

/// Reads the coins again, since they may have moved. The old snapshot is
/// deleted first, otherwise a scan that finds nothing would leave it in
/// place.
fn reread(
    ceremony: &Project,
    session: &mut Session,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<()> {
    ceremony.remove_file(project::UTXOS)?;
    if let Some(server) = step_chain(ceremony, session.server.take(), ask, chain)? {
        session.server = Some(server);
    }
    Ok(())
}

fn refills(
    ceremony: &Project,
    session: &mut Session,
    free: usize,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<()> {
    // A group signed before stays committed (signing archives it); its
    // definition and plan do not describe the new one.
    clear_definition(ceremony)?;
    remind_unseen_refills(ceremony);
    if !ensure_words(ceremony, session, ask)? {
        return Ok(());
    }

    // OnionShare needs the network: with it unplugged, addresses are typed.
    let received = if session.online {
        road::at(Point::Addresses);
        phone::step_phone(&plebline::inbox::default_roots(), ask)?
    } else {
        ui::blank();
        ui::note(t(
            "La rete è staccata, quindi il telefono non può mandarmi gli indirizzi: li scrivi o \
             li incolli tu, uno alla volta.",
            "The network is unplugged, so the phone cannot send me the addresses: you type or \
             paste them, one at a time.",
        ));
        Vec::new()
    };

    road::at(Point::Count);
    let wanted = ask_payment_count(Some(free), received.len(), ask)?;
    if received.len() > wanted {
        let sent = received.len();
        ui::warning(&tf!(
            { "Dal telefono sono arrivati {sent} indirizzi: uso i primi {wanted}." },
            { "{sent} addresses came from the phone: I will use the first {wanted}." }
        ));
    }
    if wanted > free {
        if explain_split(free, wanted, ask)? {
            let mut tail = signing_road(session);
            tail.push(Point::Send);
            road::reroute(&tail);
            split(ceremony, session, free, Some(wanted), ask, chain)?;
        }
        return Ok(());
    }

    if session.online {
        road::at(Point::Unplug);
        step_unplug(false, ask)?;
        session.unplugged();
    }
    let Some(open) = session.open.as_ref() else {
        return Ok(());
    };
    road::at(Point::Sign);
    if !step_sign(ceremony, open, Some(wanted), &received, ask)? {
        return Ok(());
    }

    road::at(Point::Check);
    if !step_check(ceremony)? {
        ui::blank();
        ui::warning(t(
            "Non ti faccio spedire niente finché i controlli non tornano: sopra c'è scritto \
             cosa non va.",
            "I will not let you send anything until the checks come out right: what is wrong is \
             written above.",
        ));
        return Ok(());
    }

    road::at(Point::Export);
    if step_export(ceremony, ask)? {
        let group = plebline_core::verify::load(&ceremony.path(project::BATCH))?;
        session
            .copied
            .extend(group.txs.into_iter().map(|tx| tx.txid));
    }
    road::at(Point::Send);
    if step_send(ceremony, ask)? {
        send_one(ceremony, session, ask, chain)?;
    }
    Ok(())
}

/// `wanted` comes from refills that need more coins; from the wallet page
/// it is asked.
fn split(
    ceremony: &Project,
    session: &mut Session,
    free: usize,
    wanted: Option<usize>,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<()> {
    // From the refills, the reminder was already given.
    if wanted.is_none() {
        remind_unseen_refills(ceremony);
    }
    if !ensure_words(ceremony, session, ask)? {
        return Ok(());
    }
    let parts = match wanted {
        Some(parts) => parts,
        None => {
            road::at(Point::Parts);
            ask_parts(free, ask)?
        }
    };
    if session.online {
        road::at(Point::Unplug);
        step_unplug(true, ask)?;
        session.unplugged();
    }
    let Some(open) = session.open.take() else {
        return Ok(());
    };
    road::at(Point::Sign);
    // `do_split` drops the words before asking to plug the network back in.
    // They are not restored however it ends, a cancel included.
    let split = do_split(ceremony, open, session.server.clone(), parts, ask, chain);
    session.forget();
    if let Some(server) = split? {
        session.server = Some(server);
        session.plugged_back();
    }
    Ok(())
}

fn ask_parts(free: usize, ask: &mut dyn Ask) -> Result<usize> {
    ui::blank();
    ui::title(t("Dividi le monete", "Split the coins"));
    ui::note(&tf!(
        {
            "Una transazione verso te stesso: le {free} monete libere diventano tante monete più \
             piccole, e ognuna potrà fare una ricarica da sola. Quelle impegnate restano dove \
             sono."
        },
        {
            "One transaction to yourself: the {free} free coins become several smaller coins, \
             and each one will be able to make a refill on its own. The committed ones stay \
             where they are."
        }
    ));
    ui::blank();
    let question = Question::new(t("In quante parti?", "Into how many parts?")).invalid(t(
        "Inserisci un numero intero, almeno 2",
        "Enter a whole number, at least 2",
    ));
    Ok(ask.range_usize(question, 2..=40, Some(4))?)
}

/// Sending needs no words, so they are dropped first, as always before the
/// network is plugged back in.
fn send_one(
    ceremony: &Project,
    session: &mut Session,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<()> {
    // Checked again here, offline: a group that fails must never reach the
    // network.
    if !check_before_send(ceremony)? {
        ui::blank();
        ui::warning(t(
            "Questo gruppo non supera i controlli, quindi non lo mando: sopra c'è scritto cosa \
             non torna.",
            "This group does not pass the checks, so I will not send it: what does not add up \
             is written above.",
        ));
        return Ok(());
    }
    forget_words(session);
    plug_back(session, ask)?;
    let server = match session.server.clone() {
        Some(server) => server,
        None => ask_server(ask)?,
    };
    session.server = Some(server.clone());

    let index = pick_transaction(ceremony, ask)?;
    online::broadcast(
        NETWORK,
        &server,
        online::BroadcastInput {
            batch: ceremony.path(project::BATCH),
            index,
        },
        ask,
        chain,
    )?;
    Ok(())
}

/// Asks to plug the network back in, dropping the words first so that a
/// machine holding them is never online after signing.
pub(crate) fn plug_back(session: &mut Session, ask: &mut dyn Ask) -> Result<()> {
    if session.online {
        return Ok(());
    }
    forget_words(session);
    ui::blank();
    ui::warning(t(
        "Riattacca la rete prima di continuare.",
        "Plug the network back in before continuing.",
    ));
    ask.text(
        Question::new(t("Invio quando sei pronto:", "Enter when you are ready:")),
        None,
    )?;
    session.plugged_back();
    Ok(())
}

fn forget_words(session: &mut Session) {
    if session.forget() {
        ui::blank();
        ui::note(t(
            "Le tue parole le ho tolte dalla memoria: per mandare una transazione già firmata \
             bastano i suoi byte.",
            "I have removed your words from memory: sending a transaction that is already signed \
             only takes its bytes.",
        ));
    }
}

/// Refills of earlier sessions are set aside at the opening and never
/// read: their coins look free, and signing on them again makes two
/// refills of which only the first sent will count. Once per session,
/// before the first signature.
fn remind_unseen_refills(ceremony: &Project) {
    if !project::signed_files(ceremony).is_empty() {
        return;
    }
    ui::blank();
    ui::warning(t(
        "Prima di firmare: le ricariche firmate le volte prima non le guardo. Se ne hai ancora \
         qualcuna non mandata, sul telefono o su una chiavetta, le sue monete qui sembrano \
         libere: controlla di averle mandate tutte.",
        "Before signing: I do not look at the refills signed in earlier sessions. If you still \
         have some that were not sent, on the phone or a USB stick, their coins look free to me: \
         check that you have sent them all.",
    ));
    ui::note(t(
        "Se quelle vecchie non ti servono più, puoi firmare lo stesso. Quando una ricarica nuova \
         e una vecchia usano la stessa moneta, vale quella che mandi per prima e l'altra viene \
         rifiutata per sempre: butta via le vecchie che non vuoi più mandare.",
        "If you no longer need the old ones, you can sign anyway. When a new refill and an old \
         one use the same coin, the one you send first counts and the other is refused forever: \
         throw away the old ones you no longer want to send.",
    ));
}

/// The words for signing: still in memory, or typed again. They must open
/// the wallet whose coins were read, or the plan would be someone else's.
fn ensure_words(ceremony: &Project, session: &mut Session, ask: &mut dyn Ask) -> Result<bool> {
    if session.open.is_some() {
        return Ok(true);
    }
    let Some(expected) = wallet_fingerprint(ceremony) else {
        ui::warning(t(
            "Non trovo più la parte pubblica del portafoglio: non so con quali parole \
             confrontare quelle nuove.",
            "I can no longer find the wallet's public half: I do not know what to compare new \
             words with.",
        ));
        return Ok(false);
    };
    road::at(Point::Words);
    ui::blank();
    ui::title(t(
        "Servono di nuovo le tue parole",
        "Your words are needed again",
    ));
    ui::note(t(
        "Le avevo tolte dalla memoria prima che la rete tornasse. Per firmare servono di nuovo.",
        "I removed them from memory before the network came back. Signing needs them again.",
    ));
    loop {
        let open = step_seed(ask)?;
        let typed = open.checksum()?;
        if typed == expected {
            session.remember(open);
            return Ok(true);
        }
        ui::blank();
        ui::warning(&tf!(
            {
                "Queste parole aprono un altro portafoglio ({typed}), non quello di cui ho letto \
                 le monete ({expected})."
            },
            {
                "These words open a different wallet ({typed}), not the one whose coins I read \
                 ({expected})."
            }
        ));
        drop(open);
        let again = ask.confirm(
            Question::new(t("Le riscrivi?", "Will you type them again?")),
            true,
        )?;
        if !again {
            return Ok(false);
        }
    }
}

fn clear_definition(ceremony: &Project) -> Result<()> {
    for file in [project::CONFIG, project::PLAN] {
        ceremony.remove_file(file)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::Scripted;

    /// A group that fails its checks (here: fake signatures) never reaches
    /// the network, even when asked for from the wallet page.
    #[test]
    fn an_unsound_group_is_never_sent() {
        in_italian();
        let temp = Temp::new("manda-non-valido");
        let ceremony = temp.ceremony();
        write_wallet(&ceremony);
        write_batch(&ceremony, &[0, 1]);
        let mut session = Session {
            open: None,
            server: Some("http://server-finto".to_string()),
            online: true,
            returned: true,
            copied: Default::default(),
        };
        let mut ask = Scripted::new([]);
        let mut chain = plebline::chain::Fake::new();

        send_one(&ceremony, &mut session, &mut ask, &mut chain).expect("nessun errore");

        assert!(chain.sent().is_empty(), "non è partito niente");
        assert!(
            ask.asked().is_empty(),
            "non si chiede nemmeno quale mandare"
        );
    }
}
