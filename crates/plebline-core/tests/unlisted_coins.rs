//! A source transaction enters the offline wallet whole: its other outputs
//! to the wallet become visible too, though `utxos.json` does not list them
//! (they may be spent already). Neither the plan nor the split spends them.

use plebline_core::bdk_wallet::{KeychainKind, Wallet};
use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::hashes::Hash;
use plebline_core::bitcoin::{
    absolute::LockTime, transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness,
};
use plebline_core::plan::{self, BatchConfig, BatchEntry};
use plebline_core::split::{self, SplitConfig};
use plebline_core::utxos::{self, UtxoEntry, UtxoSet};

mod common;
use common::NETWORK;

/// One source transaction paying the wallet twice: 100 000 sat to receiving
/// address 0, 500 000 sat to receiving address 1.
fn parent_with_two_outputs(wallet: &Wallet) -> Transaction {
    let spk = |index| {
        wallet
            .peek_address(KeychainKind::External, index)
            .address
            .script_pubkey()
    };
    let mut previous = [0u8; 32];
    previous[0] = 9;
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from_byte_array(previous), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![
            TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: spk(0),
            },
            TxOut {
                value: Amount::from_sat(500_000),
                script_pubkey: spk(1),
            },
        ],
    }
}

/// The wallet hydrated from a `utxos.json` that lists only output 0: output
/// 1, the larger, is not vouched for (for instance, already spent by a
/// refill that landed). Returns the wallet, the listed coin and the other.
fn hydrated_listing_only_the_first() -> (Wallet, OutPoint, OutPoint) {
    let mut wallet = common::wallet(common::SEED, None);
    let parent = parent_with_two_outputs(&wallet);
    let txid = parent.compute_txid();
    let set = UtxoSet {
        version: 1,
        network: NETWORK.to_string(),
        generated_at: 1_700_000_000,
        tip_height: 300,
        descriptor_checksum: Some(wallet.descriptor_checksum(KeychainKind::External)),
        last_active_indices: [("external".to_string(), 10)].into_iter().collect(),
        utxos: vec![UtxoEntry {
            outpoint: format!("{txid}:0"),
            value_sat: 100_000,
            script_pubkey: parent.output[0].script_pubkey.to_hex_string(),
            keychain: "external".into(),
            derivation_index: 0,
            confirmations: 144,
            prev_tx_hex: serialize_hex(&parent),
        }],
    };
    let set = utxos::validate(set, NETWORK, "utxos.json").expect("il file è coerente");
    set.hydrate(&mut wallet).expect("idratazione riuscita");
    (wallet, OutPoint::new(txid, 0), OutPoint::new(txid, 1))
}

fn destination(index: u32) -> String {
    common::wallet(common::OTHER_SEED, None)
        .peek_address(KeychainKind::External, index)
        .address
        .to_string()
}

#[test]
fn the_plan_never_spends_a_coin_the_file_does_not_list() {
    let (mut wallet, listed, unlisted) = hydrated_listing_only_the_first();
    let config = BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 2.0,
        entries: vec![BatchEntry {
            label: Some("ricarica".into()),
            recipient: destination(0),
            amount_sat: 50_000,
        }],
    };

    let built = plan::build(&mut wallet, &config, []).expect("costruibile con la moneta elencata");
    let spent: Vec<OutPoint> = built
        .iter()
        .flat_map(|t| t.inputs.iter().map(|(op, _)| *op))
        .collect();
    assert_eq!(
        spent,
        vec![listed],
        "il piano deve spendere solo {listed}, non {unlisted} che utxos.json non elenca"
    );
}

#[test]
fn the_plan_does_not_reach_for_an_unlisted_coin_when_the_listed_ones_are_short() {
    let (mut wallet, _, _) = hydrated_listing_only_the_first();
    let config = BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 2.0,
        entries: vec![BatchEntry {
            label: None,
            recipient: destination(0),
            amount_sat: 300_000,
        }],
    };
    assert!(
        plan::build(&mut wallet, &config, []).is_err(),
        "300 000 sat ci sono solo contando la moneta che il file non elenca"
    );
}

#[test]
fn the_split_never_spends_a_coin_the_file_does_not_list() {
    let (mut wallet, listed, unlisted) = hydrated_listing_only_the_first();
    let config = SplitConfig {
        network: NETWORK.to_string(),
        outputs: 2,
        fee_rate_sat_vb: 2.0,
    };

    let plan = split::build(&mut wallet, &config, []).expect("divisione costruibile");
    let spent: Vec<OutPoint> = plan.inputs.iter().map(|(op, _)| *op).collect();
    assert_eq!(
        spent,
        vec![listed],
        "la divisione deve spendere solo {listed}, non {unlisted} che utxos.json non elenca"
    );
    assert!(
        plan.total_in() < Amount::from_sat(500_000),
        "il totale non conta la moneta non elencata: {}",
        plan.total_in()
    );
}
