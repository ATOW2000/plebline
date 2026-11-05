//! Full path on regtest: utxos.json → plan → sign → verify, with synthetic
//! source transactions.

use std::collections::BTreeSet;

use plebline_core::batch;
use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::{Network, OutPoint};
use plebline_core::plan::{self, BatchConfig, BatchEntry};
use plebline_core::utxos;
use plebline_core::verify::{self, Expectations};
use plebline_core::{bdk_wallet::KeychainKind, bdk_wallet::Wallet, Error};

mod common;
use common::{utxo_set, NETWORK};

fn destination(index: u32) -> String {
    common::wallet(common::OTHER_SEED, None)
        .peek_address(KeychainKind::External, index)
        .address
        .to_string()
}

fn test_wallet() -> Wallet {
    common::wallet(common::SEED, None)
}

fn config(count: usize, amount_sat: u64) -> BatchConfig {
    BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 5.0,
        entries: (0..count)
            .map(|i| BatchEntry {
                label: Some(format!("uscita {i}")),
                recipient: destination(i as u32),
                amount_sat,
            })
            .collect(),
    }
}

fn prepared(funding: &[(u32, u64, u8)]) -> (Wallet, utxos::ValidatedUtxoSet) {
    common::hydrated(test_wallet(), funding)
}

#[test]
fn the_whole_batch_is_disjoint_and_verifiable() {
    let (mut wallet, set) = prepared(&[(0, 200_000, 1), (1, 200_000, 2), (2, 200_000, 3)]);
    assert_eq!(wallet.list_unspent().count(), 3);

    let config = config(3, 150_000);
    let built = plan::build(&mut wallet, &config, []).expect("batch costruibile");
    assert_eq!(built.len(), 3);

    // No two transactions share an input, checked on the built data rather
    // than on an error message.
    let mut all_inputs: Vec<OutPoint> = Vec::new();
    for tx in &built {
        assert_eq!(
            tx.inputs.len(),
            1,
            "ogni tx dovrebbe consumare un solo UTXO"
        );
        all_inputs.extend(tx.inputs.iter().map(|(op, _)| *op));
    }
    let distinct: BTreeSet<&OutPoint> = all_inputs.iter().collect();
    assert_eq!(
        distinct.len(),
        all_inputs.len(),
        "input condivisi fra le tx"
    );

    let planned = plan::to_plan(&built, &config, set.generated_at, set.tip_height);
    let signed = batch::sign(&wallet, built, config.fee_rate_sat_vb).expect("firma riuscita");

    let report = verify::verify(
        &signed,
        &Expectations {
            network: Some(NETWORK),
            recipients: Some(config.entries.iter().map(|e| e.recipient.clone()).collect()),
            spendable: Some(set.utxos.iter().map(|u| u.outpoint).collect()),
            wallet_descriptors: Some(plebline_core::wallet::public_descriptors(&wallet)),
            ownership_depth: None,
        },
    );
    assert!(
        report.is_ok(),
        "verifica fallita: {:?}",
        report.failures().collect::<Vec<_>>()
    );

    // The plan must describe what was signed.
    for (planned_tx, signed_tx) in planned.txs.iter().zip(signed.txs.iter()) {
        assert_eq!(planned_tx.txid, signed_tx.txid);
        assert_eq!(planned_tx.fee_sat, signed_tx.fee_sat);
    }

    // The exported PSBT must be valid and refer to the same transaction.
    for tx in &signed.txs {
        let psbt: plebline_core::bitcoin::Psbt = tx.psbt.parse().expect("PSBT base64 valido");
        assert_eq!(psbt.unsigned_tx.compute_txid().to_string(), tx.txid);
    }
}

#[test]
fn selection_is_reproducible_between_plan_and_signing() {
    let funding = [(0, 200_000, 1), (1, 200_000, 2), (2, 300_000, 3)];
    let config = config(2, 150_000);

    let (mut first_wallet, set) = prepared(&funding);
    let first = plan::to_plan(
        &plan::build(&mut first_wallet, &config, []).unwrap(),
        &config,
        set.generated_at,
        set.tip_height,
    );

    let (mut second_wallet, _) = prepared(&funding);
    let second = plan::to_plan(
        &plan::build(&mut second_wallet, &config, []).unwrap(),
        &config,
        set.generated_at,
        set.tip_height,
    );

    assert_eq!(first.inputs_by_tx(), second.inputs_by_tx());
    assert_eq!(
        first.txs.iter().map(|t| &t.txid).collect::<Vec<_>>(),
        second.txs.iter().map(|t| &t.txid).collect::<Vec<_>>()
    );
}

