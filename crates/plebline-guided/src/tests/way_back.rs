//! Cancelling: back to the step before, or to the wallet.

use super::*;

/// Cancelling the very first question stops: there is nothing before it.
#[test]
fn cancelling_the_first_question_stops() {
    in_italian();
    let temp = Temp::new("annulla-subito");
    let ceremony = temp.ceremony();
    let mut chain = plebline::chain::Fake::new();
    let mut ask = Scripted::new([Reply::Cancelled]);

    let error = run_in(&ceremony, &mut ask, &mut chain).expect_err("fermata");
    assert!(matches!(error, Error::Cancelled), "{error}");
}

/// Cancelling the words goes back one step, to the network: the server
/// is asked again, then the words, and the ceremony goes on.
#[test]
fn cancelling_the_words_goes_back_to_the_network() {
    in_italian();
    let temp = Temp::new("annulla-parole");
    let ceremony = temp.ceremony();
    let wallet = test_wallet(SEED12);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut ask = from_the_opening([
        Reply::like("Ce le ho già"),
        Reply::like("Un server pubblico"),
        Reply::like("12"),
        Reply::Cancelled,
        // Back at the network step.
        Reply::like("Un server pubblico"),
        Reply::like("12"),
        Reply::seed(SEED12),
        Reply::text(""),
        Reply::like("Chiudi e cancella"),
    ]);
    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));
    run_to_the_end(&ceremony, &mut ask, &mut chain);
    plebline::out::take();

    assert_script_used_up(&ask);
    let servers = ask
        .prompts()
        .iter()
        .filter(|p| **p == "A chi chiedo?")
        .count();
    assert_eq!(servers, 2, "di nuovo alla rete: {:?}", ask.prompts());

    let mut visited: Vec<String> = Vec::new();
    for place in notebook.places() {
        let label = place.road[place.here].label.clone();
        if visited.last() != Some(&label) {
            visited.push(label);
        }
    }
    assert_eq!(
        visited,
        ["Inizio", "Rete", "Parole", "Rete", "Parole", "Portafoglio"]
    );
}

/// Cancelling inside an operation goes back to the wallet, which says
/// so; cancelling at the wallet itself stops. The window is told which.
#[test]
fn cancelling_an_operation_goes_back_to_the_wallet() {
    in_italian();
    let temp = Temp::new("annulla-operazione");
    let ceremony = temp.ceremony();
    let wallet = write_wallet(&ceremony);
    let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
        .with(coin(&wallet, 0, 500_000, 1));
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);

    let mut ask = up_to_the_server([
        Reply::like("Nuove ricariche"),
        Reply::like("Li incollo"),
        // "How many?"
        Reply::Cancelled,
        // The wallet again: cancelling here stops.
        Reply::Cancelled,
    ]);
    let notebook = plebline::out::Recorder::new();
    plebline::out::to(Box::new(notebook.clone()));
    let error = run_in(&ceremony, &mut ask, &mut chain).expect_err("fermata al portafoglio");
    plebline::out::take();

    assert!(matches!(error, Error::Cancelled), "{error}");
    assert_script_used_up(&ask);
    assert!(
        notebook.text().contains("torno al portafoglio"),
        "{}",
        notebook.text()
    );
    assert_eq!(
        ask.prompts()
            .iter()
            .filter(|p| **p == "Cosa facciamo?")
            .count(),
        2
    );

    let ways: Vec<(String, Cancel)> = notebook
        .places()
        .iter()
        .map(|p| (p.road[p.here].label.clone(), p.cancel))
        .collect();
    let at = |label: &str| ways.iter().rev().find(|(l, _)| l == label).map(|(_, c)| *c);
    assert_eq!(at("Parole"), Some(Cancel::Back));
    assert_eq!(at("Quante"), Some(Cancel::Home));
    assert_eq!(ways.last().map(|(_, c)| *c), Some(Cancel::Stop));
}
