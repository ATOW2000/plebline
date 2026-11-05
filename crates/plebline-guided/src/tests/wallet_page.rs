//! The wallet page: refills, sending, splitting, and the stick.

use super::*;

/// The stick gets the right addresses, derived from the public half. The
/// file is checked, since that is what the user will open.
#[test]
fn the_receiving_addresses_end_up_on_the_stick() {
    in_italian();
    let temp = Temp::new("indirizzi");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let stick = temp.path.join("chiavetta");
    std::fs::create_dir_all(&stick).expect("cartella creabile");

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut ask = up_to_the_server([
        Reply::like("Nuove ricariche"),
        Reply::like("Li incollo"),
        Reply::number(1usize),
        Reply::text("fatto"),
        Reply::number(2.0f64),
        Reply::text(recipient(0)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::Yes,
        Reply::like("Su una chiavetta"),
        Reply::text(stick.display().to_string()),
        Reply::like("PSBT"),
        Reply::number(3usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);

    // The chosen QR holds the same PSBT the library produces.
    let payments = stick.join("plebline-ricariche");
    let batch = plebline_core::verify::load(&payments.join("batch.json")).expect("batch");
    let tx = &batch.txs[0];
    let drawn = payments.join(format!("psbt-{}-{}.png", tx.index, &tx.txid[..8]));
    assert_eq!(
        plebline::qr::decode_file(&drawn).expect("il QR del PSBT si rilegge"),
        plebline_core::batch::shareable_psbt(tx).expect("PSBT condivisibile")
    );

    let sheet = stick.join("plebline-ricariche/indirizzi-ricezione.txt");
    let written = std::fs::read_to_string(&sheet).expect("il foglio degli indirizzi c'è");

    // Coins used indexes 0 and 1: the first free is 2.
    for index in 2..5u32 {
        let expected = wallet
            .peek_address(plebline_core::bdk_wallet::KeychainKind::External, index)
            .address
            .to_string();
        assert!(
            written.contains(&expected),
            "manca l'indirizzo #{index} ({expected}) in:\n{written}"
        );
        assert!(
            stick
                .join(format!("plebline-ricariche/ricevi-{index}.png"))
                .exists(),
            "manca il QR dell'indirizzo #{index}"
        );
    }

    let first = wallet
        .peek_address(plebline_core::bdk_wallet::KeychainKind::External, 0)
        .address
        .to_string();
    assert!(
        !written.contains(&first),
        "l'indice 0 ha già una storia in catena: non va riconsegnato"
    );
}

/// The answers that sign one refill to `recipient(k)` and take it nowhere.
fn one_refill(k: u32, unplug: bool) -> Vec<Reply> {
    let mut replies = vec![Reply::number(1usize)];
    if unplug {
        replies.push(Reply::text("fatto"));
    }
    replies.extend([
        Reply::number(2.0f64),
        Reply::text(recipient(k)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::Yes,
        Reply::like("Su una chiavetta"),
        Reply::text(""),
        Reply::number(3usize),
    ]);
    replies
}

/// Two rounds in one sitting, back at the wallet in between: the second
/// never spends a coin of the first, and with the network still unplugged
/// the words are not asked again.
#[test]
fn two_rounds_of_refills_never_share_a_coin() {
    in_italian();
    let temp = Temp::new("due-giri");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2))
        .with(coin(&wallet, 2, 500_000, 3));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut replies = vec![Reply::like("Nuove ricariche"), Reply::like("Li incollo")];
    replies.extend(one_refill(0, true));
    replies.push(Reply::No);
    // Second round: offline, so no phone and no unplugging.
    replies.push(Reply::like("Nuove ricariche"));
    replies.extend(one_refill(1, false));
    replies.extend([Reply::No, Reply::like("Chiudi e cancella"), Reply::No]);
    let mut ask = up_to_the_server(replies);

    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert_script_used_up(&ask);
    let seeds = ask.asked().iter().filter(|a| a.kind == Kind::Seed).count();
    assert_eq!(seeds, 1, "a rete staccata le parole restano in memoria");
    assert_eq!(chain.servers().len(), 1, "una lettura sola della catena");

    let files = project::signed_files(&ceremony);
    assert!(files.len() >= 2, "il primo gruppo è archiviato: {files:?}");
    let mut spent = Vec::new();
    for file in &files {
        let batch = plebline_core::verify::load(file).expect("leggibile");
        for tx in &batch.txs {
            spent.extend(tx.inputs.iter().map(|i| i.outpoint.clone()));
        }
    }
    let distinct: std::collections::BTreeSet<_> = spent.iter().collect();
    assert_eq!(spent.len(), 2, "una moneta per ricarica: {spent:?}");
    assert_eq!(
        distinct.len(),
        spent.len(),
        "nessuna moneta in due ricariche"
    );
}

/// Refills of earlier sessions are never read: the user is reminded before
/// the first signature of the session. Once this session has signed some,
/// the program sees them and the reminder is not repeated.
#[test]
fn earlier_refills_are_recalled_once_per_session() {
    in_italian();
    let temp = Temp::new("promemoria-ricariche");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2))
        .with(coin(&wallet, 2, 500_000, 3));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut replies = vec![Reply::like("Nuove ricariche"), Reply::like("Li incollo")];
    replies.extend(one_refill(0, true));
    replies.push(Reply::No);
    replies.push(Reply::like("Nuove ricariche"));
    replies.extend(one_refill(1, false));
    replies.extend([Reply::No, Reply::like("Chiudi e cancella"), Reply::No]);
    let mut ask = up_to_the_server(replies);

    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));
    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_script_used_up(&ask);
    let screen = notebook.text();
    let reminders = screen
        .matches("le ricariche firmate le volte prima non le guardo")
        .count();
    assert_eq!(reminders, 1, "solo al primo giro: {screen}");
    assert!(
        screen.contains("vale quella che mandi per prima"),
        "dice cosa succede a firmare sopra una vecchia: {screen}"
    );
}

