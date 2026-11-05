//! Ceremony files come back from a USB stick or a phone, so they are treated
//! as hostile. Every loader must answer garbage with an error, without
//! panicking and without accepting any part of a bad file.

use std::path::PathBuf;

use plebline_core::batch::{self, SignedBatch};
use plebline_core::bdk_wallet::{KeychainKind, Wallet};
use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::{Amount, ScriptBuf, TxOut};
use plebline_core::plan::{self, BatchConfig, BatchEntry, PlannedBatch};
use plebline_core::verify::{self, Expectations};
use plebline_core::{read_json, utxos, watch, Error};
use serde_json::{json, Value};

mod common;
use common::{utxo_set, NETWORK};

/// A self-deleting directory, to avoid a `tempfile` dependency.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "plebline-ostili-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("cartella creabile");
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("scrivibile");
        path
    }

    fn json(&self, name: &str, value: &Value) -> PathBuf {
        self.write(name, value.to_string().as_bytes())
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bytes that are not any of our files.
fn garbage() -> Vec<(&'static str, Vec<u8>)> {
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    vec![
        ("vuoto", Vec::new()),
        ("troncato", b"{\"version\": 1, \"network\": \"reg".to_vec()),
        ("parentesi", b"{".to_vec()),
        ("lista", b"[]".to_vec()),
        ("null", b"null".to_vec()),
        ("numero", b"42".to_vec()),
        ("oggetto vuoto", b"{}".to_vec()),
        ("tipo sbagliato", b"{\"version\": \"uno\"}".to_vec()),
        (
            "non utf-8",
            b"{\"version\": 1, \"network\": \"\xff\xfe\"}".to_vec(),
        ),
        ("zeri", vec![0u8; 64]),
        ("annidato", deep.into_bytes()),
        ("numero enorme", b"{\"version\": 1e999}".to_vec()),
    ]
}

/// Every loader, by name: each must refuse what it is given.
fn every_loader(path: &std::path::Path) -> Vec<(&'static str, bool)> {
    vec![
        ("utxos.json", utxos::load(path, NETWORK).is_err()),
        ("wallet.json", watch::load(path, NETWORK).is_err()),
        ("batch.json", verify::load(path).is_err()),
        ("batch-config.json", read_json::<BatchConfig>(path).is_err()),
        ("plan.json", read_json::<PlannedBatch>(path).is_err()),
    ]
}

#[test]
fn garbage_is_refused_by_every_loader() {
    let temp = Temp::new("spazzatura");
    for (what, bytes) in garbage() {
        let path = temp.write("file.json", &bytes);
        for (loader, refused) in every_loader(&path) {
            assert!(refused, "{loader} ha accettato «{what}»");
        }
    }
}

#[test]
fn a_directory_or_a_missing_file_is_an_error_not_a_panic() {
    let temp = Temp::new("cartella");
    let dir = temp.0.join("utxos.json");
    std::fs::create_dir_all(&dir).expect("cartella creabile");
    for (loader, refused) in every_loader(&dir) {
        assert!(refused, "{loader} ha letto una cartella");
    }
    for (loader, refused) in every_loader(&temp.0.join("non-esiste.json")) {
        assert!(refused, "{loader} ha letto un file che non c'è");
    }
}

#[test]
fn the_error_says_which_file_in_both_languages() {
    use plebline_core::lang::{self, Lang};
    let temp = Temp::new("lingue");
    let path = temp.write("utxos.json", b"{");
    let error = utxos::load(&path, NETWORK).expect_err("JSON rotto");
    lang::set_for_this_thread(Lang::En);
    let english = error.to_string();
    lang::set_for_this_thread(Lang::It);
    let italian = error.to_string();
    assert!(english.contains("utxos.json"), "{english}");
    assert!(italian.contains("utxos.json"), "{italian}");
    assert_ne!(english, italian);
}

fn wallet() -> Wallet {
    common::wallet(common::SEED, None)
}

fn utxos_json(funding: &[(u32, u64, u8)]) -> Value {
    serde_json::to_value(utxo_set(&wallet(), funding)).expect("serializzabile")
}

fn load_utxos(temp: &Temp, value: &Value) -> plebline_core::Result<utxos::ValidatedUtxoSet> {
    utxos::load(&temp.json("utxos.json", value), NETWORK)
}

#[test]
fn utxos_with_wrong_types_or_missing_fields_are_refused() {
    let temp = Temp::new("utxos-tipi");
    let good = utxos_json(&[(0, 100_000, 1)]);
    load_utxos(&temp, &good).expect("il file buono passa");

    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut v = good.clone();
    v["utxos"][0]["value_sat"] = json!("100000");
    cases.push(("importo come testo", v));
    let mut v = good.clone();
    v["utxos"][0]["value_sat"] = json!(-1);
    cases.push(("importo negativo", v));
    let mut v = good.clone();
    v["utxos"][0]["value_sat"] = serde_json::from_str("18446744073709551616").expect("numero");
    cases.push(("importo oltre u64", v));
    let mut v = good.clone();
    v["utxos"][0]["value_sat"] = json!(0);
    cases.push(("importo zero, diverso dalla transazione", v));
    let mut v = good.clone();
    v["utxos"][0]
        .as_object_mut()
        .expect("oggetto")
        .remove("prev_tx_hex");
    cases.push(("manca la transazione d'origine", v));
    let mut v = good.clone();
    v.as_object_mut().expect("oggetto").remove("utxos");
    cases.push(("manca l'elenco", v));
    let mut v = good.clone();
    v["version"] = json!(2);
    cases.push(("versione sconosciuta", v));
    let mut v = good.clone();
    v["network"] = json!("bitcoin");
    cases.push(("rete sbagliata", v));
    let mut v = good.clone();
    v["network"] = json!(" regtest");
    cases.push(("rete con spazi", v));
    let mut v = good.clone();
    v["utxos"][0]["keychain"] = json!("EXTERNAL");
    cases.push(("keychain sconosciuto", v));
    let mut v = good.clone();
    v["utxos"][0]["derivation_index"] = json!(u64::from(u32::MAX) + 1);
    cases.push(("indice oltre u32", v));

    for (what, value) in cases {
        assert!(load_utxos(&temp, &value).is_err(), "accettato: {what}");
    }
}

#[test]
fn extra_fields_in_utxos_change_nothing() {
    let temp = Temp::new("utxos-extra");
    let good = utxos_json(&[(0, 100_000, 1)]);
    let mut extra = good.clone();
    extra["chi_sono"] = json!("un campo in più");
    extra["utxos"][0]["value_btc"] = json!(1000);
    let plain = load_utxos(&temp, &good).expect("buono");
    let with_extra = load_utxos(&temp, &extra).expect("i campi in più si ignorano");
    assert_eq!(plain.total(), with_extra.total());
    assert_eq!(
        plain.utxos.iter().map(|u| u.outpoint).collect::<Vec<_>>(),
        with_extra
            .utxos
            .iter()
            .map(|u| u.outpoint)
            .collect::<Vec<_>>()
    );
}

#[test]
fn malformed_coins_are_refused() {
    let temp = Temp::new("utxos-monete");
    let good = utxos_json(&[(0, 100_000, 1)]);
    let txid = good["utxos"][0]["outpoint"]
        .as_str()
        .expect("testo")
        .split_once(':')
        .expect("txid:vout")
        .0
        .to_string();

    let mut cases: Vec<(&str, Value)> = Vec::new();
    for (what, outpoint) in [
        ("outpoint senza vout", txid.clone()),
        ("vout fuori dalla transazione", format!("{txid}:5")),
        ("vout oltre u32", format!("{txid}:4294967296")),
        ("txid corto", "abcd:0".to_string()),
        ("outpoint con spazi", format!(" {txid}:0")),
        ("outpoint non ascii", format!("{}é:0", &txid[..62])),
    ] {
        let mut v = good.clone();
        v["utxos"][0]["outpoint"] = json!(outpoint);
        cases.push((what, v));
    }
    for (what, hex) in [
        ("hex vuoto", String::new()),
        ("hex non esadecimale", "zz".repeat(100)),
        ("hex dispari", "0".repeat(101)),
        ("hex troncato", {
            let full = good["utxos"][0]["prev_tx_hex"].as_str().expect("testo");
            full[..full.len() - 10].to_string()
        }),
        ("hex lunghissimo", "00".repeat(2_000_000)),
    ] {
        let mut v = good.clone();
        v["utxos"][0]["prev_tx_hex"] = json!(hex);
        cases.push((what, v));
    }
    let mut v = good.clone();
    v["utxos"][0]["script_pubkey"] = json!("non hex");
    cases.push(("script non hex", v));

    for (what, value) in cases {
        assert!(load_utxos(&temp, &value).is_err(), "accettato: {what}");
    }
}

/// The same coin twice would let two payments spend it.
#[test]
fn a_coin_listed_twice_is_refused_even_in_a_huge_list() {
    let temp = Temp::new("utxos-doppie");
    let mut v = utxos_json(&[(0, 100_000, 1)]);
    let entry = v["utxos"][0].clone();
    v["utxos"] = Value::Array(vec![entry; 20_000]);
    let error = load_utxos(&temp, &v).expect_err("una moneta ripetuta");
    assert!(matches!(error, Error::InvalidUtxo { .. }), "{error}");
}

/// No coin is worth more than every bitcoin there will ever be: such a
/// source transaction is invalid, and its sums would overflow downstream.
#[test]
fn a_coin_worth_more_than_all_bitcoin_is_refused() {
    let temp = Temp::new("utxos-troppo");
    let over = Amount::MAX_MONEY.to_sat() + 1;
    let error = load_utxos(&temp, &utxos_json(&[(0, over, 1)])).expect_err("oltre 21 milioni");
    assert!(matches!(error, Error::InvalidUtxo { .. }), "{error}");

    let error = load_utxos(&temp, &utxos_json(&[(0, u64::MAX, 1)])).expect_err("u64::MAX");
    assert!(matches!(error, Error::InvalidUtxo { .. }), "{error}");
}

#[test]
fn coins_adding_up_to_more_than_all_bitcoin_are_refused() {
    let temp = Temp::new("utxos-somma");
    let half = Amount::MAX_MONEY.to_sat() / 2 + 1;
    let error = load_utxos(&temp, &utxos_json(&[(0, half, 1), (1, half, 2)]))
        .expect_err("insieme oltre 21 milioni");
    assert!(matches!(error, Error::InvalidUtxo { .. }), "{error}");
}

/// A source transaction with an impossible output that is not even ours.
#[test]
fn a_source_transaction_with_an_impossible_output_is_refused() {
    let temp = Temp::new("utxos-origine");
    let wallet = wallet();
    let mut tx = common::funding_tx(&wallet, 0, 100_000, 1);
    tx.output.push(TxOut {
        value: Amount::from_sat(u64::MAX),
        script_pubkey: ScriptBuf::new(),
    });
    let mut v = utxos_json(&[(0, 100_000, 1)]);
    v["utxos"][0]["outpoint"] = json!(format!("{}:0", tx.compute_txid()));
    v["utxos"][0]["prev_tx_hex"] = json!(serialize_hex(&tx));
    let error = load_utxos(&temp, &v).expect_err("output impossibile");
    assert!(matches!(error, Error::InvalidUtxo { .. }), "{error}");
}

/// BIP32 indexes above 2^31 are hardened: no public descriptor reaches
/// them, and the margin added later would overflow.
#[test]
fn derivation_indexes_beyond_bip32_are_refused() {
    let temp = Temp::new("utxos-indici");
    let mut v = utxos_json(&[(0, 100_000, 1)]);
    v["utxos"][0]["derivation_index"] = json!(u32::MAX);
    assert!(load_utxos(&temp, &v).is_err(), "indice della moneta");

    let mut v = utxos_json(&[(0, 100_000, 1)]);
    v["last_active_indices"]["external"] = json!(1u64 << 31);
    assert!(load_utxos(&temp, &v).is_err(), "indice dichiarato");
}

/// Below 2^31 an index is still derivable, one address at a time: a file
/// claiming index two billion would stall the program for days.
#[test]
fn derivation_indexes_no_wallet_reaches_are_refused() {
    let temp = Temp::new("utxos-indici-enormi");
    let cap = utxos::MAX_DERIVATION_INDEX;
    for (what, field, index) in [
        ("moneta", "derivation_index", cap + 1),
        ("moneta", "derivation_index", (1 << 31) - 1),
        ("dichiarato", "last_active_indices", cap + 1),
    ] {
        let mut v = utxos_json(&[(0, 100_000, 1)]);
        if field == "derivation_index" {
            v["utxos"][0]["derivation_index"] = json!(index);
        } else {
            v["last_active_indices"]["internal"] = json!(index);
        }
        let error = load_utxos(&temp, &v).expect_err(what);
        assert!(error.to_string().contains(&index.to_string()), "{error}");
    }

    let mut v = utxos_json(&[(0, 100_000, 1)]);
    v["last_active_indices"]["external"] = json!(cap);
    load_utxos(&temp, &v).expect("al limite si accetta");
}

fn wallet_json(wallet: &Wallet) -> Value {
    let (external, internal) = plebline_core::wallet::public_descriptors(wallet);
    serde_json::to_value(watch::WatchFile {
        version: watch::WATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        external,
        internal,
        descriptor_checksum: wallet.descriptor_checksum(KeychainKind::External),
        created_by: "test".into(),
    })
    .expect("serializzabile")
}

#[test]
fn a_broken_wallet_json_is_refused() {
    let temp = Temp::new("wallet");
    let good = wallet_json(&wallet());
    watch::load(&temp.json("wallet.json", &good), NETWORK).expect("il file buono passa");

    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut v = good.clone();
    v["version"] = json!(99);
    cases.push(("versione", v));
    let mut v = good.clone();
    v["network"] = json!("bitcoin");
    cases.push(("rete", v));
    let mut v = good.clone();
    v["external"] = json!("wpkh(non una chiave)");
    cases.push(("descriptor illeggibile", v));
    let mut v = good.clone();
    let external = v["external"].as_str().expect("testo").to_string();
    let (body, _) = external.rsplit_once('#').expect("con checksum");
    v["external"] = json!(format!("{body}#deadbeef"));
    cases.push(("checksum del descriptor alterato", v));
    let mut v = good.clone();
    v["descriptor_checksum"] = json!("aaaaaaaa");
    cases.push(("checksum dichiarato diverso dal descriptor", v));
    let mut v = good.clone();
    let other = wallet_json(&common::wallet(common::OTHER_SEED, None));
    v["external"] = other["external"].clone();
    cases.push(("descriptor di un altro wallet", v));

    for (what, value) in cases {
        let path = temp.json("wallet.json", &value);
        assert!(watch::load(&path, NETWORK).is_err(), "accettato: {what}");
    }
}

/// The descriptor without its `#checksum`, so it can be edited.
fn body(descriptor: &str) -> String {
    descriptor
        .rsplit_once('#')
        .expect("con checksum")
        .0
        .to_string()
}

/// `wallet.json` from two descriptors, named as the wallet built from them.
fn watch_file(external: &str, internal: &str) -> plebline_core::Result<Value> {
    let wallet = plebline_core::wallet::from_descriptors(external, internal, NETWORK)?;
    Ok(serde_json::to_value(watch::WatchFile {
        version: watch::WATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        external: external.to_string(),
        internal: internal.to_string(),
        descriptor_checksum: wallet.descriptor_checksum(KeychainKind::External),
        created_by: "test".into(),
    })
    .expect("serializzabile"))
}

/// The change descriptor decides which outputs count as change; one from
/// another wallet would make a payment to someone else look like change.
#[test]
fn a_change_descriptor_of_another_account_is_refused() {
    let temp = Temp::new("wallet-resto");
    let (external, _) = plebline_core::wallet::public_descriptors(&wallet());
    let (_, theirs) =
        plebline_core::wallet::public_descriptors(&common::wallet(common::OTHER_SEED, None));
    let ours = body(&external);

    for (what, internal) in [
        ("resto di un altro wallet", theirs.clone()),
        ("ramo 2 invece di 1", ours.replace("/0/*", "/2/*")),
        ("un livello in più", ours.replace("/0/*", "/1/0/*")),
        (
            "altro tipo di script",
            ours.replacen("wpkh(", "pkh(", 1).replace("/0/*", "/1/*"),
        ),
    ] {
        let file = watch_file(&external, &internal);
        let refused = match file {
            Ok(file) => watch::load(&temp.json("wallet.json", &file), NETWORK).is_err(),
            Err(_) => true,
        };
        assert!(refused, "accettato: {what}");
    }
}

/// Every wallet.json the program writes still loads: from the words, and
/// imported from another wallet (`h` for hardened, no checksum, multisig).
#[test]
fn wallet_json_as_the_program_writes_it_still_loads() {
    let temp = Temp::new("wallet-buoni");
    let (external, internal) = plebline_core::wallet::public_descriptors(&wallet());
    let (other_external, _) =
        plebline_core::wallet::public_descriptors(&common::wallet(common::OTHER_SEED, None));

    let sparrow = |d: &str| body(d).replace('\'', "h");
    let key = |d: &str, branch: &str| {
        body(d)
            .trim_start_matches("wpkh(")
            .trim_end_matches(')')
            .replace("/0/*", branch)
    };
    let multisig = |branch: &str| {
        format!(
            "wsh(sortedmulti(2,{},{}))",
            key(&external, branch),
            key(&other_external, branch)
        )
    };

    for (what, external, internal) in [
        ("dalle parole", external.clone(), internal.clone()),
        ("stile Sparrow", sparrow(&external), sparrow(&internal)),
        ("multisig", multisig("/0/*"), multisig("/1/*")),
    ] {
        let file = watch_file(&external, &internal).expect(what);
        watch::load(&temp.json("wallet.json", &file), NETWORK).expect(what);
    }
}

fn destination(index: u32) -> String {
    common::wallet(common::OTHER_SEED, None)
        .peek_address(KeychainKind::External, index)
        .address
        .to_string()
}

/// A batch really signed, then serialized as it would be on disk.
fn signed_batch() -> (SignedBatch, Value) {
    let (mut wallet, _) = common::hydrated(wallet(), &[(0, 200_000, 1), (1, 200_000, 2)]);
    let config = BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 5.0,
        entries: (0..2)
            .map(|i| BatchEntry {
                label: Some(format!("ricarica {i}")),
                recipient: destination(i),
                amount_sat: 150_000,
            })
            .collect(),
    };
    let built = plan::build(&mut wallet, &config, []).expect("costruibile");
    let signed = batch::sign(&wallet, built, config.fee_rate_sat_vb).expect("firmato");
    let value = serde_json::to_value(&signed).expect("serializzabile");
    (signed, value)
}

