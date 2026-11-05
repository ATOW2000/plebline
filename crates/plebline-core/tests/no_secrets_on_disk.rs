//! No secret on disk: every artifact is scanned for seed words as text and
//! extended private keys as bytes (base64 would hide them from a text search).

use std::str::FromStr;

use plebline_core::batch;
use plebline_core::bitcoin::Psbt;
use plebline_core::plan::{self, BatchConfig, BatchEntry};
use plebline_core::split::{self, SplitConfig};
use plebline_core::wallet::public_descriptors;
use plebline_core::watch::{WatchFile, WATCH_SCHEMA_VERSION};
use plebline_core::{bdk_wallet::KeychainKind, bdk_wallet::Wallet};

mod common;
use common::{NETWORK, SEED};

const PASSPHRASE: &str = "una passphrase che non deve comparire da nessuna parte";

/// xprv/tprv version bytes, found whatever the file encoding.
const XPRV_MAGIC: [u8; 4] = [0x04, 0x88, 0xAD, 0xE4];
const TPRV_MAGIC: [u8; 4] = [0x04, 0x35, 0x83, 0x94];

const FUNDING: [(u32, u64, u8); 3] = [(0, 500_000, 1), (1, 500_000, 2), (2, 500_000, 3)];

fn test_wallet() -> Wallet {
    common::wallet(SEED, Some(PASSPHRASE))
}

fn prepared() -> Wallet {
    common::hydrated(test_wallet(), &FUNDING).0
}

/// Checks a textual artifact: no seed words, no passphrase, no private keys in
/// the clear.
fn assert_no_secrets(name: &str, content: &str) {
    let lower = content.to_lowercase();

    assert!(!lower.contains(SEED), "{name} contiene il seed per intero");
    assert!(
        !lower.contains("abandon abandon"),
        "{name} contiene una sequenza di parole del seed"
    );
    assert!(
        !content.contains(PASSPHRASE),
        "{name} contiene la passphrase"
    );
    assert!(!content.contains("xprv"), "{name} contiene una xprv");
    assert!(!content.contains("tprv"), "{name} contiene una tprv");
    assert!(
        !lower.contains("private"),
        "{name} nomina qualcosa di privato: da guardare a mano"
    );
}

/// Checks the decoded bytes of a PSBT.
fn assert_psbt_carries_no_private_key(name: &str, base64: &str) {
    let psbt = Psbt::from_str(base64).expect("la PSBT scritta deve essere rileggibile");
    let bytes = psbt.serialize();

    for magic in [XPRV_MAGIC, TPRV_MAGIC] {
        assert!(
            !bytes.windows(4).any(|w| w == magic),
            "{name} contiene una chiave estesa privata nei byte della PSBT"
        );
    }
    assert!(
        psbt.xpub.keys().len() < 1_000,
        "{name}: numero di xpub implausibile, la struttura non è quella attesa"
    );
}

#[test]
fn wallet_json_holds_only_public_keys() {
    let wallet = test_wallet();
    let (external, internal) = public_descriptors(&wallet);

    let file = WatchFile {
        version: WATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        external,
        internal,
        descriptor_checksum: wallet.descriptor_checksum(KeychainKind::External),
        created_by: "test".into(),
    };
    let json = serde_json::to_string_pretty(&file).expect("serializzabile");

    assert_no_secrets("wallet.json", &json);
    // And it must contain what is needed: an extended public key.
    assert!(
        json.contains("tpub"),
        "wallet.json dovrebbe contenere la chiave estesa pubblica"
    );
}

#[test]
fn plan_json_holds_no_secrets() {
    let mut wallet = prepared();
    let config = BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 2.0,
        entries: vec![BatchEntry {
            label: Some("uscita".into()),
            recipient: wallet
                .peek_address(KeychainKind::External, 50)
                .address
                .to_string(),
            amount_sat: 100_000,
        }],
    };

    let built = plan::build(&mut wallet, &config, []).expect("piano costruibile");
    let planned = plan::to_plan(&built, &config, 1_700_000_000, 300);
    let json = serde_json::to_string_pretty(&planned).expect("serializzabile");

    assert_no_secrets("plan.json", &json);
}

#[test]
fn batch_json_holds_no_secrets_not_even_in_the_psbts() {
    let mut wallet = prepared();
    let config = BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 2.0,
        entries: (0..2)
            .map(|i| BatchEntry {
                label: Some(format!("uscita {i}")),
                recipient: wallet
                    .peek_address(KeychainKind::External, 50 + i)
                    .address
                    .to_string(),
                amount_sat: 100_000,
            })
            .collect(),
    };

    let built = plan::build(&mut wallet, &config, []).expect("piano costruibile");
    let signed = batch::sign(&wallet, built, config.fee_rate_sat_vb).expect("firma riuscita");
    let json = serde_json::to_string_pretty(&signed).expect("serializzabile");

    assert_no_secrets("batch.json", &json);
    for tx in &signed.txs {
        assert_psbt_carries_no_private_key("batch.json", &tx.psbt);
    }
}

#[test]
fn split_json_holds_no_secrets() {
    let mut wallet = prepared();
    let config = SplitConfig {
        network: NETWORK.to_string(),
        outputs: 4,
        fee_rate_sat_vb: 2.0,
    };

    let plan = split::build(&mut wallet, &config, []).expect("divisione costruibile");
    let signed = split::sign(&wallet, plan, &config).expect("firma riuscita");
    let json = serde_json::to_string_pretty(&signed).expect("serializzabile");

    assert_no_secrets("split.json", &json);
    for tx in &signed.txs {
        assert_psbt_carries_no_private_key("split.json", &tx.psbt);
    }
}

#[test]
fn utxos_json_holds_no_secrets() {
    // With the coins in it, as the online side writes it.
    let set = common::utxo_set(&test_wallet(), &FUNDING);
    assert!(!set.utxos.is_empty());
    let json = serde_json::to_string_pretty(&set).expect("serializzabile");
    assert_no_secrets("utxos.json", &json);
}

/// Feeds `assert_no_secrets` a descriptor with a tprv and expects a panic,
/// which shows the tests above can catch a leaked key.
#[test]
#[should_panic(expected = "contiene una tprv")]
fn the_check_really_recognizes_a_private_key() {
    assert_no_secrets("finto", "{\"descriptor\":\"wpkh(tprv8ZgxMBicQKsPd...)\"}");
}
