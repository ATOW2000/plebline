//! Outputs marked as change: every output must be an expected recipient or
//! ours. Otherwise a tampered binary could send the change (usually the
//! largest output) elsewhere and still pass verification, also by writing
//! in the file an address other than the one in the signed bytes.

use plebline_core::batch::{
    shown_address, SignedBatch, SignedInput, SignedOutput, SignedTx, BATCH_SCHEMA_VERSION,
};
use plebline_core::bdk_wallet::{SignOptions, TxOrdering};
use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::{
    absolute::LockTime, transaction::Version, Address, Amount, FeeRate, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Witness,
};
use plebline_core::verify::{self, Expectations};
use plebline_core::wallet::public_descriptors;
use plebline_core::{bdk_wallet::KeychainKind, bdk_wallet::Wallet};

mod common;
use common::{NETWORK, SEED};

const THIEF_SEED: &str = common::OTHER_SEED;

fn wallet(seed: &str) -> Wallet {
    common::wallet(seed, None)
}

use common::outpoint;

/// True if only the (fake) signatures fail; real ones are tested in
/// `signatures` and `batch_regtest.rs`.
fn only_the_signatures_fail(report: &verify::Report) -> bool {
    report
        .failures()
        .all(|c| c.kind == verify::CheckKind::Signatures(0))
}

/// Fake witness: see [`only_the_signatures_fail`].
fn tx_with(inputs: &[OutPoint], outputs: &[(ScriptBuf, u64)]) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: inputs
            .iter()
            .map(|op| TxIn {
                previous_output: *op,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::from_slice(&[vec![0u8; 71], vec![0u8; 33]]),
            })
            .collect(),
        output: outputs
            .iter()
            .map(|(spk, value)| TxOut {
                value: Amount::from_sat(*value),
                script_pubkey: spk.clone(),
            })
            .collect(),
    }
}

/// The batch a tampered binary might produce: it pays the right recipient the
/// right amount, and sends the change to a stranger.
fn batch_with_stolen_change() -> (SignedBatch, String) {
    let mine = wallet(SEED);
    let thief = wallet(THIEF_SEED);

    let recipient = mine.peek_address(KeychainKind::External, 50).address;
    let thief_address = thief.peek_address(KeychainKind::External, 0).address;

    let tx = tx_with(
        &[outpoint(1)],
        &[
            (recipient.script_pubkey(), 100_000),
            // The change: 399,000 sat that do not come back.
            (thief_address.script_pubkey(), 399_000),
        ],
    );
    let vsize = tx.vsize() as u64;

    let signed = SignedTx {
        index: 0,
        label: None,
        txid: tx.compute_txid().to_string(),
        hex: serialize_hex(&tx),
        psbt: String::new(),
        inputs: vec![SignedInput {
            outpoint: outpoint(1).to_string(),
            value_sat: 500_000,
        }],
        outputs: vec![
            SignedOutput {
                address: recipient.to_string(),
                value_sat: 100_000,
                is_change: false,
            },
            SignedOutput {
                // Marked as change, which hides the theft unless verification
                // checks who owns the change.
                address: thief_address.to_string(),
                value_sat: 399_000,
                is_change: true,
            },
        ],
        fee_sat: 1_000,
        fee_rate_sat_vb: 1_000.0 / vsize as f64,
        vsize,
    };

    let batch = SignedBatch {
        version: BATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        created_by: "manomesso".into(),
        descriptor_checksum: mine.descriptor_checksum(KeychainKind::External),
        fee_rate_sat_vb: 1.0,
        txs: vec![signed],
    };

    (batch, recipient.to_string())
}

/// Without descriptors, change cannot be checked: a known limit.
#[test]
fn without_descriptors_stolen_change_cannot_be_detected() {
    let (batch, recipient) = batch_with_stolen_change();

    let report = verify::verify(
        &batch,
        &Expectations {
            network: Some(NETWORK),
            recipients: Some(vec![recipient]),
            spendable: Some(vec![outpoint(1)]),
            ..Default::default()
        },
    );

    assert!(
        only_the_signatures_fail(&report),
        "senza descriptor non c'è modo di sapere che il resto è finito altrove: {:?}",
        report.failures().map(|c| &c.detail).collect::<Vec<_>>()
    );
}

#[test]
fn with_descriptors_stolen_change_is_detected() {
    let (batch, recipient) = batch_with_stolen_change();
    let mine = wallet(SEED);
    let (external, internal) = public_descriptors(&mine);

    let report = verify::verify(
        &batch,
        &Expectations {
            network: Some(NETWORK),
            recipients: Some(vec![recipient]),
            spendable: Some(vec![outpoint(1)]),
            wallet_descriptors: Some((external, internal)),
            ownership_depth: None,
        },
    );

    assert!(
        !report.is_ok(),
        "il resto finito a un estraneo deve far fallire la verifica"
    );
    assert!(
        report
            .failures()
            .any(|c| c.detail.contains("399_000") || c.detail.contains("399000")),
        "il messaggio deve dire quanti sat stanno uscendo: {:?}",
        report.failures().map(|c| &c.detail).collect::<Vec<_>>()
    );
}