/// The verification report for a hostile batch, or `None` if loading
/// already refuses it.
fn verified(temp: &Temp, value: &Value) -> Option<verify::Report> {
    let path = temp.json("batch.json", value);
    let batch = verify::load(&path).ok()?;
    Some(verify::verify(
        &batch,
        &Expectations {
            network: Some(NETWORK),
            ..Default::default()
        },
    ))
}

/// Refused at load or failed by verification.
fn refused(temp: &Temp, value: &Value) -> bool {
    verified(temp, value).is_none_or(|report| !report.is_ok())
}

#[test]
fn a_batch_with_wrong_types_or_missing_fields_is_refused() {
    let temp = Temp::new("batch-tipi");
    let (_, good) = signed_batch();
    assert!(!refused(&temp, &good), "il gruppo buono passa");

    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut v = good.clone();
    v["txs"][0]["inputs"][0]["value_sat"] = json!("200000");
    cases.push(("importo come testo", v));
    let mut v = good.clone();
    v["txs"][0]["fee_sat"] = json!(-1);
    cases.push(("fee negativa", v));
    let mut v = good.clone();
    v["txs"][0]["index"] = json!(-1);
    cases.push(("indice negativo", v));
    let mut v = good.clone();
    v["txs"][0].as_object_mut().expect("oggetto").remove("hex");
    cases.push(("manca l'hex", v));
    let mut v = good.clone();
    v["txs"] = json!([]);
    cases.push(("nessuna transazione", v));
    let mut v = good.clone();
    v["txs"][0]["hex"] = json!("zz");
    cases.push(("hex non esadecimale", v));
    let mut v = good.clone();
    v["txs"][0]["hex"] = json!("00".repeat(1_000_000));
    cases.push(("hex lunghissimo", v));
    let mut v = good.clone();
    v["txs"][0]["txid"] = json!("abc");
    cases.push(("txid corto", v));
    let mut v = good.clone();
    v["txs"][0]["txid"] = json!("é".repeat(40));
    cases.push(("txid non ascii", v));
    let mut v = good.clone();
    v["network"] = json!("bitcoin");
    cases.push(("rete sbagliata", v));
    let mut v = good.clone();
    v["txs"][1] = v["txs"][0].clone();
    cases.push(("la stessa transazione due volte", v));
    let mut v = good.clone();
    v["txs"][0]["inputs"][0]["outpoint"] = json!("non un outpoint");
    cases.push(("outpoint illeggibile", v));

    for (what, value) in cases {
        assert!(refused(&temp, &value), "accettato: {what}");
    }
}