/// The refills still to send are listed as the screen counts them, from 1.
#[test]
fn after_sending_the_others_are_counted_from_one() {
    in_italian();
    let temp = Temp::new("manda-una-di-due");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut replies = vec![Reply::like("Nuove ricariche"), Reply::like("Li incollo")];
    replies.extend([
        Reply::number(2usize),
        Reply::text("fatto"),
        Reply::number(2.0f64),
        Reply::text(recipient(0)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::text(recipient(1)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::Yes,
        Reply::like("Su una chiavetta"),
        Reply::text(""),
        Reply::number(3usize),
        Reply::Yes,
        Reply::text(""),
        Reply::like("#1"),
        send_it("100000"),
        Reply::like("Chiudi e cancella"),
        Reply::No,
    ]);
    let mut ask = up_to_the_server(replies);

    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));
    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_script_used_up(&ask);
    assert_eq!(chain.sent().len(), 1, "una ricarica mandata");
    let text = notebook.text();
    assert!(
        text.contains("Restano trasmissibili le transazioni #2,"),
        "{text}"
    );
}

/// Sending at the end of a round: the words are dropped before the network
/// is plugged back in, so signing again needs them typed, and they must
/// open the same wallet.
#[test]
fn sending_drops_the_words_and_other_words_are_refused() {
    in_italian();
    let temp = Temp::new("manda-e-riparti");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut replies = vec![Reply::like("Nuove ricariche"), Reply::like("Li incollo")];
    replies.extend(one_refill(0, true));
    replies.extend([
        // Send one now: network back on, pick #1, confirm.
        Reply::Yes,
        Reply::text(""),
        Reply::like("#1"),
        send_it("100000"),
        // Back at the wallet: a new round needs the words again.
        Reply::like("Nuove ricariche"),
        Reply::like("24"),
        Reply::seed(SEED24),
        Reply::text(""),
        // Another wallet: not typed again.
        Reply::No,
        Reply::like("Chiudi e cancella"),
        Reply::No,
    ]);
    let mut ask = up_to_the_server(replies);

    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));
    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_script_used_up(&ask);
    assert_eq!(chain.sent().len(), 1, "una ricarica mandata");

    // The step line sent to the window goes through the same steps.
    let places = notebook.places();
    let mut visited: Vec<&str> = Vec::new();
    for place in &places {
        let label = place.road[place.here].label.as_str();
        if visited.last() != Some(&label) {
            visited.push(label);
        }
    }
    assert_eq!(
        visited,
        vec![
            "Inizio",
            "Rete",
            "Parole",
            "Portafoglio",
            "Indirizzi",
            "Quante",
            "Stacca",
            "Firma",
            "Controllo",
            "Uscita",
            "Invio",
            "Portafoglio",
            "Parole",
            "Portafoglio",
        ]
    );
    let links: Vec<Link> = places.iter().map(|p| p.link).collect::<Vec<_>>();
    let mut changes = links.clone();
    changes.dedup();
    assert_eq!(
        changes,
        vec![Link::Unknown, Link::Tor, Link::Unplugged, Link::Tor],
        "la rete: da controllare, Tor, staccata, di nuovo Tor"
    );
    for pair in places.windows(2) {
        if pair[0].link == Link::Unplugged && pair[1].link != Link::Unplugged {
            assert!(!pair[1].words, "la rete torna solo a parole già uscite");
        }
    }

    let asked = ask.asked();
    let cable = asked
        .iter()
        .position(|a| a.prompt == "Invio quando sei pronto:")
        .expect("la rete è stata riattaccata");
    let seeds: Vec<usize> = asked
        .iter()
        .enumerate()
        .filter(|(_, a)| a.kind == Kind::Seed)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        seeds.len(),
        2,
        "le parole si ribattono per firmare di nuovo"
    );
    assert!(seeds[0] < cable && cable < seeds[1], "e solo dopo la rete");
    assert!(
        ask.prompts().contains(&"Le riscrivi?"),
        "parole di un altro portafoglio: {:?}",
        ask.prompts()
    );
    let wallet_after_send = asked
        .iter()
        .filter(|a| a.prompt == "Cosa facciamo?")
        .nth(1)
        .expect("di nuovo al portafoglio");
    assert!(
        wallet_after_send
            .options
            .iter()
            .any(|o| o == "Manda una ricarica firmata"),
        "tornando, si può mandare: {:?}",
        wallet_after_send.options
    );
}

