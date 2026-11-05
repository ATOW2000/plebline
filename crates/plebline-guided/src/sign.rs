//! Signing: how many payments, splitting the coins, verification.

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

use plebline::ask::{Ask, Question};
use plebline::chain::Chain;
use plebline::project::{self, Project};
use plebline::{online, ui, wizard};

use crate::snapshot::ask_server;
use crate::{exists, Open, NETWORK};

/// Asks how many payments. N independent payments need N distinct coins,
/// and the user is told so before answering instead of by an error after.
/// `coins`: free coins, the only ones spent. `received`: addresses already
/// sent from the phone, the suggested count.
pub(crate) fn ask_payment_count(
    coins: Option<usize>,
    received: usize,
    ask: &mut dyn Ask,
) -> Result<usize> {
    ui::title(t(
        "Quante ricariche prepariamo",
        "How many refills we prepare",
    ));

    if let Some(coins) = coins {
        ui::note(&tf!(
            {
                "Hai {coins} monete libere. Ogni ricarica prefirmata deve spenderne una \
                 diversa: è quello che fa sì che mandarne una non rovini le altre."
            },
            {
                "You have {coins} free coins. Every pre-signed refill must spend a different \
                 one: that is what makes sending one leave the others intact."
            }
        ));
        ui::note(&tf!(
            {
                "Se ne chiedi più di {coins} non è un problema: prima divido le monete, con una \
                 transazione verso te stesso. Te lo spiego se serve."
            },
            {
                "Asking for more than {coins} is not a problem: I split the coins first, with a \
                 transaction to yourself. I will explain if it comes to that."
            }
        ));
    }

    ui::blank();
    let question = Question::new(t(
        "Quante ricariche vuoi preparare?",
        "How many refills do you want to prepare?",
    ))
    .invalid(t(
        "Inserisci un numero intero maggiore di zero",
        "Enter an integer greater than zero",
    ));

    // Bounded by the coin count, so the GUI shows a slider; the scale goes
    // beyond it on purpose (more payments means a split).
    let count = match coins {
        Some(coins) if coins > 0 => {
            let max = (coins * 3).clamp(8, 40);
            let suggested = match received {
                0 => coins.clamp(1, 4),
                sent => sent.min(max),
            };
            ask.range_usize(question, 1..=max, Some(suggested))
        }
        // No coin count, no scale.
        _ => ask.number_usize(question, (received > 0).then_some(received)),
    }?;

    if count == 0 {
        return Err(Error::Interaction(
            t(
                "con zero ricariche non c'è niente da preparare",
                "with zero refills there is nothing to prepare",
            )
            .into(),
        ));
    }
    Ok(count)
}

/// `false` when signing was declined: nothing new on disk to check or export.
pub(crate) fn step_sign(
    ceremony: &Project,
    open: &Open,
    planned: Option<usize>,
    received: &[plebline_core::address::Destination],
    ask: &mut dyn Ask,
) -> Result<bool> {
    ui::title(t("Le ricariche da preparare", "The refills to prepare"));

    // The count is already known: do not ask it again.
    let config = exists(ceremony, project::CONFIG);
    if config.is_none() {
        let built = wizard::ask_batch_config(NETWORK, planned, received, ask)?;
        wizard::write_json(&ceremony.path(project::CONFIG), &built)?;
    }

    wizard::sign(
        NETWORK,
        wizard::SignInput {
            utxos: ceremony.path(project::UTXOS),
            config: exists(ceremony, project::CONFIG),
            config_out: Some(ceremony.path(project::CONFIG)),
            plan: exists(ceremony, project::PLAN),
            plan_out: Some(ceremony.path(project::PLAN)),
            previous: project::signed_files(ceremony),
            out: ceremony.path(project::BATCH),
            wallet: open.access(),
        },
        ask,
    )
}

