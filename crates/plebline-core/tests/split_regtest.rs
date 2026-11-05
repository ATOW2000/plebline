//! Split on a synthetic chain: the amounts add up, every output stays in the
//! wallet, and the new coins are enough for N independent payments.

use std::collections::BTreeSet;

use plebline_core::bdk_wallet::Wallet;
use plebline_core::bitcoin::{Amount, OutPoint};
use plebline_core::split::{self, SplitConfig};

mod common;
use common::NETWORK;

fn prepared(funding: &[(u32, u64, u8)]) -> Wallet {
    common::hydrated(common::wallet(common::SEED, None), funding).0
}

fn config(outputs: usize) -> SplitConfig {
    SplitConfig {
        network: NETWORK.to_string(),
        outputs,
        fee_rate_sat_vb: 2.0,
    }
}

#[test]
fn from_four_coins_to_six() {
    let mut wallet = prepared(&[
        (0, 500_000, 1),
        (1, 500_000, 2),
        (2, 500_000, 3),
        (3, 500_000, 4),
    ]);

    let plan = split::build(&mut wallet, &config(6), []).expect("divisione costruibile");

    assert_eq!(plan.inputs.len(), 4, "devono entrare tutte le monete");
    assert_eq!(plan.outputs.len(), 6, "devono uscire sei monete");
}

#[test]
fn the_numbers_add_up_to_the_satoshi() {
    let mut wallet = prepared(&[(0, 500_000, 1), (1, 300_000, 2)]);
    let plan = split::build(&mut wallet, &config(5), []).expect("divisione costruibile");

    assert_eq!(
        plan.total_in(),
        plan.total_out() + plan.fee,
        "input {} ≠ output {} + fee {}",
        plan.total_in(),
        plan.total_out(),
        plan.fee
    );
}

#[test]
fn every_output_belongs_to_the_wallet() {
    let mut wallet = prepared(&[(0, 1_000_000, 1)]);
    let plan = split::build(&mut wallet, &config(4), []).expect("divisione costruibile");

    for (address, _) in &plan.outputs {
        assert!(
            wallet.is_mine(address.script_pubkey()),
            "{address} non appartiene al wallet"
        );
    }
}

/// A coin inside a signed payment is left out, because sending the split
/// would otherwise cancel that payment silently.
#[test]
fn the_split_leaves_committed_coins_alone() {
    let mut wallet = prepared(&[(0, 500_000, 1), (1, 500_000, 2), (2, 500_000, 3)]);
    let held = wallet
        .list_unspent()
        .map(|u| u.outpoint)
        .min()
        .expect("tre monete");

    let plan = split::build(&mut wallet, &config(4), [held]).expect("divisione costruibile");

    assert_eq!(plan.inputs.len(), 2, "entrano solo le due monete libere");
    assert!(
        plan.inputs.iter().all(|(op, _)| *op != held),
        "la moneta impegnata non deve entrare nella divisione"
    );
    assert_eq!(plan.total_in().to_sat(), 1_000_000);
}

#[test]
fn with_every_coin_committed_there_is_nothing_to_split() {
    let mut wallet = prepared(&[(0, 500_000, 1), (1, 500_000, 2)]);
    let all: Vec<OutPoint> = wallet.list_unspent().map(|u| u.outpoint).collect();

    assert!(split::build(&mut wallet, &config(4), all).is_err());
}

/// Reusing an address would link independent payments.
#[test]
fn the_addresses_produced_are_all_different() {
    let mut wallet = prepared(&[(0, 1_000_000, 1)]);
    let plan = split::build(&mut wallet, &config(6), []).expect("divisione costruibile");

    let distinct: BTreeSet<String> = plan.outputs.iter().map(|(a, _)| a.to_string()).collect();
    assert_eq!(distinct.len(), plan.outputs.len());
}

/// `max_outputs` is exact: the limit it reports can be built, one more
/// cannot.
#[test]
fn the_declared_cap_is_the_real_one() {
    let funding = [(0, 100_000u64, 1u8)];
    let mut wallet = prepared(&[(funding[0].0, funding[0].1, funding[0].2)]);
    let total = Amount::from_sat(funding[0].1);

    let limit = split::max_outputs(total, 1, 2.0);
    assert!(limit >= 2, "con 100_000 sat ci si deve poter dividere");

    let mut at_limit = prepared(&[(0, 100_000, 1)]);
    split::build(&mut at_limit, &config(limit), []).expect("il limite dichiarato deve funzionare");

    let over = split::build(&mut wallet, &config(limit + 1), []);
    assert!(
        over.is_err(),
        "oltre il limite la costruzione deve fallire, non produrre polvere"
    );
}

#[test]
fn fewer_than_two_coins_is_refused() {
    let mut wallet = prepared(&[(0, 1_000_000, 1)]);
    assert!(split::build(&mut wallet, &config(1), []).is_err());
    assert!(split::build(&mut wallet, &config(0), []).is_err());
}

/// The signed transaction must be finalized and broadcastable, and the txid
/// must not change between building and signing.
#[test]
fn the_signed_split_is_broadcastable() {
    let mut wallet = prepared(&[(0, 500_000, 1), (1, 500_000, 2)]);
    let config = config(4);
    let plan = split::build(&mut wallet, &config, []).expect("divisione costruibile");
    let expected_txid = plan.txid.to_string();
    let expected_outputs = plan.outputs.len();

    let signed = split::sign(&wallet, plan, &config).expect("firma riuscita");

    assert_eq!(signed.txs.len(), 1, "la divisione è una transazione sola");
    let tx = &signed.txs[0];
    assert_eq!(tx.txid, expected_txid, "il txid non deve cambiare firmando");
    assert_eq!(tx.outputs.len(), expected_outputs);
    assert!(
        tx.outputs.iter().all(|o| o.is_change),
        "nella divisione ogni moneta torna al portafoglio"
    );
    assert!(!tx.hex.is_empty());
}

#[test]
fn after_the_split_there_are_enough_coins_for_the_batch() {
    let mut wallet = prepared(&[(0, 1_000_000, 1)]);
    let plan = split::build(&mut wallet, &config(6), []).expect("divisione costruibile");

    // Six outputs in the transaction itself, each a coin of this wallet that
    // a refill can spend on its own.
    let paid = &plan.psbt.unsigned_tx.output;
    assert_eq!(paid.len(), 6);
    assert!(
        paid.iter().all(|o| wallet.is_mine(o.script_pubkey.clone())),
        "ogni moneta nuova è del portafoglio"
    );

    for (_, value) in &plan.outputs {
        assert!(
            value.to_sat() >= split::MIN_USEFUL_OUTPUT_SAT,
            "una moneta da {value} è sotto la soglia utile"
        );
    }
}

/// The fee rate shown is the one paid: the unsigned size underestimates it.
#[test]
fn the_price_shown_is_the_price_paid() {
    let mut wallet = prepared(&[(0, 500_000, 1), (1, 500_000, 2), (2, 500_000, 3)]);
    let config = config(5);

    let plan = split::build(&mut wallet, &config, []).expect("divisione costruibile");
    let shown = plan.fee.to_sat() as f64 / plan.vsize as f64;

    let signed = split::sign(&wallet, plan, &config).expect("firma riuscita");
    let real = signed.txs[0].fee_rate_sat_vb;

    let gap = (shown - real).abs() / real;
    assert!(
        gap < 0.05,
        "a schermo {shown:.2} sat/vB, in realtà {real:.2} sat/vB: scarto del {:.0}%",
        gap * 100.0
    );
}
