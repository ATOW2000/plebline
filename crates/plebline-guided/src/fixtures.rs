//! Test helpers shared by several modules (`cfg(test)` only).

use plebline::ask::{Reply, Scripted};
use plebline::project::{self, Project};
use plebline_core::batch::{SignedBatch, SignedInput, SignedOutput, SignedTx};
use plebline_core::wallet::SeedWords;

use crate::NETWORK;

/// The two opening answers (Italian, skip the explanation), then `rest`.
pub(crate) fn from_the_opening(rest: impl IntoIterator<Item = Reply>) -> Scripted {
    Scripted::new([Reply::like("Italiano"), Reply::No].into_iter().chain(rest))
}

/// The opening, "I have the words", public server, twelve words, seed, no
/// passphrase.
pub(crate) fn up_to_the_server(rest: impl IntoIterator<Item = Reply>) -> Scripted {
    from_the_opening(
        [
            Reply::like("Ce le ho già"),
            Reply::like("Un server pubblico"),
            Reply::like("12"),
            Reply::seed(SEED12),
            Reply::text(""),
        ]
        .into_iter()
        .chain(rest),
    )
}

/// BIP39 test vector; no funds anywhere.
pub(crate) const SEED12: &str = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";

pub(crate) const SEED24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon abandon abandon art";

/// Tests match on translated text: pin the language.
pub(crate) use plebline::testing::in_italian;

/// The broadcast confirmation: mainnet asks for the amount typed, a test
/// network for a yes (`plebline::online`, `confirm_broadcast`).
pub(crate) fn send_it(amount: &str) -> Reply {
    if NETWORK == plebline_core::bitcoin::Network::Bitcoin {
        Reply::text(amount)
    } else {
        Reply::Yes
    }
}

/// Every scripted answer was used, so the flow took the path the test
/// expects.
#[track_caller]
pub(crate) fn assert_script_used_up(ask: &Scripted) {
    assert_eq!(ask.remaining(), 0, "il flusso ha preso un'altra strada");
}

pub(crate) use plebline::testing::Temp;

/// The ceremony folder of a test: its temporary folder.
pub(crate) trait Ceremony {
    fn ceremony(&self) -> Project;
}

impl Ceremony for Temp {
    fn ceremony(&self) -> Project {
        Project {
            name: "prova".to_string(),
            dir: self.path.clone(),
        }
    }
}

/// A fake signed group, with whichever indexes you want.
pub(crate) fn write_batch(ceremony: &Project, indexes: &[usize]) {
    write_group(ceremony, project::BATCH, indexes);
}