/// Amounts no transaction can have: refused before any sum is made.
#[test]
fn a_batch_with_impossible_amounts_is_refused_at_load() {
    let temp = Temp::new("batch-importi");
    let (_, good) = signed_batch();

    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut v = good.clone();
    v["txs"][0]["inputs"][0]["value_sat"] = json!(u64::MAX);
    cases.push(("input u64::MAX", v));
    let mut v = good.clone();
    v["txs"][0]["outputs"][0]["value_sat"] = json!(u64::MAX);
    v["txs"][0]["outputs"][0]["is_change"] = json!(false);
    v["txs"][1]["outputs"][0]["value_sat"] = json!(u64::MAX);
    v["txs"][1]["outputs"][0]["is_change"] = json!(false);
    cases.push(("output u64::MAX", v));
    let mut v = good.clone();
    v["txs"][0]["fee_sat"] = json!(u64::MAX);
    v["txs"][1]["fee_sat"] = json!(u64::MAX);
    cases.push(("fee u64::MAX", v));
    let mut v = good.clone();
    let over = Amount::MAX_MONEY.to_sat() / 2 + 1;
    v["txs"][0]["inputs"][0]["value_sat"] = json!(over);
    v["txs"][1]["inputs"][0]["value_sat"] = json!(over);
    cases.push(("input che insieme superano tutti i bitcoin", v));

    for (what, value) in cases {
        let path = temp.json("batch.json", &value);
        let error = verify::load(&path).expect_err(what);
        assert!(error.to_string().contains("batch.json"), "{what}: {error}");
    }
}

