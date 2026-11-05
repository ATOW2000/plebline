//! What stays on disk, and what goes.

use super::*;

/// After the ceremony `wallet.json` is gone and the payments stay: only a
/// "yes" deletes them.
#[test]
fn when_the_ceremony_ends_the_wallet_is_forgotten_and_the_payments_stay() {
    in_italian();
    let temp = Temp::new("dimentica");
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
        // No copy: "delete anyway?" no.
        Reply::No,
    ]);

    assert!(ceremony.has(project::WALLET), "in partenza c'era");
    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert!(
        !ceremony.has(project::WALLET),
        "wallet.json deve sparire: diceva tutti gli indirizzi e i saldi"
    );
    assert!(
        ceremony.has(project::BATCH),
        "i pagamenti firmati restano: si cancellano solo col sì"
    );
}

/// Without `wallet.json` the next ceremony rebuilds it from the words and
/// goes on.
#[test]
fn without_the_wallet_the_next_ceremony_asks_for_the_words_and_goes_on() {
    in_italian();
    let temp = Temp::new("riparte");
    let ceremony = temp.ceremony();
    let wallet = test_wallet(SEED12);

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    assert!(!ceremony.has(project::WALLET));

    // No descriptors on disk: they come from the words.
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

    run_in(&ceremony, &mut ask, &mut chain).expect("riparte senza il wallet");
    assert!(ceremony.has(project::BATCH), "ha firmato lo stesso");
    assert!(!ceremony.has(project::WALLET), "e se lo è ridimenticato");
}

/// Last time's groups, sent or not: the opening asks the chain nothing
/// about them and asks nothing about deleting. They are set aside, kept,
/// and no longer read.
#[test]
fn reopening_sets_last_times_refills_aside_and_asks_nothing() {
    in_italian();
    let temp = Temp::new("riapre-da-zero");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    write_batch(&ceremony, &[0, 1]);
    write_group(&ceremony, "batch-1.json", &[2]);

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    // At the wallet: stop there.
    let mut ask = up_to_the_server([Reply::Cancelled]);
    let error = run_in(&ceremony, &mut ask, &mut chain).expect_err("fermata al portafoglio");

    assert!(matches!(error, Error::Cancelled), "{error}");
    assert_script_used_up(&ask);
    assert_eq!(
        ask.prompts().last(),
        Some(&"Cosa facciamo?"),
        "dritti al portafoglio: {:?}",
        ask.prompts()
    );
    assert_eq!(
        chain.servers().len(),
        1,
        "alla catena si chiedono solo le monete: {:?}",
        chain.servers()
    );
    assert!(
        project::signed_files(&ceremony).is_empty(),
        "nessun gruppo vecchio resta in vista"
    );
    assert_eq!(
        project::set_aside_files(&ceremony).len(),
        2,
        "ma sono tutti tenuti"
    );
}

/// Coins seen a moment ago are read again: nothing is resumed.
#[test]
fn reopening_reads_the_coins_again_however_recent() {
    in_italian();
    let temp = Temp::new("rilegge-sempre");
    let ceremony = temp.ceremony();
    snapshot_of_two(&ceremony);
    let wallet = test_wallet(SEED12);

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut ask = up_to_the_server([Reply::Cancelled]);
    let error = run_in(&ceremony, &mut ask, &mut chain).expect_err("fermata al portafoglio");

    assert!(matches!(error, Error::Cancelled), "{error}");
    assert_script_used_up(&ask);
    assert_eq!(chain.servers().len(), 1, "le monete si rileggono");
}

/// With payments on the stick, the working directory goes, unasked.
#[test]
fn with_the_payments_safe_the_folder_disappears() {
    in_italian();
    let temp = Temp::new("sparisce");
    // The stick must not be inside the ceremony directory.
    let ceremony = Project {
        name: "prova".to_string(),
        dir: temp.path.join("cerimonia"),
    };
    std::fs::create_dir_all(&ceremony.dir).expect("cartella creabile");
    let wallet = write_wallet(&ceremony);
    let stick = temp.path.join("chiavetta");
    std::fs::create_dir_all(&stick).expect("cartella creabile");

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));

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
        Reply::like("hex"),
        Reply::number(3usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_eq!(
        ask.remaining(),
        0,
        "con la copia fatta non si chiede niente: si cancella e basta"
    );
    for file in project::FILES {
        assert!(!ceremony.has(file), "{file} doveva sparire");
    }
    assert!(
        !ceremony.dir.exists(),
        "e la cartella con lei: {}",
        ceremony.dir.display()
    );
    assert!(
        stick.join("plebline-ricariche/batch.json").exists(),
        "i pagamenti però ci sono, sulla chiavetta"
    );

    // The cost of forgetting must be said: unsent payments can be re-signed.
    let said = notebook.text();
    assert!(
        said.contains("verrà rifiutata per sempre"),
        "l'avviso sulle monete ancora impegnate non c'è: {said}"
    );
}

/// All payments saved on the phone count as taken away: the working
/// directory goes, and the receive addresses are shown as QR codes.
#[test]
fn with_the_payments_on_the_phone_the_folder_disappears() {
    in_italian();
    let temp = Temp::new("sparisce-telefono");
    let ceremony = Project {
        name: "prova".to_string(),
        dir: temp.path.join("cerimonia"),
    };
    std::fs::create_dir_all(&ceremony.dir).expect("cartella creabile");
    let wallet = write_wallet(&ceremony);

    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1))
        .with(coin(&wallet, 1, 500_000, 2));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));

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
        Reply::like("Sul telefono"),
        Reply::like("hex"),
        // Saved, and all of them.
        Reply::Yes,
        Reply::Yes,
        Reply::number(3usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_eq!(ask.remaining(), 0);
    assert!(!ceremony.dir.exists(), "la cartella doveva sparire");
    let qr = notebook
        .lines()
        .into_iter()
        .filter(|l| matches!(l.kind, plebline::out::Kind::Mono(plebline::out::Mono::Qr)))
        .count();
    assert_eq!(qr, 1 + 3, "un pagamento e tre indirizzi");
}

#[test]
fn without_a_copy_the_payments_are_not_deleted_without_asking() {
    in_italian();
    let temp = Temp::new("senza-copia");
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
        // No stick.
        Reply::text(""),
        Reply::number(3usize),
        Reply::No,
        // Back at the wallet: close.
        Reply::like("Chiudi e cancella"),
        // "Delete anyway?": no.
        Reply::No,
    ]);

    run_to_the_end(&ceremony, &mut ask, &mut chain);

    assert!(
        ceremony.has(project::BATCH),
        "i pagamenti non copiati non si cancellano"
    );
    assert!(
        !ceremony.has(project::WALLET),
        "la parte pubblica se ne va lo stesso: non è un pagamento da salvare"
    );
}
