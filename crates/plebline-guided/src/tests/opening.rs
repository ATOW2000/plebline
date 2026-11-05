//! The opening: the wallet question, the network, the first scan.

use super::*;

/// The wallet question is always asked, before the words, whatever is in
/// the directory.
#[test]
fn the_wallet_question_is_asked_even_with_a_full_folder() {
    in_italian();
    let temp = Temp::new("piena");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    assert!(ceremony.has(project::WALLET), "la cartella non è vuota");

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
        Reply::Yes,
        Reply::like("Su una chiavetta"),
        Reply::text(""),
        Reply::number(2usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
        Reply::No,
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);

    let questions = ask.prompts();
    let wallet_question = questions
        .iter()
        .position(|d| d.contains("ce le hai già"))
        .expect("la domanda sul portafoglio deve essere stata fatta");
    let words = ask
        .asked()
        .iter()
        .position(|a| a.kind == Kind::Seed)
        .expect("le parole devono essere state chieste");
    assert!(
        wallet_question < words,
        "si chiede prima delle parole, non dopo: {questions:?}"
    );
}

#[test]
fn a_network_that_is_not_there_stops_before_the_words() {
    in_italian();
    let temp = Temp::new("rete-assente");
    let ceremony = temp.ceremony();
    let mut chain = plebline::chain::Fake::new().probe_fails("Tor non risponde");

    let mut ask = from_the_opening([
        Reply::like("Ce le ho già"),
        Reply::like("Un server pubblico"),
        Reply::like("Lascio stare"),
    ]);

    run_in(&ceremony, &mut ask, &mut chain).expect("fermarsi qui non è un errore");

    assert_script_used_up(&ask);
    assert!(
        !ask.asked().iter().any(|a| a.kind == Kind::Seed),
        "le parole non devono essere state chieste: {:?}",
        ask.prompts()
    );
    assert_eq!(
        chain.probes().len(),
        plebline::online::default_apis(NETWORK)
            .expect("mainnet")
            .len(),
        "ha bussato una volta a ogni server pubblico"
    );
    assert!(
        chain.servers().is_empty(),
        "e non ha mai dato via il portafoglio"
    );
    assert!(
        !ceremony.has(project::WALLET),
        "e non ha lasciato niente sul disco"
    );
}

#[test]
fn a_scan_that_fails_after_the_words_does_not_ask_for_them_again() {
    in_italian();
    let temp = Temp::new("scansione-ripresa");
    let ceremony = temp.ceremony();
    let wallet = test_wallet(SEED12);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new();
    for _ in 0..plebline::online::SCAN_ATTEMPTS {
        chain = chain.scan_fails("tempo scaduto");
    }
    let mut chain = chain.with_scan(scanned);

    let mut ask = up_to_the_server([
        Reply::like("Riprovo adesso"),
        Reply::like("Chiudi e cancella"),
    ]);
    run_in(&ceremony, &mut ask, &mut chain).expect("uscire non è un errore");

    assert_script_used_up(&ask);
    let seeds = ask.asked().iter().filter(|a| a.kind == Kind::Seed).count();
    assert_eq!(seeds, 1, "le parole si scrivono una volta sola");
    // New refills are offered only once the coins have been read.
    assert!(
        ask.asked()
            .iter()
            .any(|a| a.options.iter().any(|o| o == "Nuove ricariche")),
        "e le monete sono state lette"
    );
}

/// The guide is chosen, and the user still has to roll the dice.
#[test]
fn not_ready_in_the_guide_stops_the_ceremony_without_writing_anything() {
    in_italian();
    let temp = Temp::new("non-pronto");
    let ceremony = temp.ceremony();
    let mut chain = plebline::chain::Fake::new();

    let mut ask = from_the_opening([
        Reply::like("Non ce le ho"),
        Reply::like("carte"),
        Reply::No,
        Reply::No,
    ]);

    run_in(&ceremony, &mut ask, &mut chain).expect("fermarsi qui non è un errore");

    assert_script_used_up(&ask);
    assert!(
        !ceremony.has(project::WALLET),
        "nessun portafoglio: non c'erano parole da cui farlo"
    );
    assert!(
        chain.servers().is_empty(),
        "e nessuno ha parlato con la rete"
    );
}