/// Before sending, the definition may be gone or describe a newer round:
/// payments are then not compared, but the change still must come home.
#[test]
fn without_the_definition_stolen_change_is_still_detected() {
    let (batch, _) = batch_with_stolen_change();
    let (external, internal) = public_descriptors(&wallet(SEED));

    let report = verify::verify(
        &batch,
        &Expectations {
            network: Some(NETWORK),
            wallet_descriptors: Some((external, internal)),
            ..Default::default()
        },
    );

    let escaping: Vec<_> = report
        .failures()
        .filter(|c| c.detail.contains("399_000") || c.detail.contains("399000"))
        .collect();
    assert_eq!(
        escaping.len(),
        1,
        "il resto rubato va segnalato, il pagamento no: {:?}",
        report.failures().map(|c| &c.detail).collect::<Vec<_>>()
    );
    assert!(
        !escaping[0].detail.contains("100_000") && !escaping[0].detail.contains("100000"),
        "il pagamento al destinatario non è un'uscita sospetta: {}",
        escaping[0].detail
    );
}

/// A file can also leave an output out of its list: every output the bytes
/// pay is checked, not only the declared ones.
#[test]
fn stolen_change_left_out_of_the_file_is_still_detected() {
    let (mut batch, recipient) = batch_with_stolen_change();
    batch.txs[0].outputs.truncate(1);
    let (external, internal) = public_descriptors(&wallet(SEED));

    let report = verify::verify(
        &batch,
        &Expectations {
            network: Some(NETWORK),
            recipients: Some(vec![recipient]),
            spendable: Some(vec![outpoint(1)]),
            wallet_descriptors: Some((external, internal)),
            ownership_depth: None,
        },
    );

    let failures: Vec<_> = report.failures().collect();
    assert!(
        failures
            .iter()
            .any(|c| c.kind == verify::CheckKind::Destination
                && (c.detail.contains("399_000") || c.detail.contains("399000"))),
        "l'uscita non dichiarata va all'estraneo: deve fallire la destinazione: {:?}",
        failures.iter().map(|c| &c.detail).collect::<Vec<_>>()
    );
    assert!(
        failures
            .iter()
            .any(|c| c.kind == verify::CheckKind::Addresses(0)),
        "e lo schermo non la mostrerebbe: {:?}",
        failures.iter().map(|c| &c.detail).collect::<Vec<_>>()
    );
}

#[test]
fn change_returning_to_the_wallet_raises_no_false_alarm() {
    let mine = wallet(SEED);
    let recipient = wallet(THIEF_SEED)
        .peek_address(KeychainKind::External, 0)
        .address;
    let change = mine.peek_address(KeychainKind::Internal, 0).address;

    let tx = tx_with(
        &[outpoint(1)],
        &[
            (recipient.script_pubkey(), 100_000),
            (change.script_pubkey(), 399_000),
        ],
    );
    let vsize = tx.vsize() as u64;

    let signed = SignedTx {
        index: 0,
        label: None,
        txid: tx.compute_txid().to_string(),
        hex: serialize_hex(&tx),
        psbt: String::new(),
        inputs: vec![SignedInput {
            outpoint: outpoint(1).to_string(),
            value_sat: 500_000,
        }],
        outputs: vec![
            SignedOutput {
                address: recipient.to_string(),
                value_sat: 100_000,
                is_change: false,
            },
            SignedOutput {
                address: change.to_string(),
                value_sat: 399_000,
                is_change: true,
            },
        ],
        fee_sat: 1_000,
        fee_rate_sat_vb: 1_000.0 / vsize as f64,
        vsize,
    };

    let batch = SignedBatch {
        version: BATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        created_by: "onesto".into(),
        descriptor_checksum: mine.descriptor_checksum(KeychainKind::External),
        fee_rate_sat_vb: 1.0,
        txs: vec![signed],
    };

    let (external, internal) = public_descriptors(&mine);
    let report = verify::verify(
        &batch,
        &Expectations {
            network: Some(NETWORK),
            recipients: Some(vec![recipient.to_string()]),
            spendable: Some(vec![outpoint(1)]),
            wallet_descriptors: Some((external, internal)),
            ownership_depth: None,
        },
    );

    assert!(
        only_the_signatures_fail(&report),
        "un resto che torna al wallet non è un problema: {:?}",
        report.failures().map(|c| &c.detail).collect::<Vec<_>>()
    );
}