/// The same fake group under another signed name (`batch-1.json`,
/// `split.json`…). Txids follow the indexes, as in [`fake_txid`].
pub(crate) fn write_group(ceremony: &Project, file: &str, indexes: &[usize]) {
    let batch = SignedBatch {
        version: plebline_core::batch::BATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        created_by: "test".into(),
        descriptor_checksum: "aaaaaaaa".into(),
        fee_rate_sat_vb: 1.0,
        txs: indexes
            .iter()
            .map(|i| SignedTx {
                index: *i,
                label: Some(format!("pagamento {i}")),
                txid: format!("{i:064}"),
                hex: plebline::testing::spending_hex(&format!("{i:064}:0")),
                psbt: "cHNidP8=".into(),
                inputs: vec![SignedInput {
                    outpoint: format!("{i:064}:0"),
                    value_sat: 100_000,
                }],
                outputs: vec![SignedOutput {
                    address: "bc1qqqqq".into(),
                    // Distinct amounts, distinct labels.
                    value_sat: 90_000 + *i as u64,
                    is_change: false,
                }],
                fee_sat: 1_000,
                fee_rate_sat_vb: 1.0,
                vsize: 141,
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&batch).expect("serializzabile");
    std::fs::write(ceremony.path(file), json).expect("scrivibile");
}

/// Another wallet, for destinations outside the cold storage.
pub(crate) const SEED_OF_ANOTHER: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";

pub(crate) fn test_wallet(seed: &str) -> plebline_core::bdk_wallet::Wallet {
    let seed = SeedWords::from_interactive_input(seed).expect("seed valido");
    plebline_core::wallet::from_seed(&seed, None, NETWORK).expect("wallet derivabile")
}

/// `wallet.json` as the first run would leave it: the public half only.
pub(crate) fn write_wallet(ceremony: &Project) -> plebline_core::bdk_wallet::Wallet {
    let wallet = test_wallet(SEED12);
    let (external, internal) = plebline_core::wallet::public_descriptors(&wallet);
    let file = plebline_core::watch::WatchFile {
        version: plebline_core::watch::WATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        external,
        internal,
        descriptor_checksum: fingerprint(&wallet),
        created_by: "test".into(),
    };
    std::fs::write(
        ceremony.path(project::WALLET),
        serde_json::to_string_pretty(&file).expect("serializzabile"),
    )
    .expect("scrivibile");
    wallet
}

pub(crate) fn fingerprint(wallet: &plebline_core::bdk_wallet::Wallet) -> String {
    wallet.descriptor_checksum(plebline_core::bdk_wallet::KeychainKind::External)
}

/// A coin the wallet recognizes: a synthetic but real source transaction,
/// validated against its bytes like any other.
pub(crate) fn coin(
    wallet: &plebline_core::bdk_wallet::Wallet,
    index: u32,
    sats: u64,
    seed: u8,
) -> plebline::chain::Coin {
    use plebline_core::bdk_wallet::KeychainKind;
    use plebline_core::bitcoin::consensus::encode::serialize_hex;

    let spk = wallet
        .peek_address(KeychainKind::External, index)
        .address
        .script_pubkey();
    let tx = plebline::testing::paying(seed, &[(spk, sats)]);

    plebline::chain::Coin {
        outpoint: format!("{}:0", tx.compute_txid()),
        value_sat: sats,
        script_pubkey: tx.output[0].script_pubkey.to_hex_string(),
        keychain: "external".into(),
        derivation_index: index,
        height: Some(799_900),
        origin: Some(plebline::chain::Origin {
            hex: serialize_hex(&tx),
            coinbase: false,
        }),
    }
}

/// `utxos.json` with the wallet's two coins, as the snapshot writes it.
pub(crate) fn snapshot_of_two(ceremony: &Project) -> Vec<String> {
    snapshot_with(ceremony, &[(0, 200_000, 1), (1, 300_000, 2)])
}

/// `utxos.json` with the wallet's coins `(index, sats, seed)`, as the
/// snapshot writes it.
pub(crate) fn snapshot_with(ceremony: &Project, held: &[(u32, u64, u8)]) -> Vec<String> {
    let wallet = write_wallet(ceremony);
    let coins: Vec<_> = held
        .iter()
        .map(|(index, sats, seed)| coin(&wallet, *index, *sats, *seed))
        .collect();
    let outpoints = coins.iter().map(|c| c.outpoint.clone()).collect();
    let mut scanned = plebline::chain::ScanResult::empty(800_000, fingerprint(&wallet));
    for c in coins {
        scanned = scanned.with(c);
    }
    let mut chain = plebline::chain::Fake::new().with_scan(scanned);
    plebline::online::export_utxos(
        NETWORK,
        "http://server-finto",
        plebline::online::ExportInput {
            wallet: ceremony.path(project::WALLET),
            out: ceremony.path(project::UTXOS),
        },
        &mut chain,
    )
    .expect("utxos.json scritto");
    outpoints
}

/// Somebody else's address, to send the payments to.
pub(crate) fn recipient(index: u32) -> String {
    test_wallet(SEED_OF_ANOTHER)
        .peek_address(plebline_core::bdk_wallet::KeychainKind::External, index)
        .address
        .to_string()
}

pub(crate) fn fake_txid(index: usize) -> String {
    format!("{index:064}")
}

/// Last word swapped: a bad checksum, as when trying the wrong candidate.
pub(crate) const SEED12_BROKEN: &str = "abandon abandon abandon abandon abandon abandon \
                             abandon abandon abandon abandon abandon zoo";