/// Two entries with the same index: broadcasting "#N" could send another.
#[test]
fn a_batch_with_a_repeated_index_is_refused_at_load() {
    let temp = Temp::new("batch-indici");
    let (_, good) = signed_batch();
    let mut v = good.clone();
    v["txs"][1]["index"] = v["txs"][0]["index"].clone();
    assert!(verify::load(&temp.json("batch.json", &v)).is_err());
}

/// The bytes may claim anything; their sums must not overflow.
#[test]
fn transaction_bytes_with_impossible_outputs_fail_verification() {
    use plebline_core::bitcoin::consensus::encode::deserialize_hex;
    use plebline_core::bitcoin::Transaction;

    let temp = Temp::new("batch-byte");
    let (_, good) = signed_batch();
    let mut tx: Transaction =
        deserialize_hex(good["txs"][0]["hex"].as_str().expect("testo")).expect("hex buono");
    for output in &mut tx.output {
        output.value = Amount::from_sat(u64::MAX);
    }
    let mut v = good.clone();
    v["txs"][0]["hex"] = json!(serialize_hex(&tx));
    v["txs"][0]["txid"] = json!(tx.compute_txid().to_string());
    let report = verified(&temp, &v).expect("si carica: i numeri del file sono plausibili");
    assert!(!report.is_ok());
}

