//! The whole ceremony, from the words to the signing.

use super::*;

/// The complete path from the chain to the signing, with a fake chain and
/// scripted answers.
#[test]
fn the_whole_ceremony_reaches_the_signing() {
    in_italian();
    let temp = Temp::new("intera");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2))
        .with(coin(&wallet, 2, 500_000, 3));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut ask = up_to_the_server([
        Reply::like("Nuove ricariche"),
        Reply::like("Li incollo"),
        Reply::number(2usize),
        Reply::text("fatto"),
        Reply::number(2.0f64),
        Reply::text(recipient(0)),
        Reply::number(100_000u64),
        Reply::text("affitto"),
        Reply::text(recipient(1)),
        Reply::number(150_000u64),
        Reply::text("scuola"),
        Reply::Yes,
        Reply::like("Su una chiavetta"),
        Reply::text(""),
        Reply::number(3usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
        // No copy: "delete anyway?" no, files stay.
        Reply::No,
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert_script_used_up(&ask);
    assert!(
        ceremony.has(project::BATCH),
        "il gruppo firmato deve restare sul disco"
    );
    assert!(
        chain.sent().is_empty(),
        "nessuno ha detto di trasmettere: non deve essere uscito niente"
    );
}

/// The addresses come after the words and after the coins are seen, right
/// before "how many payments".
#[test]
fn the_addresses_are_asked_after_the_words_and_the_coins() {
    in_italian();
    let temp = Temp::new("ordine-indirizzi");
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
    assert_script_used_up(&ask);

    let position = |text: &str| {
        ask.prompts()
            .iter()
            .position(|p| p.contains(text))
            .unwrap_or_else(|| panic!("«{text}» non è stato chiesto: {:?}", ask.prompts()))
    };
    let words = ask
        .asked()
        .iter()
        .position(|a| a.kind == Kind::Seed)
        .expect("le parole sono state chieste");
    let coins = position("Cosa facciamo?");
    let addresses = position("Come mi dai gli indirizzi?");
    let count = position("Quante ricariche");
    assert!(words < coins, "{:?}", ask.prompts());
    assert!(coins < addresses, "{:?}", ask.prompts());
    assert!(addresses < count, "{:?}", ask.prompts());
}

/// No network after "unplug": the fake chain has one snapshot, and a second
/// look would fail the test.
#[test]
fn after_unplugging_the_network_is_never_touched_again() {
    in_italian();
    let temp = Temp::new("stacco");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);

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
        Reply::text(""),
        Reply::number(3usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
        // No copy: "delete anyway?" no, files stay.
        Reply::No,
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert_eq!(
        chain.servers().len(),
        1,
        "un viaggio solo in tutta la cerimonia: {:?}",
        chain.servers()
    );

    // The pre-seed probe and the snapshot share the one server answer.
    assert_eq!(
        chain.probes().len(),
        1,
        "una bussata sola: {:?}",
        chain.probes()
    );
    assert_eq!(
        chain.probes()[0],
        chain.servers()[0],
        "e va allo stesso server della fotografia"
    );
    assert_eq!(
        ask.prompts()
            .iter()
            .filter(|d| d.contains("A chi chiedo"))
            .count(),
        1,
        "il server si sceglie una volta sola"
    );

    let questions = ask.prompts();
    assert!(
        questions.iter().any(|d| d.contains("hai staccato")),
        "l'avviso di staccare il cavo deve essere stato dato: {questions:?}"
    );

    // The words come before unplugging (no `wallet.json` is trusted) and stay
    // in memory until signing. After "unplug", no network: one snapshot only.
    let words = ask
        .asked()
        .iter()
        .position(|a| a.kind == Kind::Seed)
        .expect("le parole devono essere state chieste");
    let unplug = questions
        .iter()
        .position(|d| d.contains("hai staccato"))
        .expect("c'è");
    assert!(
        words < unplug,
        "le parole servono per rifare la parte pubblica, quindi prima: {questions:?}"
    );
}
