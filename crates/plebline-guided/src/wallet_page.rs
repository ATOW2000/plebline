//! What the wallet page shows and asks: every coin with its state, and what
//! to do next. Only free coins are ever spent; to use a committed one, the
//! signed payment holding it is withdrawn first (file work is in
//! `plebline::replace`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use plebline_core::bitcoin::{OutPoint, Txid};
use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

use plebline::ask::{Ask, Question};
use plebline::chain::{Chain, TxStatus};
use plebline::project::{self, Project};
use plebline::replace::{self, Pending};
use plebline::{ui, wizard};

use crate::home::{plug_back, Session};
use crate::seed::master_fingerprint;
use crate::snapshot::ask_server;
use crate::NETWORK;

/// What the user picked at the wallet page.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Refills { free: usize },
    Split { free: usize },
    Send,
    Reread,
    Close,
}

enum Choice {
    Do(Action),
    Replace,
}

pub(crate) fn reread_choice() -> (Action, &'static str) {
    (
        Action::Reread,
        t("Rileggi la catena", "Read the chain again"),
    )
}

pub(crate) fn close_choice() -> (Action, &'static str) {
    (Action::Close, t("Chiudi e cancella", "Close and delete"))
}

/// Reads every signed batch on this machine.
pub(crate) fn step_home(
    ceremony: &Project,
    session: &mut Session,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<Action> {
    let mut sources = project::signed_files(ceremony);
    sources.extend(project::all_signed_files());
    home(ceremony, &sources, session, ask, chain)
}

/// Batches passed in, so tests do not read the real `~/PlebLine`.
fn home(
    ceremony: &Project,
    sources: &[PathBuf],
    session: &mut Session,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<Action> {
    let mut first = true;
    loop {
        let set = plebline_core::utxos::load(&ceremony.path(project::UTXOS), NETWORK)?;
        let wallet = set.descriptor_checksum.clone();
        let committed = wizard::committed_outpoints(
            sources,
            &ceremony.path(project::BATCH),
            wallet.as_deref(),
        )?;
        let on_hand: BTreeSet<OutPoint> = set.utxos.iter().map(|u| u.outpoint).collect();
        let free = on_hand.iter().filter(|c| !committed.contains(c)).count();
        // Without the wallet checksum, batches cannot be attributed: do not
        // offer to remove what may not be ours.
        let pending = match &wallet {
            Some(wallet) => replace::pending(sources, wallet, &on_hand)?,
            None => Vec::new(),
        };

        ui::blank();
        ui::title(t("Il tuo portafoglio", "Your wallet"));
        if let Some(name) = &wallet {
            match master_fingerprint(ceremony) {
                Some(print) => ui::info(&tf!({ "Portafoglio {name} · impronta {print}" }, {
                    "Wallet {name} · fingerprint {print}"
                })),
                None => ui::info(&tf!({ "Portafoglio {name}" }, { "Wallet {name}" })),
            }
        }
        if first && !session.returned {
            introduce(free, on_hand.len() - free);
        }
        first = false;
        show_coins(&set.utxos, &committed, &pending);

        if free == 0 && pending.is_empty() {
            ui::blank();
            ui::note(t(
                "Le monete sono impegnate dalla divisione, che va mandata in rete e aspettata in \
                 un blocco: finché non c'è, le monete nuove non esistono. Quando è confermata, \
                 rileggi la catena.",
                "The coins are held by the split, which has to be sent and waited for in a \
                 block: until it is, the new coins do not exist. Once it is confirmed, read the \
                 chain again.",
            ));
        }

        let mut choices = Vec::new();
        if free > 0 {
            choices.push((
                Choice::Do(Action::Refills { free }),
                t("Nuove ricariche", "New refills"),
            ));
            choices.push((
                Choice::Do(Action::Split { free }),
                t("Dividi le monete", "Split the coins"),
            ));
        }
        // Only a group signed in this session: earlier ones are set aside
        // at the opening and sent from the phone.
        if session.returned && ceremony.has(project::BATCH) {
            choices.push((
                Choice::Do(Action::Send),
                t("Manda una ricarica firmata", "Send a signed refill"),
            ));
        }
        if !pending.is_empty() {
            choices.push((
                Choice::Replace,
                t("Sostituisci una ricarica", "Replace a refill"),
            ));
        }
        if session.online && (session.returned || free == 0) {
            let (action, label) = reread_choice();
            choices.push((Choice::Do(action), label));
        }
        let (action, label) = close_choice();
        choices.push((Choice::Do(action), label));

        match pick_one(choices, ask)? {
            Choice::Do(action) => return Ok(action),
            Choice::Replace => {}
        }

        let chosen = pick_payment(&pending, ask)?;
        // Only the chain can say whether it was sent.
        plug_back(session, ask)?;
        if !never_sent(chosen, &mut session.server, ask, chain)? {
            continue;
        }
        if !confirm(chosen, ask)? {
            continue;
        }

        let done = replace::withdraw(&chosen.file, &chosen.txid)?;
        let index = plebline_core::batch::number(chosen.index);
        ui::blank();
        ui::success(&tf!(
            { "Fatto: la ricarica #{index} non c'è più, e la sua moneta è libera." },
            { "Done: refill #{index} is gone, and its coin is free." }
        ));
        if done.copies_removed > 0 {
            let count = done.copies_removed;
            ui::note(&tf!(
                { "Ho cancellato anche {count} sue copie accanto al gruppo firmato." },
                { "I also deleted {count} copies of it next to the signed group." }
            ));
        }
    }
}

/// The first choice is the default, and the usual next step.
pub(crate) fn pick_one<T>(choices: Vec<(T, &str)>, ask: &mut dyn Ask) -> Result<T> {
    let labels: Vec<&str> = choices.iter().map(|(_, label)| *label).collect();
    ui::blank();
    let index = ask.select(
        Question::new(t("Cosa facciamo?", "What do we do?")),
        &labels,
        None,
    )?;
    choices
        .into_iter()
        .nth(index)
        .map(|(choice, _)| choice)
        .ok_or_else(|| Error::Interaction(t("scelta non valida", "invalid choice").into()))
}

/// Explains free and committed coins, on the first visit.
fn introduce(free: usize, held: usize) {
    ui::blank();
    if free == 0 {
        ui::note(t(
            "Nessuna moneta è libera: ognuna è già dentro una ricarica firmata che non hai \
             ancora mandato. Non sono spese: sono ancora tue, sulla catena. Ma se firmassi \
             un'altra ricarica sulla stessa moneta, una delle due verrebbe rifiutata per sempre.",
            "No coin is free: every one is already inside a signed refill you have not sent \
             yet. They are not spent: they are still yours, on the chain. But signing another \
             refill on the same coin would get one of the two refused forever.",
        ));
        return;
    }
    if held == 0 {
        ui::note(t(
            "Nessuna delle tue monete è dentro una ricarica firmata: sono tutte libere.",
            "None of your coins is inside a signed refill: they are all free.",
        ));
        return;
    }
    ui::note(t(
        "Le monete libere non sono in nessuna ricarica firmata. Le altre sono impegnate: \
         stanno dentro ricariche firmate che non hai ancora mandato. Sono ancora tue, ma se \
         firmassi un'altra ricarica sulla stessa moneta, una delle due verrebbe rifiutata per \
         sempre. Per usarne una, prima si toglie la ricarica che la tiene.",
        "Free coins are in no signed refill. The others are committed: they sit inside \
         signed refills you have not sent yet. They are still yours, but signing another \
         refill on the same coin would get one of the two refused forever. To use one, the \
         refill holding it is removed first.",
    ));
}

/// Table of every coin and its state.
fn show_coins(
    coins: &[plebline_core::utxos::ValidatedUtxo],
    committed: &BTreeSet<OutPoint>,
    pending: &[Pending],
) {
    let mut table = ui::table(&[
        t("Moneta", "Coin"),
        t("Quanto vale", "Worth"),
        t("Stato", "State"),
    ]);
    for coin in coins {
        let state = match pending.iter().find(|p| p.coins.contains(&coin.outpoint)) {
            Some(payment) => {
                let index = plebline_core::batch::number(payment.index);
                tf!({ "firmata, non inviata (ricarica #{index})" }, {
                    "signed, not sent (refill #{index})"
                })
            }
            None if committed.contains(&coin.outpoint) => {
                t("usata da una firma", "used by a signature").to_string()
            }
            None => t("libera", "free").to_string(),
        };
        table.add_row(vec![
            comfy_table::Cell::new(ui::short_outpoint(&coin.outpoint.to_string())),
            ui::right(ui::sats(coin.value.to_sat())),
            comfy_table::Cell::new(state),
        ]);
    }
    ui::show(&table);
}

fn describe(payment: &Pending) -> String {
    let index = plebline_core::batch::number(payment.index);
    let amount = ui::sats(payment.leaving_sat);
    let to = payment
        .recipients
        .first()
        .map(|a| ui::short_address(a))
        .unwrap_or_else(|| "—".into());
    let mut line = tf!({ "#{index} · {amount} a {to}" }, {
        "#{index} · {amount} to {to}"
    });
    if let Some(label) = &payment.label {
        line.push_str(&format!(" · {label}"));
    }
    if let Some(folder) = payment
        .file
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
    {
        line.push_str(&tf!({ " · cerimonia «{folder}»" }, {
            " · ceremony «{folder}»"
        }));
    }
    line
}

fn pick_payment<'a>(pending: &'a [Pending], ask: &mut dyn Ask) -> Result<&'a Pending> {
    let lines: Vec<String> = pending.iter().map(describe).collect();
    let options: Vec<&str> = lines.iter().map(String::as_str).collect();
    let index = ask.select(
        Question::new(t(
            "Quale ricarica vuoi sostituire?",
            "Which refill do you want to replace?",
        )),
        &options,
        Some(10),
    )?;
    pending
        .get(index)
        .ok_or_else(|| Error::Interaction(t("scelta non valida", "invalid choice").into()))
}