/// Not enough coins. That is a fact about the chain, so it is explained
/// before unplugging and the user's consent is asked, instead of reporting
/// a configuration error.
pub(crate) fn explain_split(coins: usize, wanted: usize, ask: &mut dyn Ask) -> Result<bool> {
    ui::title(t(
        "Le monete non bastano: le divido",
        "Not enough coins: I will split them",
    ));
    ui::note(&tf!(
        {
            "Hai {coins} monete libere e vuoi preparare {wanted} ricariche. Ognuna deve \
             spendere una moneta diversa, quindi ne servono {wanted}: quelle che mancano vanno \
             create."
        },
        {
            "You have {coins} free coins and want to prepare {wanted} refills. Each must spend \
             a different coin, so {wanted} are needed: the missing ones have to be created."
        }
    ));
    ui::blank();
    ui::note(t(
        "Si fa con una sola transazione, verso te stesso: prende tutte le monete libere e le \
         ridistribuisce. Quelle impegnate restano dove sono. Non mandi niente a nessuno e non \
         spendi niente, a parte la commissione.",
        "It takes a single transaction, to yourself: it takes all the free coins and spreads \
         them out. The committed ones stay where they are. You send nothing to anyone and spend \
         nothing, apart from the fee.",
    ));
    ui::blank();
    ui::warning(t(
        "Questa va mandata in rete adesso e aspettata in un blocco. Finché non è confermata le \
         monete nuove non esistono, e le ricariche non si possono preparare. Di solito dieci \
         minuti, a volte di più.",
        "This one goes out to the network now and has to be waited for in a block. Until it is \
         confirmed the new coins do not exist, and the refills cannot be prepared. Usually ten \
         minutes, sometimes more.",
    ));
    ui::blank();
    ui::note(t(
        "L'ordine è quello giusto anche così: la divisione la firmo a rete staccata, come tutto \
         il resto. Riattaccherai solo per mandarla, quando le tue parole non serviranno più.",
        "The order still holds: I sign the split with the network unplugged, like everything \
         else. You will plug back in only to send it, when your words are no longer needed.",
    ));

    ui::blank();
    let go = ask.confirm(
        Question::new(t(
            "Procedo con la divisione?",
            "Shall I go ahead with the split?",
        )),
        true,
    )?;

    if !go {
        ui::blank();
        ui::info(&tf!(
            {
                "Nessun problema. Torno al portafoglio: da lì puoi chiederne fino a {coins}, \
                 senza dividere niente."
            },
            {
                "No problem. Back to the wallet: from there you can ask for up to {coins}, \
                 without splitting anything."
            }
        ));
    }
    Ok(go)
}