/// The index is shown counted from one: the largest must not overflow.
#[test]
fn the_largest_index_is_shown_without_overflow() {
    let temp = Temp::new("batch-indice");
    let (_, good) = signed_batch();
    let mut v = good.clone();
    v["txs"][0]["index"] = json!(usize::MAX);
    let report = verified(&temp, &v);
    if let Some(report) = report {
        // Kept, and shown without wrapping round to zero.
        let shown = report.checks.iter().any(|c| {
            c.kind == verify::CheckKind::Txid(usize::MAX)
                && c.name.contains(&usize::MAX.to_string())
        });
        assert!(shown, "{:?}", report.checks);
    }
}

#[test]
fn long_labels_and_extra_fields_are_harmless() {
    let temp = Temp::new("batch-etichette");
    let (signed, good) = signed_batch();
    let mut v = good.clone();
    v["txs"][0]["label"] = json!("x".repeat(1_000_000));
    v["txs"][0]["campo_nuovo"] = json!({"annidato": [1, 2, 3]});
    v["campo_nuovo"] = json!(true);
    let report = verified(&temp, &v).expect("si carica");
    assert!(report.is_ok(), "la nota non tocca le transazioni");
    let loaded = verify::load(&temp.json("batch.json", &v)).expect("si carica");
    assert_eq!(loaded.txs.len(), signed.txs.len());
    assert_eq!(loaded.txs[1].hex, signed.txs[1].hex);
}