/// Asks the chain: a payment already in the mempool cannot be replaced.
fn never_sent(
    payment: &Pending,
    server: &mut Option<String>,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<bool> {
    let txid: Txid = payment.txid.parse().map_err(|_| {
        let declared = &payment.txid;
        Error::Verification(tf!({ "txid illeggibile nel file: {declared}" }, {
            "unreadable txid in the file: {declared}"
        }))
    })?;
    let api = match server.clone() {
        Some(api) => api,
        None => {
            let chosen = ask_server(ask)?;
            *server = Some(chosen.clone());
            chosen
        }
    };
    let index = plebline_core::batch::number(payment.index);
    match chain.tx_status(&api, txid)? {
        TxStatus::NeverSent => Ok(true),
        TxStatus::InMempool => {
            ui::blank();
            ui::warning(&tf!(
                {
                    "La ricarica #{index} è già stata mandata e aspetta un blocco: non si può più \
                     sostituire da qui."
                },
                {
                    "Refill #{index} has already been sent and is waiting for a block: it can no \
                     longer be replaced from here."
                }
            ));
            Ok(false)
        }
        TxStatus::Confirmed { height } => {
            ui::blank();
            ui::warning(&tf!(
                { "La ricarica #{index} è già in un blocco ({height}): è fatta." },
                { "Refill #{index} is already in a block ({height}): it is done." }
            ));
            Ok(false)
        }
    }
}

/// Warns first that other copies of the payment stay valid. Default: no.
fn confirm(payment: &Pending, ask: &mut dyn Ask) -> Result<bool> {
    let index = plebline_core::batch::number(payment.index);
    let what = describe(payment);
    ui::blank();
    ui::info(&what);
    ui::warning(t(
        "Tolgo questa ricarica dal gruppo firmato e la sua moneta torna libera. Le altre \
         ricariche restano valide. Attenzione: se questa transazione è anche su una chiavetta, \
         stampata o in un QR, quella copia vale ancora. Chi la manda in rete prima di te fa \
         rifiutare la ricarica nuova. Se ne hai una copia, distruggila.",
        "I take this refill out of the signed group and its coin becomes free. The other \
         refills stay valid. Careful: if this transaction is also on a stick, printed or in a \
         QR code, that copy is still valid. Whoever sends it first gets the new refill \
         refused. If you have a copy, destroy it.",
    ));
    Ok(ask.confirm(
        Question::new(&tf!(
            { "Tolgo la ricarica #{index} e libero la sua moneta?" },
            { "Shall I remove refill #{index} and free its coin?" }
        )),
        false,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};
    use plebline_core::batch::{SignedBatch, SignedInput, SignedOutput, SignedTx};

    /// Payment `k` spends `coins[k]`.
    fn sign_over(ceremony: &Project, coins: &[String]) -> PathBuf {
        let checksum = fingerprint(&test_wallet(SEED12));
        let batch = SignedBatch {
            version: plebline_core::batch::BATCH_SCHEMA_VERSION,
            network: NETWORK.to_string(),
            created_by: "test".into(),
            descriptor_checksum: checksum,
            fee_rate_sat_vb: 1.0,
            txs: coins
                .iter()
                .enumerate()
                .map(|(k, outpoint)| SignedTx {
                    index: k,
                    label: Some(format!("pagamento {k}")),
                    txid: fake_txid(k),
                    hex: plebline::testing::spending_hex(outpoint),
                    psbt: "cHNidP8=".into(),
                    inputs: vec![SignedInput {
                        outpoint: outpoint.clone(),
                        value_sat: 100_000,
                    }],
                    outputs: vec![SignedOutput {
                        address: recipient(k as u32),
                        value_sat: 90_000,
                        is_change: false,
                    }],
                    fee_sat: 1_000,
                    fee_rate_sat_vb: 1.0,
                    vsize: 141,
                })
                .collect(),
        };
        let path = ceremony.path(project::BATCH);
        wizard::write_json(&path, &batch).expect("scritto");
        path
    }

    fn session() -> Session {
        Session {
            open: None,
            server: Some("http://server-finto".to_string()),
            online: true,
            returned: false,
            copied: Default::default(),
        }
    }

    fn options(ask: &Scripted) -> Vec<String> {
        ask.asked()[0].options.clone()
    }

    /// One free coin, one committed: new refills come first, and the
    /// committed coin is offered through its payment.
    #[test]
    fn the_wallet_offers_new_refills_first() {
        in_italian();
        let temp = Temp::new("portafoglio-libera");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins[..1]);

        let mut ask = Scripted::new([Reply::like("Nuove ricariche")]);
        let mut chain = plebline::chain::Fake::new();
        let mut session = session();

        let action = home(
            &ceremony,
            std::slice::from_ref(&batch),
            &mut session,
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert_eq!(
            action,
            Action::Refills { free: 1 },
            "con l'unica moneta libera"
        );
        assert_eq!(ask.prompts(), vec!["Cosa facciamo?"]);
        assert_eq!(
            options(&ask)[0],
            "Nuove ricariche",
            "il passo di sempre, per primo"
        );
        assert!(
            options(&ask)
                .iter()
                .any(|o| o == "Sostituisci una ricarica"),
            "la moneta impegnata va offerta: {:?}",
            options(&ask)
        );
        assert_eq!(
            plebline_core::verify::load(&batch)
                .expect("intatto")
                .txs
                .len(),
            1,
            "scegliere le libere non tocca le ricariche firmate"
        );
    }

    /// The first visit: nothing to send yet, nothing to read again.
    #[test]
    fn the_first_visit_offers_no_sending() {
        in_italian();
        let temp = Temp::new("portafoglio-prima");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);

        let mut ask = Scripted::new([Reply::like("Nuove ricariche")]);
        let mut chain = plebline::chain::Fake::new();

        let action =
            home(&ceremony, &[], &mut session(), &mut ask, &mut chain).expect("nessun errore");

        assert_eq!(action, Action::Refills { free: 2 });
        assert_eq!(
            options(&ask),
            vec!["Nuove ricariche", "Dividi le monete", "Chiudi e cancella"]
        );
    }

    /// Reopened: last time's group is set aside, its coin is free again,
    /// and nothing offers to send or replace it, even coming back.
    #[test]
    fn after_reopening_last_times_coin_is_free() {
        in_italian();
        let temp = Temp::new("portafoglio-da-zero");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        sign_over(&ceremony, &coins[..1]);

        crate::ceremony::start_fresh(&ceremony).expect("ripartenza");
        snapshot_of_two(&ceremony);

        let mut ask = Scripted::new([Reply::like("Nuove ricariche")]);
        let mut chain = plebline::chain::Fake::new();
        let mut session = session();
        session.returned = true;
        let action = home(
            &ceremony,
            &project::signed_files(&ceremony),
            &mut session,
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert_eq!(
            action,
            Action::Refills { free: 2 },
            "la moneta della volta scorsa è di nuovo libera"
        );
        assert!(
            !options(&ask)
                .iter()
                .any(|o| o == "Manda una ricarica firmata" || o == "Sostituisci una ricarica"),
            "{:?}",
            options(&ask)
        );
    }

    /// Back at the wallet with a signed group: it can be sent from here.
    #[test]
    fn coming_back_offers_sending_and_reading_again() {
        in_italian();
        let temp = Temp::new("portafoglio-ritorno");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins[..1]);

        let mut ask = Scripted::new([Reply::like("Manda una ricarica firmata")]);
        let mut chain = plebline::chain::Fake::new();
        let mut session = session();
        session.returned = true;

        let action =
            home(&ceremony, &[batch], &mut session, &mut ask, &mut chain).expect("nessun errore");

        assert_eq!(action, Action::Send);
        assert!(options(&ask).iter().any(|o| o == "Rileggi la catena"));
    }

    #[test]
    fn closing_from_the_wallet_is_its_own_choice() {
        in_italian();
        let temp = Temp::new("portafoglio-chiudi");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins[..1]);

        let mut ask = Scripted::new([Reply::like("Chiudi e cancella")]);
        let mut chain = plebline::chain::Fake::new();

        let action =
            home(&ceremony, &[batch], &mut session(), &mut ask, &mut chain).expect("nessun errore");
        assert_eq!(action, Action::Close);
    }

    /// One coin free, one committed: replacing withdraws its payment, and
    /// then both are free.
    #[test]
    fn the_committed_coin_is_used_only_after_its_payment_is_withdrawn() {
        in_italian();
        let temp = Temp::new("sostituisci-anche");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins[..1]);

        let mut ask = Scripted::new([
            Reply::like("Sostituisci una ricarica"),
            Reply::like("#1"),
            Reply::Yes,
            Reply::like("Nuove ricariche"),
        ]);
        let mut chain = plebline::chain::Fake::new().with_status(fake_txid(0), TxStatus::NeverSent);

        let action = home(
            &ceremony,
            std::slice::from_ref(&batch),
            &mut session(),
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert_eq!(
            action,
            Action::Refills { free: 2 },
            "adesso sono libere tutte e due"
        );
        assert_eq!(ask.remaining(), 0);
        assert!(!batch.exists(), "il gruppo rimasto vuoto sparisce");
    }

    /// Both coins held: one payment withdrawn, the other kept.
    #[test]
    fn with_no_free_coin_one_payment_can_be_replaced() {
        in_italian();
        let temp = Temp::new("sostituisci-uno");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins);

        let mut ask = Scripted::new([
            Reply::like("Sostituisci una ricarica"),
            Reply::like("#2"),
            Reply::Yes,
            Reply::like("Nuove ricariche"),
        ]);
        let mut chain = plebline::chain::Fake::new().with_status(fake_txid(1), TxStatus::NeverSent);

        let action = home(
            &ceremony,
            std::slice::from_ref(&batch),
            &mut session(),
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert!(
            !options(&ask).iter().any(|o| o == "Nuove ricariche"),
            "senza monete libere non si offrono ricariche: {:?}",
            options(&ask)
        );
        assert_eq!(action, Action::Refills { free: 1 });
        assert_eq!(ask.remaining(), 0);
        let left = plebline_core::verify::load(&batch).expect("il gruppo resta");
        let txids: Vec<_> = left.txs.iter().map(|tx| tx.txid.clone()).collect();
        assert_eq!(txids, vec![fake_txid(0)], "resta solo la ricarica #1");
    }

    /// Offline with the words in memory: asking the chain needs the
    /// network, and the words are dropped before it is plugged back in.
    #[test]
    fn replacing_offline_drops_the_words_before_the_network_returns() {
        in_italian();
        let temp = Temp::new("sostituisci-staccato");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins);

        let mut ask = Scripted::new([
            Reply::like("Sostituisci una ricarica"),
            Reply::like("#1"),
            Reply::text(""),
            Reply::No,
            Reply::like("Chiudi e cancella"),
        ]);
        let mut chain = plebline::chain::Fake::new().with_status(fake_txid(0), TxStatus::NeverSent);
        let mut session = session();
        session.online = false;
        session.open = Some(crate::Open {
            seed: plebline_core::wallet::SeedWords::from_interactive_input(SEED12)
                .expect("seed valido"),
            passphrase: None,
        });

        home(&ceremony, &[batch], &mut session, &mut ask, &mut chain).expect("nessun errore");

        assert!(session.open.is_none(), "le parole sono uscite");
        assert!(session.online, "e la rete è tornata");
        assert!(
            ask.prompts().contains(&"Invio quando sei pronto:"),
            "{:?}",
            ask.prompts()
        );
    }

    /// The split spends only the free coin: sending it must not cancel the
    /// payment holding the other.
    #[test]
    fn the_split_leaves_the_committed_coin_alone() {
        in_italian();
        let temp = Temp::new("divisione-impegnata");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        sign_over(&ceremony, &coins[..1]);

        let seed =
            plebline_core::wallet::SeedWords::from_interactive_input(SEED12).expect("seed valido");
        let mut ask = Scripted::new([Reply::number(2.0f64), Reply::Yes]);
        let signed = wizard::split(
            NETWORK,
            wizard::SplitInput {
                utxos: ceremony.path(project::UTXOS),
                previous: project::signed_files(&ceremony),
                out: ceremony.path(project::SPLIT),
                wallet: wizard::WalletAccess::Seed {
                    seed: &seed,
                    passphrase: None,
                },
                outputs: Some(3),
            },
            &mut ask,
        )
        .expect("divisione firmata");

        assert!(signed);
        let split = plebline_core::verify::load(&ceremony.path(project::SPLIT)).expect("letta");
        let spent: Vec<&str> = split.txs[0]
            .inputs
            .iter()
            .map(|i| i.outpoint.as_str())
            .collect();
        assert_eq!(
            spent,
            vec![coins[1].as_str()],
            "entra solo la moneta libera"
        );
    }

    /// A payment the chain has seen is not withdrawn.
    #[test]
    fn a_payment_already_sent_is_not_replaced() {
        in_italian();
        let temp = Temp::new("sostituisci-mandato");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins);

        let mut ask = Scripted::new([
            Reply::like("Sostituisci una ricarica"),
            Reply::like("#1"),
            Reply::like("Chiudi e cancella"),
        ]);
        let mut chain = plebline::chain::Fake::new().with_status(fake_txid(0), TxStatus::InMempool);

        let action = home(
            &ceremony,
            std::slice::from_ref(&batch),
            &mut session(),
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert_eq!(action, Action::Close);
        assert_eq!(
            plebline_core::verify::load(&batch)
                .expect("intatto")
                .txs
                .len(),
            2
        );
    }

    /// A network error is not an answer: the refill is not taken for unsent.
    #[test]
    fn without_an_answer_from_the_chain_nothing_is_replaced() {
        in_italian();
        let temp = Temp::new("sostituisci-senza-risposta");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins);

        let mut ask = Scripted::new([Reply::like("Sostituisci una ricarica"), Reply::like("#1")]);
        let mut chain = plebline::chain::Fake::new().failing_status(fake_txid(0), "tempo scaduto");

        let error = home(
            &ceremony,
            std::slice::from_ref(&batch),
            &mut session(),
            &mut ask,
            &mut chain,
        )
        .expect_err("senza risposta non si sostituisce");

        assert!(matches!(error, Error::Network(_)), "{error}");
        assert_eq!(
            plebline_core::verify::load(&batch)
                .expect("intatto")
                .txs
                .len(),
            2
        );
    }

    /// No, the default, leaves everything as it was.
    #[test]
    fn saying_no_changes_nothing() {
        in_italian();
        let temp = Temp::new("sostituisci-no");
        let ceremony = temp.ceremony();
        let coins = snapshot_of_two(&ceremony);
        let batch = sign_over(&ceremony, &coins);

        let mut ask = Scripted::new([
            Reply::like("Sostituisci una ricarica"),
            Reply::like("#1"),
            Reply::No,
            Reply::like("Chiudi e cancella"),
        ]);
        let mut chain = plebline::chain::Fake::new().with_status(fake_txid(0), TxStatus::NeverSent);

        let action = home(
            &ceremony,
            std::slice::from_ref(&batch),
            &mut session(),
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert_eq!(action, Action::Close);
        assert_eq!(
            plebline_core::verify::load(&batch)
                .expect("intatto")
                .txs
                .len(),
            2
        );
    }
}