/// Builds and signs the split offline, drops the seed, then asks to plug
/// the network back in to broadcast it. Returns the server it was sent
/// through, or `None` if nothing was signed (the network stays unplugged).
pub(crate) fn do_split(
    ceremony: &Project,
    open: Open,
    server: Option<String>,
    wanted: usize,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<Option<String>> {
    let signed = wizard::split(
        NETWORK,
        wizard::SplitInput {
            utxos: ceremony.path(project::UTXOS),
            previous: project::signed_files(ceremony),
            out: ceremony.path(project::SPLIT),
            wallet: open.access(),
            outputs: Some(wanted),
        },
        ask,
    )?;
    if !signed {
        return Ok(None);
    }

    // Seed dropped (zeroed) before asking to reconnect.
    drop(open);
    crate::road::words(false);

    ui::blank();
    ui::title(t("Solo per mandare questa", "Just to send this one"));
    ui::note(t(
        "Le tue parole non servono più: la transazione è già firmata, e per spedirla bastano i \
         suoi byte. Le ho tolte dalla memoria adesso, prima di chiederti di riattaccare.",
        "Your words are no longer needed: the transaction is already signed, and sending it only \
         takes its bytes. I removed them from memory now, before asking you to plug back in.",
    ));
    ui::blank();
    ui::info(t(
        "Riattacca la rete. Serve per questo passo soltanto.",
        "Plug the network back in. It is needed for this step only.",
    ));
    ask.text(
        Question::new(t("Invio quando sei pronto:", "Enter when you are ready:")),
        None,
    )?;

    crate::road::plugged_back();
    crate::road::at(crate::road::Point::Send);

    // A resumed ceremony may not have asked for the server yet.
    let server = match server {
        Some(server) => server,
        None => ask_server(ask)?,
    };

    let sent = online::broadcast(
        NETWORK,
        &server,
        online::BroadcastInput {
            batch: ceremony.path(project::SPLIT),
            index: 0,
        },
        ask,
        chain,
    )?;
    // Declined at the confirmation: nothing went out, so the snapshot still
    // matches the chain.
    if !sent {
        let dir = project::display(&ceremony.dir);
        ui::blank();
        ui::warning(&tf!(
            {
                "La divisione è firmata ma non è partita. Le monete che spende restano \
                 impegnate finché non la mandi: il file è in {dir}."
            },
            {
                "The split is signed but did not go out. The coins it spends stay committed \
                 until you send it: the file is in {dir}."
            }
        ));
        return Ok(Some(server));
    }

    // The snapshot is now stale (the split spent the coins): remove it so the
    // next launch rescans.
    ceremony.remove_file(project::UTXOS)?;

    ui::blank();
    ui::success(&tf!(
        { "Divisione mandata in rete. Fra poco avrai {wanted} monete libere." },
        { "Split sent to the network. Shortly you will have {wanted} free coins." }
    ));
    ui::blank();
    ui::info(t(
        "Adesso aspetta che entri in un blocco: dieci minuti, a volte di più.",
        "Now wait for it to get into a block: ten minutes, sometimes more.",
    ));
    ui::info(t(
        "Torno al portafoglio: quando è in un blocco, rileggi la catena e le monete nuove \
         compaiono.",
        "Back to the wallet: once it is in a block, read the chain again and the new coins \
         appear.",
    ));
    ui::blank();
    ui::note(t(
        "Per firmare le ricariche ti serviranno di nuovo le parole: tienile a portata. Sul disco \
         non è rimasto niente che permetta di spendere.",
        "Signing the refills will need your words again: keep them handy. Nothing that allows \
         spending is left on the disk.",
    ));
    Ok(Some(server))
}

/// Always runs the verification; the user is not asked whether to.
pub(crate) fn step_check(ceremony: &Project) -> Result<bool> {
    ui::blank();
    ui::note(t(
        "Adesso ricontrollo tutto quello che ho appena scritto, ripartendo dai byte delle \
         transazioni: le firme, gli importi, dove finisce il resto. Senza chiavi e senza rete.",
        "Now I re-check everything I have just written, starting again from the transaction \
         bytes: the signatures, the amounts, where the change ends up. No keys, no network.",
    ));
    wizard::verify(
        NETWORK,
        wizard::VerifyInput {
            batch: ceremony.path(project::BATCH),
            utxos: exists(ceremony, project::UTXOS),
            config: exists(ceremony, project::CONFIG),
            wallet: exists(ceremony, project::WALLET),
        },
    )
}

/// Before sending. The signatures prove the group is the one signed, which
/// was checked against its definition and coin list then; both files may have
/// changed since (a new round started, the chain read again), so they are not
/// used here.
pub(crate) fn check_before_send(ceremony: &Project) -> Result<bool> {
    ui::blank();
    ui::note(t(
        "Prima di mandare ricontrollo il gruppo dai byte: le firme, che nessuna ricarica \
         dipenda da un'altra, e che il resto torni a te. Destinatari e monete li ho controllati \
         quando l'ho firmato.",
        "Before sending I re-check the group from its bytes: the signatures, that no refill \
         depends on another, and that the change comes back to you. Recipients and coins were \
         checked when it was signed.",
    ));
    wizard::verify(
        NETWORK,
        wizard::VerifyInput {
            batch: ceremony.path(project::BATCH),
            utxos: None,
            config: None,
            wallet: exists(ceremony, project::WALLET),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};

    #[test]
    fn zero_payments_is_not_a_ceremony() {
        in_italian();
        let mut ask = Scripted::new([Reply::number(0usize)]);
        let error = ask_payment_count(None, 0, &mut ask).expect_err("rifiutato");
        assert!(matches!(error, Error::Interaction(_)), "{error}");
    }

    #[test]
    fn the_payment_count_is_the_one_typed() {
        in_italian();
        let mut ask = Scripted::new([Reply::number(3usize)]);
        assert_eq!(ask_payment_count(None, 0, &mut ask).expect("numero"), 3);
    }

    /// Refusing the split is an answer: stop, files stay.
    #[test]
    fn refusing_the_split_stops_the_ceremony_without_errors() {
        in_italian();
        let mut ask = Scripted::new([Reply::No]);
        assert!(!explain_split(2, 5, &mut ask).expect("risposta"));
    }

    fn open() -> Open {
        Open {
            seed: plebline_core::wallet::SeedWords::from_interactive_input(SEED12)
                .expect("seed valido"),
            passphrase: None,
        }
    }

    /// One refill of 50 000 sat, signed for real: its coin is committed.
    fn sign_one_refill(ceremony: &Project) -> Vec<String> {
        let config = ceremony.path(project::CONFIG);
        wizard::write_json(
            &config,
            &plebline_core::plan::BatchConfig {
                network: NETWORK.to_string(),
                fee_rate_sat_vb: 2.0,
                entries: vec![plebline_core::plan::BatchEntry {
                    label: None,
                    recipient: recipient(0),
                    amount_sat: 50_000,
                }],
            },
        )
        .expect("definizione scritta");
        let signed = wizard::sign(
            NETWORK,
            wizard::SignInput {
                utxos: ceremony.path(project::UTXOS),
                config: Some(config),
                config_out: None,
                plan: None,
                plan_out: None,
                previous: project::signed_files(ceremony),
                out: ceremony.path(project::BATCH),
                wallet: open().access(),
            },
            &mut Scripted::new([Reply::Yes]),
        )
        .expect("firmata");
        assert!(signed);
        let batch = plebline_core::verify::load(&ceremony.path(project::BATCH)).expect("letto");
        batch.txs[0]
            .inputs
            .iter()
            .map(|i| i.outpoint.clone())
            .collect()
    }

    fn rename_wallet_in_utxos(ceremony: &Project, name: Option<&str>) {
        let path = ceremony.path(project::UTXOS);
        let mut set: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("leggibile"))
                .expect("json");
        match name {
            Some(name) => set["descriptor_checksum"] = serde_json::json!(name),
            None => {
                set.as_object_mut()
                    .expect("oggetto")
                    .remove("descriptor_checksum");
            }
        }
        std::fs::write(&path, set.to_string()).expect("scrivibile");
    }

    /// A coin list under another wallet's name hides this wallet's signed
    /// refills: their coins would look free and be signed twice. Refused
    /// before anything is planned: no plan written, no question asked.
    #[test]
    fn coins_listed_under_another_wallets_name_are_refused() {
        in_italian();
        let temp = Temp::new("monete-altro-nome");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);
        sign_one_refill(&ceremony);
        rename_wallet_in_utxos(&ceremony, Some("aaaaaaaa"));

        let mut ask = Scripted::new([]);
        let error = step_sign(&ceremony, &open(), None, &[], &mut ask)
            .expect_err("l'elenco dice un altro wallet");
        assert!(error.to_string().contains("aaaaaaaa"), "{error}");
        assert!(!ceremony.has(project::PLAN), "nessun piano scritto");
        assert!(ask.asked().is_empty(), "{:?}", ask.prompts());
    }

    /// A label or an address read from a file may carry terminal commands
    /// (escape sequences, carriage return, bidi overrides, a line break that
    /// fakes a line of ours): shown as visible text, never obeyed, and the
    /// file is left as it is.
    #[test]
    fn terminal_commands_in_a_signed_file_are_shown_not_obeyed() {
        in_italian();
        let temp = Temp::new("comandi-terminale");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);
        sign_one_refill(&ceremony);
        let path = ceremony.path(project::BATCH);
        let mut batch: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("leggibile"))
                .expect("json");
        batch["txs"][0]["label"] = serde_json::json!(
            "affitto\u{1b}[2J\u{1b}]0;titolo\u{7}\r\u{202e}otnemagap\n  ✓ tutto a posto"
        );
        batch["txs"][0]["outputs"][0]["address"] = serde_json::json!("tb1q\u{1b}[8mnascosto");
        std::fs::write(&path, batch.to_string()).expect("scrivibile");
        let before = std::fs::read(&path).expect("leggibile");

        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        let shown = online::prepare(
            NETWORK,
            &online::BroadcastInput {
                batch: path.clone(),
                index: 0,
            },
        );
        plebline::out::take();
        shown.expect("mostrata");

        let obeyed =
            |c: char| (c.is_control() && c != '\n') || ('\u{202a}'..='\u{202e}').contains(&c);
        for line in notebook.lines() {
            assert!(!line.text.chars().any(obeyed), "{:?}", line.text);
            if line.kind == plebline::out::Kind::Info {
                assert!(!line.text.contains('\n'), "una riga sola: {:?}", line.text);
            }
        }
        let screen = notebook.text();
        assert!(screen.contains("affitto"), "{screen}");
        assert!(
            screen.contains("\\u{1b}"),
            "si vede, non si esegue: {screen}"
        );
        assert_eq!(
            std::fs::read(&path).expect("leggibile"),
            before,
            "il file non si tocca"
        );
    }

    /// Signs a split of the coins in `utxos.json` into two; returns the
    /// coins it spends.
    fn split_in_two(ceremony: &Project) -> Vec<String> {
        let signed = wizard::split(
            NETWORK,
            wizard::SplitInput {
                utxos: ceremony.path(project::UTXOS),
                previous: project::signed_files(ceremony),
                out: ceremony.path(project::SPLIT),
                wallet: open().access(),
                outputs: Some(2),
            },
            &mut Scripted::new([Reply::number(2.0f64), Reply::Yes]),
        )
        .expect("divisione firmata");
        assert!(signed);
        let split = plebline_core::verify::load(&ceremony.path(project::SPLIT)).expect("letta");
        split.txs[0]
            .inputs
            .iter()
            .map(|i| i.outpoint.clone())
            .collect()
    }

    /// A split signed and never sent still commits its coins after a newer
    /// split takes its place: it may already be on a stick.
    #[test]
    fn a_later_split_never_takes_the_coins_of_an_earlier_one() {
        in_italian();
        let temp = Temp::new("divisioni");
        let ceremony = temp.ceremony();
        snapshot_with(&ceremony, &[(0, 200_000, 1), (1, 300_000, 2)]);
        let first = split_in_two(&ceremony);

        // Never sent: those coins are still there, and a new one arrives.
        snapshot_with(
            &ceremony,
            &[(0, 200_000, 1), (1, 300_000, 2), (2, 250_000, 3)],
        );
        let second = split_in_two(&ceremony);
        assert!(second.iter().all(|coin| !first.contains(coin)));

        let held = [
            (0, 200_000, 1),
            (1, 300_000, 2),
            (2, 250_000, 3),
            (3, 400_000, 4),
        ];
        let all = snapshot_with(&ceremony, &held);
        let third = split_in_two(&ceremony);
        assert_eq!(third, [all[3].clone()], "solo la moneta nuova");

        let archives: Vec<_> = project::signed_files(&ceremony)
            .into_iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert!(
            archives.contains(&"split-1.json".to_string()),
            "{archives:?}"
        );
        assert!(
            archives.contains(&"split-2.json".to_string()),
            "{archives:?}"
        );
    }

    /// Older lists name no wallet: still accepted, and the coin of a refill
    /// already signed is still left alone. The plan is written before the
    /// confirmation, so a No leaves it to read and signs nothing.
    #[test]
    fn a_list_with_no_wallet_name_still_leaves_committed_coins_alone() {
        in_italian();
        let temp = Temp::new("monete-senza-nome");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);
        let first = sign_one_refill(&ceremony);
        rename_wallet_in_utxos(&ceremony, None);

        let mut ask = Scripted::new([Reply::No]);
        let signed = step_sign(&ceremony, &open(), None, &[], &mut ask)
            .expect("un elenco senza nome resta valido");
        assert!(!signed, "la firma è stata rifiutata");
        assert_eq!(ask.remaining(), 0);
        let plan: plebline_core::plan::PlannedBatch =
            plebline_core::read_json(&ceremony.path(project::PLAN)).expect("piano");
        for inputs in plan.inputs_by_tx() {
            assert!(
                inputs.iter().all(|coin| !first.contains(coin)),
                "{inputs:?} riusa {first:?}"
            );
        }
    }

    /// A refused split: the server's reason reaches the user, nothing counts
    /// as sent, and the snapshot stays because no coin moved.
    #[test]
    fn a_refused_split_keeps_the_snapshot() {
        in_italian();
        let temp = Temp::new("divisione-rifiutata");
        let ceremony = temp.ceremony();
        snapshot_of_two(&ceremony);

        // Sign once to learn the txid: it leaves out the witness, so the
        // second signing gives the same one.
        let signed = wizard::split(
            NETWORK,
            wizard::SplitInput {
                utxos: ceremony.path(project::UTXOS),
                previous: Vec::new(),
                out: ceremony.path(project::SPLIT),
                wallet: open().access(),
                outputs: Some(3),
            },
            &mut Scripted::new([Reply::number(2.0f64), Reply::Yes]),
        )
        .expect("divisione firmata");
        assert!(signed);
        let split = plebline_core::verify::load(&ceremony.path(project::SPLIT)).expect("letta");
        let txid = split.txs[0].txid.clone();
        std::fs::remove_file(ceremony.path(project::SPLIT)).expect("tolta");

        // A split asks yes or no on every network.
        let mut ask = Scripted::new([
            Reply::number(2.0f64),
            Reply::Yes,
            Reply::text(""),
            Reply::Yes,
        ]);
        let mut chain =
            plebline::chain::Fake::new().refusing_broadcast(txid, "min relay fee not met");

        let error = do_split(
            &ceremony,
            open(),
            Some("http://server-finto".to_string()),
            3,
            &mut ask,
            &mut chain,
        )
        .expect_err("il server rifiuta");

        assert!(
            error.to_string().contains("min relay fee not met"),
            "{error}"
        );
        assert_eq!(ask.remaining(), 0);
        assert!(chain.sent().is_empty(), "{:?}", chain.sent());
        assert!(
            ceremony.path(project::UTXOS).is_file(),
            "nessuna moneta si è mossa: la fotografia resta"
        );
    }

    /// `utxos.json` listing only output 0 of a source transaction that also
    /// pays the wallet at output 1 (larger), as after a split whose other
    /// coin was spent by a refill that landed. Returns (listed, unlisted).
    fn snapshot_with_an_unlisted_sibling(ceremony: &Project) -> (String, String) {
        use plebline_core::bdk_wallet::KeychainKind;
        use plebline_core::bitcoin::consensus::encode::serialize_hex;

        let wallet = write_wallet(ceremony);
        let spk = |index| {
            wallet
                .peek_address(KeychainKind::External, index)
                .address
                .script_pubkey()
        };
        let parent = plebline::testing::paying(9, &[(spk(0), 200_000), (spk(1), 500_000)]);
        let txid = parent.compute_txid();
        let listed = plebline::chain::Coin {
            outpoint: format!("{txid}:0"),
            value_sat: 200_000,
            script_pubkey: parent.output[0].script_pubkey.to_hex_string(),
            keychain: "external".into(),
            derivation_index: 0,
            height: Some(799_900),
            origin: Some(plebline::chain::Origin {
                hex: serialize_hex(&parent),
                coinbase: false,
            }),
        };
        let scanned =
            plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet)).with(listed.clone());
        plebline::online::export_utxos(
            NETWORK,
            "http://server-finto",
            plebline::online::ExportInput {
                wallet: ceremony.path(project::WALLET),
                out: ceremony.path(project::UTXOS),
            },
            &mut plebline::chain::Fake::new().with_scan(scanned),
        )
        .expect("utxos.json scritto");
        (listed.outpoint, format!("{txid}:1"))
    }

    /// Signing, the real path: the refill is signed, and on the listed coin,
    /// not on its larger unlisted sibling.
    #[test]
    fn signing_pays_from_the_listed_coin_not_from_its_sibling() {
        in_italian();
        let temp = Temp::new("fratello-firma");
        let ceremony = temp.ceremony();
        let (listed, unlisted) = snapshot_with_an_unlisted_sibling(&ceremony);

        let spent = sign_one_refill(&ceremony);

        assert_eq!(
            spent,
            vec![listed.clone()],
            "la ricarica spende {listed}, non {unlisted} che utxos.json non elenca"
        );
    }

    /// Splitting, the real path: signed and written only on the coins
    /// `utxos.json` lists.
    #[test]
    fn the_split_never_signs_a_coin_the_file_does_not_list() {
        in_italian();
        let temp = Temp::new("fratello-divisione");
        let ceremony = temp.ceremony();
        let (listed, unlisted) = snapshot_with_an_unlisted_sibling(&ceremony);

        let signed = wizard::split(
            NETWORK,
            wizard::SplitInput {
                utxos: ceremony.path(project::UTXOS),
                previous: project::signed_files(&ceremony),
                out: ceremony.path(project::SPLIT),
                wallet: open().access(),
                outputs: Some(2),
            },
            &mut Scripted::new([Reply::number(2.0f64), Reply::Yes]),
        )
        .expect("divisione firmata");

        assert!(signed, "la divisione è firmata");
        let split = plebline_core::verify::load(&ceremony.path(project::SPLIT)).expect("letta");
        let spent: Vec<&String> = split.txs[0].inputs.iter().map(|i| &i.outpoint).collect();
        assert_eq!(
            spent,
            vec![&listed],
            "la divisione spende {listed}, non {unlisted} che utxos.json non elenca"
        );
    }
}