#[test]
fn more_txs_than_utxos_fails_instead_of_reusing() {
    // Two UTXOs, three transactions: the third must fail, not reuse a
    // committed input.
    let (mut wallet, _) = prepared(&[(0, 200_000, 1), (1, 200_000, 2)]);
    let error = plan::build(&mut wallet, &config(3, 150_000), []).unwrap_err();

    match error {
        Error::BuildTx { index, reason } => {
            assert_eq!(index, 2, "avrebbe dovuto fallire sulla terza transazione");
            assert!(
                reason.to_lowercase().contains("insufficient")
                    || reason.to_lowercase().contains("fondi"),
                "atteso un errore di fondi insufficienti, trovato: {reason}"
            );
        }
        other => panic!("atteso BuildTx, trovato {other:?}"),
    }
}

#[test]
fn a_single_large_utxo_is_not_spent_twice() {
    // Without exclusion, both transactions would pick the one large UTXO.
    let (mut wallet, _) = prepared(&[(0, 1_000_000, 1)]);
    let error = plan::build(&mut wallet, &config(2, 100_000), []).unwrap_err();
    assert!(
        matches!(error, Error::BuildTx { index: 1, .. }),
        "{error:?}"
    );
}

#[test]
fn a_utxos_json_lying_about_the_amount_is_refused() {
    let wallet = test_wallet();
    let mut set = utxo_set(&wallet, &[(0, 200_000, 1)]);
    set.utxos[0].value_sat = 999_999_999;

    let error = utxos::validate(set, NETWORK, "utxos.json").unwrap_err();
    match error {
        Error::InvalidUtxo { reason, .. } => assert!(reason.contains("value_sat")),
        other => panic!("atteso InvalidUtxo, trovato {other:?}"),
    }
}

#[test]
fn a_utxos_json_with_a_foreign_prev_tx_is_refused() {
    let wallet = test_wallet();
    let mut set = utxo_set(&wallet, &[(0, 200_000, 1), (1, 200_000, 2)]);
    // Moves the second entry's transaction onto the first: the txid does not
    // match.
    set.utxos[0].prev_tx_hex = set.utxos[1].prev_tx_hex.clone();

    let error = utxos::validate(set, NETWORK, "utxos.json").unwrap_err();
    match error {
        Error::InvalidUtxo { reason, .. } => assert!(reason.contains("txid")),
        other => panic!("atteso InvalidUtxo, trovato {other:?}"),
    }
}

#[test]
fn a_utxo_not_owned_by_the_wallet_is_refused_on_hydration() {
    // A set built for a different wallet: the scripts are not ours.
    let other = common::wallet("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong", None);
    let set = utxos::validate(utxo_set(&other, &[(0, 200_000, 1)]), NETWORK, "utxos.json")
        .expect("il file è internamente coerente");

    let mut wallet = test_wallet();
    let error = set.hydrate(&mut wallet).unwrap_err();
    match error {
        Error::InvalidUtxo { reason, .. } => assert!(reason.contains("non lo riconosce")),
        other => panic!("atteso InvalidUtxo, trovato {other:?}"),
    }
}

#[test]
fn the_wrong_network_is_refused() {
    let wallet = test_wallet();
    let set = utxo_set(&wallet, &[(0, 200_000, 1)]);
    let error = utxos::validate(set, Network::Bitcoin, "utxos.json").unwrap_err();
    assert!(matches!(error, Error::NetworkMismatch { .. }), "{error:?}");
}

/// The case the next test guards against: without the first ceremony's
/// commitments, the second picks the same coins, and only one of the two
/// payments could ever confirm.
#[test]
fn without_memory_two_sessions_pick_the_same_coins() {
    let (mut wallet, _) = prepared(&[(0, 200_000, 1), (1, 200_000, 2), (2, 200_000, 3)]);

    let first = plan::build(&mut wallet, &config(1, 150_000), []).expect("prima cerimonia");
    let second = plan::build(&mut wallet, &config(1, 150_000), []).expect("seconda cerimonia");

    let a: BTreeSet<OutPoint> = first
        .iter()
        .flat_map(|t| t.inputs.iter().map(|(op, _)| *op))
        .collect();
    let b: BTreeSet<OutPoint> = second
        .iter()
        .flat_map(|t| t.inputs.iter().map(|(op, _)| *op))
        .collect();

    assert_eq!(
        a, b,
        "senza impegni la seconda sessione dovrebbe scegliere le stesse monete della prima"
    );
}