/// Split from the wallet, declined at the very last question: nothing
/// is called sent, the coins seen stay, and they show as held by the
/// split.
#[test]
fn a_split_declined_at_the_last_question_is_not_called_sent() {
    in_italian();
    let temp = Temp::new("dividi-rifiutata");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));
    let mut ask = up_to_the_server([
        Reply::like("Dividi le monete"),
        Reply::number(3usize),
        Reply::text("fatto"),
        Reply::number(2.0f64),
        Reply::Yes,
        Reply::text(""),
        // Declined: nothing is broadcast. A split asks yes or no on every
        // network, as nothing leaves the wallet.
        Reply::No,
        Reply::like("Chiudi e cancella"),
        // The split, signed and never sent, exists only here: kept.
        Reply::No,
    ]);
    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_script_used_up(&ask);
    assert!(
        ceremony.has(project::SPLIT),
        "la divisione non mandata resta"
    );
    assert!(chain.sent().is_empty(), "non è partito niente");
    let screen = notebook.text();
    assert!(!screen.contains("Divisione mandata"), "{screen}");
    assert!(screen.contains("non è partita"), "{screen}");
    assert!(
        screen.contains("impegnate dalla divisione"),
        "il portafoglio dice chi tiene le monete: {screen}"
    );
}

/// "No" to signing: nothing signed, nothing re-checked or exported as if
/// it were, back at the wallet with the words still in memory.
#[test]
fn declining_to_sign_goes_back_to_the_wallet() {
    in_italian();
    let temp = Temp::new("firma-rifiutata");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut ask = up_to_the_server([
        Reply::like("Nuove ricariche"),
        Reply::like("Li incollo"),
        Reply::number(1usize),
        Reply::text("fatto"),
        Reply::number(2.0f64),
        Reply::text(recipient(0)),
        Reply::number(100_000u64),
        Reply::text(""),
        // "Confirm and proceed with signing?"
        Reply::No,
        // Nothing signed: closing asks nothing more.
        Reply::like("Chiudi e cancella"),
    ]);
    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert_script_used_up(&ask);
    assert!(!ceremony.has(project::BATCH), "niente è stato firmato");
    assert!(project::signed_files(&ceremony).is_empty());
    assert!(chain.sent().is_empty());
    assert_eq!(
        ask.prompts()
            .iter()
            .filter(|p| **p == "Cosa facciamo?")
            .count(),
        2,
        "di nuovo al portafoglio: {:?}",
        ask.prompts()
    );
}

/// A new round left unsigned replaces the definition; the group signed
/// before must still be sendable, since sending does not use it.
#[test]
fn a_new_round_left_unsigned_does_not_block_the_signed_group() {
    in_italian();
    let temp = Temp::new("giro-non-firmato");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut replies = vec![Reply::like("Nuove ricariche"), Reply::like("Li incollo")];
    replies.extend(one_refill(0, true));
    replies.extend([
        // Not sent now.
        Reply::No,
        // A second round, to another recipient, not signed.
        Reply::like("Nuove ricariche"),
        Reply::number(1usize),
        Reply::number(2.0f64),
        Reply::text(recipient(1)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::No,
        // The first group goes out.
        Reply::like("Manda una ricarica firmata"),
        Reply::text(""),
        Reply::like("#1"),
        send_it("100000"),
        Reply::like("Chiudi e cancella"),
        Reply::No,
    ]);
    let mut ask = up_to_the_server(replies);
    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert_script_used_up(&ask);
    assert_eq!(chain.sent().len(), 1, "il gruppo sano è partito");
}

/// After the first refill went out and the chain was read again, its coin
/// is no longer listed; the second refill must still be sendable.
#[test]
fn after_reading_the_chain_again_the_second_refill_goes_out() {
    in_italian();
    let temp = Temp::new("rilettura-poi-seconda");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let first = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let after = plebline::chain::ScanResult::empty(800_001, fingerprint(&wallet))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new()
        .with_scan(first)
        .with_scan(after);

    let mut replies = vec![Reply::like("Nuove ricariche"), Reply::like("Li incollo")];
    replies.extend([
        Reply::number(2usize),
        Reply::text("fatto"),
        Reply::number(2.0f64),
        Reply::text(recipient(0)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::text(recipient(1)),
        Reply::number(100_000u64),
        Reply::text(""),
        Reply::Yes,
        Reply::like("Su una chiavetta"),
        Reply::text(""),
        Reply::number(3usize),
        Reply::Yes,
        Reply::text(""),
        Reply::like("#1"),
        send_it("100000"),
        Reply::like("Rileggi la catena"),
        Reply::like("Manda una ricarica firmata"),
        Reply::like("#2"),
        send_it("100000"),
        Reply::like("Chiudi e cancella"),
        Reply::No,
    ]);
    let mut ask = up_to_the_server(replies);
    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert_script_used_up(&ask);
    assert_eq!(chain.sent().len(), 2, "partite tutte e due");
}