#[test]
fn a_config_or_plan_with_wrong_types_is_refused() {
    let temp = Temp::new("config");
    let config = json!({
        "network": "regtest",
        "fee_rate_sat_vb": 5.0,
        "entries": [{"recipient": destination(0), "amount_sat": 1000}]
    });
    read_json::<BatchConfig>(&temp.json("c.json", &config)).expect("buona");

    for (what, field, value) in [
        ("importo negativo", "amount_sat", json!(-5)),
        ("importo come testo", "amount_sat", json!("1000")),
        ("importo oltre u64", "amount_sat", json!(1e20)),
        ("destinatario numerico", "recipient", json!(42)),
    ] {
        let mut v = config.clone();
        v["entries"][0][field] = value;
        assert!(
            read_json::<BatchConfig>(&temp.json("c.json", &v)).is_err(),
            "accettato: {what}"
        );
    }
    let mut v = config.clone();
    v["fee_rate_sat_vb"] = json!("cinque");
    assert!(read_json::<BatchConfig>(&temp.json("c.json", &v)).is_err());

    let (_, good) = signed_batch();
    // A batch is not a plan.
    assert!(read_json::<PlannedBatch>(&temp.json("p.json", &good)).is_err());
}

/// Impossible requests in a batch definition are refused while building,
/// before any key signs: zero, more than exists, a fee rate no one would pay.
#[test]
fn an_impossible_definition_is_refused_while_building() {
    let entry = |amount_sat| BatchEntry {
        label: None,
        recipient: destination(0),
        amount_sat,
    };
    let cases = [
        ("zero", 5.0, entry(0)),
        ("u64::MAX", 5.0, entry(u64::MAX)),
        (
            "oltre tutti i bitcoin",
            5.0,
            entry(Amount::MAX_MONEY.to_sat() + 1),
        ),
        ("fee rate enorme", 1e18, entry(10_000)),
        ("fee rate oltre u64", 1e300, entry(10_000)),
        ("fee rate sotto il minimo", 0.5, entry(10_000)),
        (
            "indirizzo misto maiuscolo",
            5.0,
            BatchEntry {
                label: None,
                recipient: {
                    let a = destination(0);
                    format!("{}{}", a[..6].to_ascii_uppercase(), &a[6..])
                },
                amount_sat: 10_000,
            },
        ),
        (
            "indirizzo di mainnet",
            5.0,
            BatchEntry {
                label: None,
                recipient: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".into(),
                amount_sat: 10_000,
            },
        ),
    ];
    for (what, fee_rate_sat_vb, entry) in cases {
        let (mut wallet, _) = common::hydrated(wallet(), &[(0, 200_000, 1)]);
        let config = BatchConfig {
            network: NETWORK.to_string(),
            fee_rate_sat_vb,
            entries: vec![entry],
        };
        assert!(
            plan::build(&mut wallet, &config, []).is_err(),
            "costruito: {what}"
        );
    }
}