#[test]
fn with_memory_the_two_sessions_are_disjoint() {
    let (mut wallet, _) = prepared(&[(0, 200_000, 1), (1, 200_000, 2), (2, 200_000, 3)]);

    let first = plan::build(&mut wallet, &config(1, 150_000), []).expect("prima cerimonia");
    let committed: BTreeSet<OutPoint> = first
        .iter()
        .flat_map(|t| t.inputs.iter().map(|(op, _)| *op))
        .collect();

    let second = plan::build(&mut wallet, &config(1, 150_000), committed.iter().copied())
        .expect("seconda cerimonia");
    let used: BTreeSet<OutPoint> = second
        .iter()
        .flat_map(|t| t.inputs.iter().map(|(op, _)| *op))
        .collect();

    assert!(
        committed.is_disjoint(&used),
        "la seconda cerimonia ha riusato una moneta già impegnata: {committed:?} ∩ {used:?}"
    );

    // And the two together stay broadcastable in any order.
    let spending: Vec<_> = first
        .iter()
        .chain(second.iter())
        .enumerate()
        .map(|(index, tx)| {
            (
                index,
                tx.txid,
                tx.inputs.iter().map(|(op, _)| *op).collect(),
            )
        })
        .collect();
    plan::check_disjoint(&spending)
        .expect("i due batch insieme devono restare mutuamente indipendenti");
}

#[test]
fn once_free_coins_run_out_building_fails() {
    let (mut wallet, _) = prepared(&[(0, 200_000, 1), (1, 200_000, 2)]);

    let first = plan::build(&mut wallet, &config(2, 150_000), []).expect("prima cerimonia");
    let committed: BTreeSet<OutPoint> = first
        .iter()
        .flat_map(|t| t.inputs.iter().map(|(op, _)| *op))
        .collect();
    assert_eq!(committed.len(), 2, "la prima cerimonia impegna tutto");

    let error = plan::build(&mut wallet, &config(1, 150_000), committed.iter().copied())
        .expect_err("senza monete libere non si può costruire nulla");
    assert!(
        matches!(error, Error::BuildTx { .. }),
        "atteso un errore di costruzione, non {error:?}"
    );
}

#[test]
fn the_shareable_psbt_is_the_signed_transaction_and_nothing_more() {
    let (mut wallet, _set) = prepared(&[(0, 200_000, 1), (1, 200_000, 2)]);
    let config = config(2, 150_000);
    let built = plan::build(&mut wallet, &config, []).expect("batch costruibile");
    let signed = batch::sign(&wallet, built, config.fee_rate_sat_vb).expect("firma riuscita");

    for tx in &signed.txs {
        let full: plebline_core::bitcoin::Psbt = tx.psbt.parse().expect("PSBT base64 valido");
        let shared = batch::shareable_psbt(tx).expect("PSBT condivisibile");
        let psbt: plebline_core::bitcoin::Psbt = shared.parse().expect("PSBT base64 valido");

        // The same bytes as the hex: what is handed over is what was signed.
        assert_eq!(
            serialize_hex(&psbt.clone().extract_tx().expect("estraibile")),
            tx.hex
        );

        for (input, spent) in psbt.inputs.iter().zip(&tx.inputs) {
            // The amount stays, so whoever receives it can show the fee.
            let utxo = input.witness_utxo.as_ref().expect("l'importo resta");
            assert_eq!(utxo.value.to_sat(), spent.value_sat);
            assert!(input.final_script_witness.is_some(), "la firma resta");
            assert!(
                input.non_witness_utxo.is_none(),
                "la transazione sorgente esce"
            );
            assert!(input.bip32_derivation.is_empty(), "i percorsi escono");
        }

        // The full PSBT does carry the source transactions, so the
        // `non_witness_utxo` check above tests a real removal.
        assert!(
            full.inputs.iter().all(|i| i.non_witness_utxo.is_some()),
            "il PSBT completo dovrebbe portare le transazioni sorgente"
        );
        // Finalizing drops the paths already; pinned in case BDK changes.
        for output in &psbt.outputs {
            assert!(
                output.bip32_derivation.is_empty(),
                "il resto non si riconosce"
            );
        }
        assert!(psbt.xpub.is_empty());

        assert!(
            shared.len() < tx.psbt.len(),
            "ripulito deve pesare meno: {} contro {}",
            shared.len(),
            tx.psbt.len()
        );
    }
}

#[test]
fn a_psbt_that_does_not_match_the_hex_is_refused() {
    let (mut wallet, _set) = prepared(&[(0, 200_000, 1), (1, 200_000, 2)]);
    let config = config(2, 150_000);
    let built = plan::build(&mut wallet, &config, []).expect("batch costruibile");
    let signed = batch::sign(&wallet, built, config.fee_rate_sat_vb).expect("firma riuscita");

    let mut swapped = signed.txs[0].clone();
    swapped.psbt = signed.txs[1].psbt.clone();
    assert!(batch::shareable_psbt(&swapped).is_err());
}