/// Really signed: 100,000 sat to `recipient` from a 500,000 sat coin, the
/// rest to `rest_to`. `label` writes each output's text and change flag, as
/// an honest or a tampered binary would.
fn really_signed(
    recipient: &Address,
    rest_to: &Address,
    label: impl Fn(&TxOut) -> (String, bool),
) -> (SignedBatch, Wallet, Vec<OutPoint>) {
    let (mut mine, set) = common::hydrated(wallet(SEED), &[(0, 500_000, 1)]);
    let mut psbt = {
        let mut builder = mine.build_tx();
        builder
            .ordering(TxOrdering::Untouched)
            .nlocktime(LockTime::ZERO)
            .fee_rate(FeeRate::from_sat_per_vb(5).expect("tariffa valida"))
            .add_recipient(recipient.script_pubkey(), Amount::from_sat(100_000))
            .drain_to(rest_to.script_pubkey());
        builder.finish().expect("transazione costruita")
    };
    assert!(mine
        .sign(&mut psbt, SignOptions::default())
        .expect("firma riuscita"));
    let tx = psbt.extract_tx().expect("transazione estratta");

    let paid: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
    let fee = 500_000 - paid;
    let vsize = tx.vsize() as u64;
    let outputs = tx
        .output
        .iter()
        .map(|o| {
            let (address, is_change) = label(o);
            SignedOutput {
                address,
                value_sat: o.value.to_sat(),
                is_change,
            }
        })
        .collect();

    let batch = SignedBatch {
        version: BATCH_SCHEMA_VERSION,
        network: NETWORK.to_string(),
        created_by: "prova".into(),
        descriptor_checksum: mine.descriptor_checksum(KeychainKind::External),
        fee_rate_sat_vb: 5.0,
        txs: vec![SignedTx {
            index: 0,
            label: None,
            txid: tx.compute_txid().to_string(),
            hex: serialize_hex(&tx),
            psbt: String::new(),
            inputs: vec![SignedInput {
                outpoint: tx.input[0].previous_output.to_string(),
                value_sat: 500_000,
            }],
            outputs,
            fee_sat: fee,
            fee_rate_sat_vb: fee as f64 / vsize as f64,
            vsize,
        }],
    };
    let spendable = set.utxos.iter().map(|u| u.outpoint).collect();
    (batch, mine, spendable)
}

fn expecting(recipient: &Address, spendable: &[OutPoint], mine: Option<&Wallet>) -> Expectations {
    Expectations {
        network: Some(NETWORK),
        recipients: Some(vec![recipient.to_string()]),
        spendable: Some(spendable.to_vec()),
        wallet_descriptors: mine.map(public_descriptors),
        ownership_depth: None,
    }
}

#[test]
fn honest_text_on_a_really_signed_payment_passes() {
    let recipient = wallet(THIEF_SEED)
        .peek_address(KeychainKind::External, 0)
        .address;
    let change = wallet(SEED).peek_address(KeychainKind::Internal, 0).address;
    let (batch, mine, spendable) = really_signed(&recipient, &change, |o| {
        (
            shown_address(&o.script_pubkey, NETWORK),
            o.script_pubkey == change.script_pubkey(),
        )
    });

    let report = verify::verify(&batch, &expecting(&recipient, &spendable, Some(&mine)));
    assert!(
        report.is_ok(),
        "un file onesto non deve fallire: {:?}",
        report.failures().map(|c| &c.detail).collect::<Vec<_>>()
    );
}

/// The file names the recipient for every output and marks the stolen one
/// as change: on screen, only the payment to the recipient would show.
#[test]
fn text_naming_the_recipient_for_stolen_change_is_caught() {
    let recipient = wallet(THIEF_SEED)
        .peek_address(KeychainKind::External, 0)
        .address;
    let thief = wallet(THIEF_SEED)
        .peek_address(KeychainKind::External, 7)
        .address;
    let (batch, mine, spendable) = really_signed(&recipient, &thief, |o| {
        (
            recipient.to_string(),
            o.script_pubkey != recipient.script_pubkey(),
        )
    });

    // Bytes against text needs no descriptors: a third party with only the
    // file sees it too.
    for descriptors in [None, Some(&mine)] {
        let report = verify::verify(&batch, &expecting(&recipient, &spendable, descriptors));
        assert!(
            report
                .failures()
                .any(|c| c.detail.contains(&thief.to_string())),
            "il ladro vero deve comparire fra gli errori: {:?}",
            report.failures().map(|c| &c.detail).collect::<Vec<_>>()
        );
    }
}

/// Honest text, but the change flag hides a second, larger payment to the
/// recipient.
#[test]
fn change_paying_an_expected_address_is_caught() {
    let recipient = wallet(THIEF_SEED)
        .peek_address(KeychainKind::External, 0)
        .address;
    let (batch, mine, spendable) = really_signed(&recipient, &recipient, |o| {
        (
            shown_address(&o.script_pubkey, NETWORK),
            o.value.to_sat() != 100_000,
        )
    });

    let report = verify::verify(&batch, &expecting(&recipient, &spendable, Some(&mine)));
    assert!(
        report
            .failures()
            .any(|c| c.detail.contains(&recipient.to_string())),
        "un resto che non torna al wallet deve fallire anche se paga un indirizzo atteso: {:?}",
        report.failures().map(|c| &c.detail).collect::<Vec<_>>()
    );
}
