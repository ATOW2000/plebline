//! Network check, chain snapshot, server choice, and unplugging.

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

use plebline::ask::{Ask, Question};
use plebline::chain::{Chain, Reachability, RouteProof};
use plebline::project::{self, Project};
use plebline::{online, ui};

use crate::NETWORK;

/// `None` if the user gives up after the network failed: the words stay
/// unwritten, and are typed again next time.
pub(crate) fn step_chain(
    ceremony: &Project,
    known: Option<String>,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<Option<String>> {
    ui::title(t("Guardo cosa hai", "Let me see what you have"));
    ui::note(t(
        "Chiedo alla rete quali monete risultano tue. Da qui non si può spendere niente: \
         sto usando solo la parte pubblica del wallet.",
        "I ask the network which coins turn out to be yours. Nothing can be spent from here: \
         I am only using the wallet's public half.",
    ));

    let mut server = match known {
        Some(server) => server,
        None => ask_server(ask)?,
    };

    // The words are in memory, so a network failure leads to a question
    // instead of ending the ceremony, which would mean typing them again.
    loop {
        let outcome = online::export_utxos(
            NETWORK,
            &server,
            online::ExportInput {
                wallet: ceremony.path(project::WALLET),
                out: ceremony.path(project::UTXOS),
            },
            chain,
        );
        let problem = match outcome {
            Ok(_) => return Ok(Some(server)),
            Err(problem) if road_problem(&problem) => problem,
            Err(other) => return Err(other),
        };

        ui::error_with_remedy(&problem.to_string(), remedy(&problem).as_deref());
        match retry(ask)? {
            Retry::SameServer => {}
            Retry::AnotherServer => server = another(&server, ask)?,
            Retry::GiveUp => {
                ui::blank();
                ui::note(t(
                    "Mi fermo qui. Le tue parole non le ho scritte da nessuna parte: quando la \
                     rete risponde, riapri il programma e si riparte, parole comprese.",
                    "I stop here. I have written your words nowhere: when the network answers, \
                     open the program again and we start over, words included.",
                ));
                return Ok(None);
            }
        }
    }
}

/// Worth asking "try again?": the server or the connection to it failed.
/// Tor itself may come back too, so a dead Tor is asked about here, unlike
/// in [`online::worth_retrying`] where nobody waits.
fn road_problem(problem: &Error) -> bool {
    matches!(problem, Error::Network(_) | Error::TorDown { .. })
}

/// After a failed scan: the next public server, or the user's node.
fn another(current: &str, ask: &mut dyn Ask) -> Result<String> {
    Ok(match ask_whom(ask)? {
        ServerChoice::Own(url) => url,
        ServerChoice::Public => {
            let list = online::default_apis(NETWORK)?;
            let next = list
                .iter()
                .position(|server| *server == current)
                .map_or(0, |index| (index + 1) % list.len());
            ui::note(t(
                "Anche questo server vedrà quali indirizzi sono tuoi.",
                "This server too will see which addresses are yours.",
            ));
            list[next].to_string()
        }
    })
}

/// Reachability check, before the words: finding Tor down after typing 24
/// words means typing them again. Asks for the tip height, which only a
/// reached server knows, shown as proof; reveals no wallet data. `None` if
/// the user gives up: nothing written, nothing typed.
pub(crate) fn step_network(ask: &mut dyn Ask, chain: &mut dyn Chain) -> Result<Option<String>> {
    ui::title(t(
        "Guardo che la rete ci sia",
        "Let me check the network is there",
    ));
    ui::note(t(
        "Fra un momento ti chiedo le tue parole, e subito dopo guardo sulla catena quali \
         monete sono tue. Prima però provo se la rete risponde davvero: se non risponde, è \
         meglio saperlo adesso che dopo aver scritto ventiquattro parole.",
        "In a moment I will ask you for your words, and right after that I will look at the \
         chain to see which coins are yours. First, though, I check that the network really \
         answers: if it does not, better to find out now than after typing twenty-four words.",
    ));

    // The choice is kept across retries.
    let mut chosen: Option<ServerChoice> = None;
    // Attempts on this choice: only this loop can tell a retry from a fresh
    // start.
    let mut attempt = 0usize;

    loop {
        let whom = match chosen.take() {
            Some(whom) => whom,
            None => {
                // A different choice restarts the count.
                attempt = 0;
                ask_whom(ask)?
            }
        };
        attempt += 1;
        let candidates: Vec<String> = match &whom {
            ServerChoice::Public => online::default_apis(NETWORK)?
                .iter()
                .map(|server| server.to_string())
                .collect(),
            ServerChoice::Own(url) => vec![url.clone()],
        };

        // Status line before the work. From the second try on the same server
        // it replaces the failed block and shows the attempt count.
        let contacting = t("Contatto la rete…", "Contacting the network…");
        if attempt == 1 {
            ui::working(contacting);
        } else {
            ui::retrying(&with_attempt(contacting, attempt));
        }

        match first_answering(&candidates, chain) {
            Ok((server, reached)) => {
                let height = reached.height;
                ui::done(&tf!({ "Contattato. L'ultimo blocco è il {height}." }, {
                    "Contacted. The last block is {height}."
                }));
                say_proof(&reached.proof);
                crate::road::connected(match reached.proof {
                    RouteProof::OwnNode => plebline::out::Link::OwnNode,
                    _ => plebline::out::Link::Tor,
                });
                return Ok(Some(server));
            }
            Err(problem) => {
                ui::stopped(&with_attempt(t("Non risponde.", "No answer."), attempt));
                ui::error_with_remedy(&problem.to_string(), remedy(&problem).as_deref());
                match retry(ask)? {
                    Retry::SameServer => chosen = Some(whom),
                    Retry::AnotherServer => chosen = None,
                    Retry::GiveUp => {
                        ui::blank();
                        ui::note(t(
                            "Non ho scritto niente e non ti ho chiesto niente: quando la rete \
                             c'è, riapri il programma e si riparte da qui.",
                            "I have written nothing and asked you for nothing: when the network \
                             is there, open the program again and we start from here.",
                        ));
                        return Ok(None);
                    }
                }
            }
        }
    }
}

/// The first server that answers, in order. Only a server's own failure
/// moves on to the next: a dead Tor is the same for all of them.
fn first_answering(candidates: &[String], chain: &mut dyn Chain) -> Result<(String, Reachability)> {
    let mut last = None;
    for server in candidates {
        if let Some(problem) = &last {
            ui::trace(&tf!(
                { "non risponde ({problem}): passo al prossimo server" },
                { "no answer ({problem}): moving on to the next server" }
            ));
        }
        match chain.reach(server) {
            Ok(reached) => return Ok((server.clone(), reached)),
            Err(problem) if online::worth_retrying(&problem) => last = Some(problem),
            Err(problem) => return Err(problem),
        }
    }
    Err(last.unwrap_or_else(|| {
        Error::Network(t("nessun server da interrogare", "no server to ask").to_string())
    }))
}

/// Adds the attempt count from the second attempt on.
fn with_attempt(text: &str, attempt: usize) -> String {
    if attempt < 2 {
        return text.to_string();
    }
    tf!({ "{text} ({attempt}° tentativo)" }, {
        "{text} (attempt {attempt})"
    })
}

/// One sentence per evidence strength; what was done is already in the
/// trace.
fn say_proof(proof: &RouteProof) {
    match proof {
        RouteProof::Onion => ui::note(t(
            "Un indirizzo .onion non ha un IP e non sta in nessun DNS: lo raggiunge solo Tor. \
             La risposta arrivata è quindi la prova, e non è passata da nessun nodo di uscita.",
            "An onion address has no IP and is in no DNS: only Tor reaches it. So the answer \
             that came back is the proof, and it went through no exit node.",
        )),
        RouteProof::Exit { ip } => ui::note(&tf!(
            {
                "Con un server in chiaro la risposta da sola non dimostra che la richiesta è \
              passata da Tor: in mezzo c'è un nodo di uscita, e solo Tor può dire quale. \
              Gliel'ho chiesto: è il {ip}."
            },
            {
                "With a clearnet server the answer alone does not prove the request went \
              through Tor: there is an exit node in between, and only Tor can say which. I \
              asked it: it is {ip}."
            }
        )),
        // A missing confirmation is a warning, so it is not read as a pass.
        RouteProof::Unconfirmed { why } => ui::warning(&tf!(
            {
                "La conferma da Tor non è arrivata: {why}. La richiesta è passata comunque dal \
              proxy, ma qui non te lo sto dimostrando."
            },
            {
                "No confirmation came back from Tor: {why}. The request did go through the \
              proxy, but this is not proving it to you."
            }
        )),
        // Already said by the route.
        RouteProof::OwnNode => {}
    }
}

enum Retry {
    SameServer,
    AnotherServer,
    GiveUp,
}

fn retry(ask: &mut dyn Ask) -> Result<Retry> {
    let again = t("Riprovo adesso", "Try again now");
    let other = t("Chiedo a un altro", "Ask somebody else");
    let stop = t("Lascio stare per ora", "Leave it for now");

    let index = ask.select(
        Question::new(t("Cosa faccio?", "What do I do?")).help(t(
            "Se Tor si è appena acceso, dagli qualche secondo e riprova.",
            "If Tor has just started, give it a few seconds and try again.",
        )),
        &[again, other, stop],
        None,
    )?;

    Ok(match index {
        0 => Retry::SameServer,
        1 => Retry::AnotherServer,
        _ => Retry::GiveUp,
    })
}

/// Which server to ask: the public ones in turn, or one's own node.
#[derive(Debug, Clone)]
enum ServerChoice {
    Public,
    Own(String),
}

/// [`ask_whom`], settled on one server: the first public one.
pub(crate) fn ask_server(ask: &mut dyn Ask) -> Result<String> {
    Ok(match ask_whom(ask)? {
        ServerChoice::Public => online::default_api(NETWORK)?.to_string(),
        ServerChoice::Own(url) => url,
    })
}

/// The one choice that involves a third party.
fn ask_whom(ask: &mut dyn Ask) -> Result<ServerChoice> {
    ui::blank();
    ui::note(t(
        "Il server a cui chiedo vede quali indirizzi sono tuoi, e quindi quanto hai. Non può \
         spendere le tue monete, ma vede tutto questo.",
        "The server I ask sees which addresses are yours, and so how much you have. It cannot \
         spend your coins, but it does see all of that.",
    ));

    let public_server = t(
        "Un server pubblico: non devi installare niente, ma vede i tuoi indirizzi",
        "A public server: nothing to install, but it sees your addresses",
    );
    let own_node = t(
        "Il mio nodo: nessun altro vede i tuoi indirizzi, ma devi averne uno acceso",
        "My own node: nobody else sees your addresses, but you need one running",
    );

    let index = ask.select(
        Question::new(t("A chi chiedo?", "Whom do I ask?")),
        &[public_server, own_node],
        None,
    )?;

    if index == 0 {
        return Ok(ServerChoice::Public);
    }

    let url = ask.text(
        Question::new(t("Indirizzo del tuo nodo:", "Your node's address:"))
            .help(t("es. http://localhost:3002", "e.g. http://localhost:3002")),
        None,
    )?;
    Ok(ServerChoice::Own(url.trim().to_string()))
}

/// The remedy for the network errors, in this path's terms: the question
/// that picks a node of one's own, and on the Mac the app that starts Tor.
/// Everything else keeps the core's.
pub(crate) fn remedy(problem: &Error) -> Option<String> {
    let own_node = t(
        "Se hai un nodo tuo sulla rete di casa, alla domanda «A chi chiedo?» scegli «Il mio \
         nodo»: non passa da nessun estraneo, e Tor non serve.",
        "If you have a node of your own on your home network, at «Whom do I ask?» choose «My \
         own node»: it goes through no stranger, and needs no Tor.",
    );
    let road = match problem {
        Error::Network(_) => t(
            "Non è un problema delle tue monete: quello che è già scritto resta valido. Di \
             solito basta riprovare fra poco, e ogni tentativo usa un circuito nuovo dentro \
             Tor. Se quel server continua a non rispondere, chiedi a un altro.",
            "This is not a problem with your coins: what is already written stays valid. Trying \
             again in a little while is usually enough, and every attempt uses a new circuit \
             inside Tor. If that server keeps not answering, ask another one.",
        ),
        Error::TorDown { .. } | Error::Clearnet { .. } if cfg!(target_os = "macos") => t(
            "Tor non risponde. Apri il programma dal suo pacchetto, che accende Tor da solo.",
            "Tor is not answering. Open the program from its package, which starts Tor by itself.",
        ),
        Error::TorDown { .. } | Error::Clearnet { .. } => t(
            "Tor non risponde. Su Tails è del sistema: se la macchina si è appena accesa sta \
             ancora costruendo i suoi collegamenti, aspetta che Tails dica di essere connesso a \
             Tor e riprova.",
            "Tor is not answering. On Tails it belongs to the system: if the machine has just \
             booted it is still building its connections, so wait until Tails says it is \
             connected to Tor and try again.",
        ),
        Error::NotTor { .. } => t(
            "Dove dovrebbe esserci Tor risponde qualcos'altro: da lì non mando niente, perché \
             vedrebbe da dove arrivano le domande. Su Tails non deve succedere: riavvia la \
             macchina.",
            "Something other than Tor answers where Tor should be: I send nothing through it, \
             because it would see where the questions come from. On Tails this must not \
             happen: restart the machine.",
        ),
        other => return other.remedy().map(str::to_string),
    };
    // A busy server is solved by another public one; a broken Tor only by
    // not needing it (the user's own node).
    if matches!(problem, Error::Network(_)) {
        return Some(road.to_string());
    }
    Some(format!("{road} {own_node}"))
}

/// No network is needed from here on, and an unplugged machine is a real
/// airgap. `split_first` changes what is said about plugging back in, so
/// the text does not contradict itself two lines later.
pub(crate) fn step_unplug(split_first: bool, ask: &mut dyn Ask) -> Result<()> {
    ui::blank();
    // A `Kind::Alert`: framing is up to the sink.
    ui::alert(t(
        "DA QUI IN POI LA RETE NON SERVE PIÙ.",
        "FROM HERE ON THE NETWORK IS NO LONGER NEEDED.",
    ));
    ui::blank();
    ui::note(t(
        "Ho tutto quello che mi serve, ed è scritto qui sul disco: quante monete hai, quanto \
         valgono, da dove vengono. Tutto quello che viene dopo lo leggo da lì.",
        "I have everything I need, and it is written here on the disk: how many coins you have, \
         what they are worth, where they come from. Everything that follows I read from there.",
    ));
    ui::note(t(
        "Da adesso si lavora con le tue parole, e nessuno dei passi che restano ha bisogno \
         della rete: per questo è il momento giusto per staccare.",
        "From here on we work with your words, and none of the remaining steps needs the \
         network: that is why this is the right moment to unplug.",
    ));
    ui::blank();
    ui::info(t(
        "Stacca il cavo di rete, o spegni il Wi-Fi. Adesso.",
        "Unplug the network cable, or turn off Wi-Fi. Now.",
    ));
    ui::note(t(
        "Non è obbligatorio, ed è la cosa più utile che puoi fare in tutta la cerimonia: una \
         macchina senza rete non fa uscire niente, nemmeno se il programma avesse un difetto.",
        "It is not compulsory, and it is the most useful thing you can do in the whole ceremony: \
         a machine with no network lets nothing out, not even if the program had a flaw.",
    ));
    ui::blank();
    if split_first {
        ui::info(t(
            "Poi ti chiederò di riattaccare una volta sola, per mandare in rete la divisione: \
             a quel punto le tue parole non serviranno più e le avrò già tolte dalla memoria.",
            "Then I will ask you to plug back in once, to send the split to the network: by then \
             your words will no longer be needed and I will already have removed them from memory.",
        ));
    } else {
        ui::info(t(
            "Resterà staccato fino alla fine. Riattaccherai solo quando vorrai spedire una \
             ricarica, e per spedire le tue parole non servono.",
            "It stays unplugged until the end. You will plug back in only when you want to send a \
             refill, and sending does not need your words.",
        ));
    }
    ui::blank();

    // An answer is required, so the user does it instead of only reading it.
    ask.text(
        Question::new(t(
            "Scrivi «fatto» quando hai staccato (o Invio per saltare):",
            "Type \"done\" once you have unplugged (or Enter to skip):",
        )),
        None,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};

    fn public() -> Vec<String> {
        online::default_apis(NETWORK)
            .expect("mainnet")
            .iter()
            .map(|server| server.to_string())
            .collect()
    }

    /// A ceremony with `wallet.json` and the chain that will show one coin.
    fn one_coin(temp: &Temp) -> (Project, plebline::chain::ScanResult) {
        let ceremony = temp.ceremony();
        let wallet = write_wallet(&ceremony);
        let scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet))
            .with(coin(&wallet, 0, 500_000, 1));
        (ceremony, scanned)
    }

    /// The next public server takes over with no question, under the same
    /// status line.
    #[test]
    fn a_public_server_that_is_down_hands_over_to_the_next() {
        use plebline::out::{Kind, Status};

        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let list = public();
        let mut chain = plebline::chain::Fake::new()
            .with_height(800_000)
            .probe_fails_on(list[0].clone(), "risposta vuota");
        let mut ask = Scripted::new([Reply::like("Un server pubblico")]);

        let server = step_network(&mut ask, &mut chain)
            .expect("il controllo passa")
            .expect("e non si rinuncia");
        plebline::out::to(Box::new(plebline::out::Terminal));

        assert_eq!(server, list[1], "risponde il secondo, e si usa quello");
        assert_eq!(chain.probes(), &list[..2]);
        assert_eq!(ask.remaining(), 0, "nessuna domanda in più");
        let lights: Vec<Kind> = notebook
            .lines()
            .into_iter()
            .map(|line| line.kind)
            .filter(|kind| matches!(kind, Kind::Status(_)))
            .collect();
        assert_eq!(
            lights,
            vec![Kind::Status(Status::Working), Kind::Status(Status::Done)],
            "un tentativo solo, anche se i server sono stati due"
        );
        assert!(
            notebook.text().contains("passo al prossimo server"),
            "{}",
            notebook.text()
        );
    }

    /// A dead Tor is the same for every server: trying them all only wastes
    /// time. No network: the proxy is a closed local port.
    #[test]
    fn a_dead_tor_does_not_go_through_every_server() {
        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Esplora::through(Some("socks5h://127.0.0.1:1".into()));
        let outcome = first_answering(&public(), &mut chain);
        plebline::out::to(Box::new(plebline::out::Terminal));

        assert!(matches!(outcome, Err(Error::TorDown { .. })), "{outcome:?}");
        let knocks = notebook
            .lines()
            .into_iter()
            .filter(|line| line.text.contains("127.0.0.1:1"))
            .count();
        assert_eq!(knocks, 1, "ha bussato una volta sola");
    }

    /// One timeout does not end the ceremony and lose the typed words: the
    /// scan tries again by itself.
    #[test]
    fn a_scan_that_fails_once_is_retried_without_asking() {
        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let temp = Temp::new("scan-retry");
        let (ceremony, scanned) = one_coin(&temp);
        let mut chain = plebline::chain::Fake::new()
            .scan_fails("tempo scaduto")
            .with_scan(scanned);
        let mut ask = Scripted::new([]);
        let server = public()[0].clone();

        let outcome = step_chain(&ceremony, Some(server.clone()), &mut ask, &mut chain);
        plebline::out::to(Box::new(plebline::out::Terminal));

        assert_eq!(outcome.expect("la lettura riesce"), Some(server.clone()));
        assert!(ceremony.has(project::UTXOS), "e l'elenco è scritto");
        assert_eq!(chain.servers(), [server.clone(), server], "due tentativi");
        assert!(
            notebook.text().contains("Riprovo da solo"),
            "{}",
            notebook.text()
        );
    }

    /// When the automatic attempts are used up, the user is asked: the
    /// words are in memory and the ceremony does not end.
    #[test]
    fn a_scan_that_keeps_failing_asks_and_can_go_on() {
        in_italian();
        let temp = Temp::new("scan-ask");
        let (ceremony, scanned) = one_coin(&temp);
        let mut chain = plebline::chain::Fake::new();
        for _ in 0..online::SCAN_ATTEMPTS {
            chain = chain.scan_fails("tempo scaduto");
        }
        let mut chain = chain.with_scan(scanned);
        let mut ask = Scripted::new([Reply::like("Riprovo adesso")]);

        let outcome = step_chain(&ceremony, Some(public()[0].clone()), &mut ask, &mut chain);

        assert!(outcome.expect("non è un errore").is_some());
        assert_eq!(ask.remaining(), 0);
        assert_eq!(chain.servers().len(), online::SCAN_ATTEMPTS + 1);
        assert!(ceremony.has(project::UTXOS));
    }

    #[test]
    fn giving_up_after_a_failed_scan_writes_no_list() {
        in_italian();
        let temp = Temp::new("scan-give-up");
        let (ceremony, scanned) = one_coin(&temp);
        let mut chain = plebline::chain::Fake::new();
        for _ in 0..online::SCAN_ATTEMPTS {
            chain = chain.scan_fails("tempo scaduto");
        }
        let mut chain = chain.with_scan(scanned);
        let mut ask = Scripted::new([Reply::like("Lascio stare")]);

        let outcome = step_chain(&ceremony, Some(public()[0].clone()), &mut ask, &mut chain);

        assert!(outcome.expect("rinunciare non è un errore").is_none());
        assert!(!ceremony.has(project::UTXOS));
        assert_eq!(chain.remaining(), 1, "la fotografia non è stata chiesta");
    }

    /// "Ask somebody else" after a failed scan, public again: the next one
    /// on the list, not the same.
    #[test]
    fn another_public_server_after_a_failed_scan_is_the_next_one() {
        in_italian();
        let temp = Temp::new("scan-other");
        let (ceremony, scanned) = one_coin(&temp);
        let mut chain = plebline::chain::Fake::new();
        for _ in 0..online::SCAN_ATTEMPTS {
            chain = chain.scan_fails("tempo scaduto");
        }
        let mut chain = chain.with_scan(scanned);
        let mut ask = Scripted::new([
            Reply::like("Chiedo a un altro"),
            Reply::like("Un server pubblico"),
        ]);
        let list = public();

        let outcome = step_chain(&ceremony, Some(list[0].clone()), &mut ask, &mut chain);

        assert_eq!(outcome.expect("riesce"), Some(list[1].clone()));
        assert_eq!(chain.servers().last(), Some(&list[1]));
    }

    /// The guided path has no `--api` and no `sh avvia`.
    #[test]
    fn the_network_remedies_point_at_nothing_that_is_not_here() {
        for lang in [plebline_core::lang::Lang::It, plebline_core::lang::Lang::En] {
            plebline_core::lang::set_for_this_thread(lang);
            for problem in [
                Error::Network("x".into()),
                Error::TorDown {
                    host: "h".into(),
                    proxy: "p".into(),
                },
                Error::Clearnet { host: "h".into() },
                Error::NotTor { proxy: "p".into() },
            ] {
                let text = remedy(&problem).expect("un rimedio c'è");
                assert!(!text.contains("--"), "{text}");
                assert!(!text.contains("sh avvia"), "{text}");
            }
        }
        in_italian();
        let dead_tor = remedy(&Error::TorDown {
            host: "h".into(),
            proxy: "p".into(),
        })
        .expect("un rimedio c'è");
        assert!(dead_tor.contains("«Il mio nodo»"), "{dead_tor}");
    }

    #[test]
    fn the_public_server_asks_for_no_address() {
        in_italian();
        let mut ask = Scripted::new([Reply::like("Un server pubblico")]);
        let server = ask_server(&mut ask).expect("server");
        assert_eq!(server, online::default_api(NETWORK).expect("mainnet"));
        assert_eq!(ask.remaining(), 0, "non deve chiedere altro");
    }

    #[test]
    fn your_own_node_asks_for_the_address() {
        in_italian();
        let mut ask = Scripted::new([
            Reply::like("Il mio nodo"),
            Reply::text("  http://umbrel.local:3002  "),
        ]);
        assert_eq!(
            ask_server(&mut ask).expect("server"),
            "http://umbrel.local:3002"
        );
    }

    /// The chain height is shown as proof that the server answered.
    #[test]
    fn the_check_says_on_screen_what_the_chain_answered() {
        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new().with_height(321_192);
        let mut ask = Scripted::new([Reply::like("Un server pubblico")]);

        let server = step_network(&mut ask, &mut chain)
            .expect("il controllo passa")
            .expect("e non si rinuncia");

        plebline::out::to(Box::new(plebline::out::Terminal));

        assert_eq!(
            chain.probes(),
            std::slice::from_ref(&server),
            "ha interrogato il server"
        );
        assert!(
            chain.servers().is_empty(),
            "e non gli ha dato il portafoglio: quello viene dopo"
        );
        let screen = notebook.text();
        assert!(
            screen.contains("321192"),
            "l'ultimo blocco deve essere a schermo: {screen}"
        );
    }

    /// Clearnet: Tor's answer and exit node are shown.
    #[test]
    fn a_clearnet_server_shows_what_tor_answered() {
        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new()
            .with_height(800_000)
            .with_proof(RouteProof::Exit {
                ip: "185.220.101.1".to_string(),
            });
        let mut ask = Scripted::new([Reply::like("Un server pubblico")]);

        step_network(&mut ask, &mut chain).expect("il controllo passa");
        plebline::out::to(Box::new(plebline::out::Terminal));

        let screen = notebook.text();
        assert!(
            screen.contains("185.220.101.1"),
            "il nodo di uscita deve essere a schermo: {screen}"
        );
    }

    /// Checks the order of the status lines, not their wording.
    #[test]
    fn the_status_shows_working_before_the_work_and_done_after() {
        use plebline::out::{Kind, Status};

        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new().with_height(321_387);
        let mut ask = Scripted::new([Reply::like("Un server pubblico")]);

        step_network(&mut ask, &mut chain).expect("il controllo passa");
        plebline::out::to(Box::new(plebline::out::Terminal));

        let lights: Vec<Kind> = notebook
            .lines()
            .into_iter()
            .map(|line| line.kind)
            .filter(|kind| matches!(kind, Kind::Status(_)))
            .collect();
        assert_eq!(
            lights,
            vec![Kind::Status(Status::Working), Kind::Status(Status::Done)],
            "prima rossa mentre lavora, poi verde: e in quest'ordine"
        );
    }

    #[test]
    fn a_network_that_does_not_answer_ends_the_status_as_stopped() {
        use plebline::out::{Kind, Status};

        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new().probe_fails("Tor non risponde");
        let mut ask = Scripted::new([
            Reply::like("Un server pubblico"),
            Reply::like("Lascio stare"),
        ]);

        let given_up = step_network(&mut ask, &mut chain).expect("rinunciare non è un errore");
        plebline::out::to(Box::new(plebline::out::Terminal));

        assert!(
            given_up.is_none(),
            "chi lascia stare non porta via un server"
        );
        let lights: Vec<Kind> = notebook
            .lines()
            .into_iter()
            .map(|line| line.kind)
            .filter(|kind| matches!(kind, Kind::Status(_)))
            .collect();
        assert_eq!(
            lights,
            vec![Kind::Status(Status::Working), Kind::Status(Status::Stopped)],
            "la spia si spegne anche quando va male"
        );
    }

    #[test]
    fn a_missing_tor_confirmation_is_not_shown_as_proof() {
        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new()
            .with_height(800_000)
            .with_proof(RouteProof::Unconfirmed {
                why: "connessione rifiutata".to_string(),
            });
        let mut ask = Scripted::new([Reply::like("Un server pubblico")]);

        step_network(&mut ask, &mut chain).expect("non è un errore: si va avanti");
        plebline::out::to(Box::new(plebline::out::Terminal));

        let screen = notebook.text();
        assert!(screen.contains("connessione rifiutata"), "{screen}");
        assert!(
            screen.contains("non te lo sto dimostrando"),
            "deve dire che non è una prova: {screen}"
        );
    }

    /// Unreachable offers a retry that keeps the chosen server.
    #[test]
    fn trying_again_does_not_ask_whom_to_ask_a_second_time() {
        in_italian();
        let mut chain = plebline::chain::Fake::new().probe_fails("Tor non risponde");
        let mut ask = Scripted::new([
            Reply::like("Un server pubblico"),
            Reply::like("Riprovo adesso"),
            Reply::like("Lascio stare"),
        ]);

        let given_up = step_network(&mut ask, &mut chain).expect("rinunciare non è un errore");

        assert!(
            given_up.is_none(),
            "chi rinuncia non porta a casa un server"
        );
        assert_script_used_up(&ask);
        let public = online::default_apis(NETWORK).expect("mainnet").len();
        assert_eq!(
            chain.probes().len(),
            2 * public,
            "ha provato due volte, ogni volta tutti i server pubblici"
        );
        let asked_whom = ask
            .prompts()
            .iter()
            .filter(|d| d.contains("A chi chiedo"))
            .count();
        assert_eq!(asked_whom, 1, "la domanda sul server si fa una volta sola");
    }

    /// Retrying the same server says so, with the count.
    #[test]
    fn trying_the_same_server_again_is_declared_a_retry_and_counted() {
        use plebline::out::{Kind, Status};

        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new().probe_fails("Tor non risponde");
        let mut ask = Scripted::new([
            Reply::like("Un server pubblico"),
            Reply::like("Riprovo adesso"),
            Reply::like("Riprovo adesso"),
            Reply::like("Lascio stare"),
        ]);

        step_network(&mut ask, &mut chain).expect("rinunciare non è un errore");
        plebline::out::to(Box::new(plebline::out::Terminal));

        let lights: Vec<(Kind, String)> = notebook
            .lines()
            .into_iter()
            .filter(|line| matches!(line.kind, Kind::Status(_)))
            .map(|line| (line.kind, line.text))
            .collect();

        let kinds: Vec<Kind> = lights.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Status(Status::Working),
                Kind::Status(Status::Stopped),
                Kind::Status(Status::Retrying),
                Kind::Status(Status::Stopped),
                Kind::Status(Status::Retrying),
                Kind::Status(Status::Stopped),
            ],
            "il primo tentativo apre, i due seguenti riscrivono"
        );

        assert!(
            !lights[0].1.contains("tentativo"),
            "sul primo il conteggio sarebbe rumore: {}",
            lights[0].1
        );
        assert!(lights[2].1.contains("2°"), "{}", lights[2].1);
        assert!(lights[4].1.contains("3°"), "{}", lights[4].1);
        assert!(
            lights[5].1.contains("3°"),
            "anche la spia che si spegne porta il conto: {}",
            lights[5].1
        );
    }

    /// A different server opens a new block, count reset.
    #[test]
    fn asking_a_different_server_is_not_a_retry() {
        use plebline::out::{Kind, Status};

        in_italian();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut chain = plebline::chain::Fake::new().probe_fails("Tor non risponde");
        let mut ask = Scripted::new([
            Reply::like("Un server pubblico"),
            Reply::like("Chiedo a un altro"),
            Reply::like("Un server pubblico"),
            Reply::like("Lascio stare"),
        ]);

        step_network(&mut ask, &mut chain).expect("rinunciare non è un errore");
        plebline::out::to(Box::new(plebline::out::Terminal));

        let opened: Vec<String> = notebook
            .lines()
            .into_iter()
            .filter(|line| line.kind == Kind::Status(Status::Working))
            .map(|line| line.text)
            .collect();
        assert_eq!(opened.len(), 2, "due server, due blocchi");
        assert!(
            opened.iter().all(|text| !text.contains("tentativo")),
            "e nessuno dei due è un ritentativo: {opened:?}"
        );
    }

    #[test]
    fn asking_somebody_else_asks_the_question_again() {
        in_italian();
        let mut chain = plebline::chain::Fake::new().probe_fails("Tor non risponde");
        let mut ask = Scripted::new([
            Reply::like("Un server pubblico"),
            Reply::like("Chiedo a un altro"),
            Reply::like("Il mio nodo"),
            Reply::text("http://umbrel.local:3002"),
            Reply::like("Lascio stare"),
        ]);

        step_network(&mut ask, &mut chain).expect("rinunciare non è un errore");

        let mut expected: Vec<&str> = online::default_apis(NETWORK).expect("mainnet").to_vec();
        expected.push("http://umbrel.local:3002");
        assert_eq!(
            chain.probes(),
            expected,
            "il secondo tentativo è andato dove ha detto lui"
        );
    }
}
