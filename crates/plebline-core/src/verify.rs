//! Independent verification of `batch.json`: no keys, no network, no
//! signing wallet. Txids, signatures, amounts, fees, addresses and
//! disjointness are recomputed from the transaction bytes. Every failure is
//! reported, not just the first.
//!
//! Signatures are verified cryptographically ([`crate::signatures`]); since
//! the SegWit sighash commits to input amounts, that also proves the fees.

use std::collections::BTreeMap;
use std::str::FromStr;

use bdk_wallet::bitcoin::consensus::encode::deserialize_hex;
use bdk_wallet::bitcoin::{absolute::LockTime, Amount, Network, OutPoint, Transaction, Txid};

use crate::batch::{SignedBatch, SignedTx, BATCH_SCHEMA_VERSION};
use crate::error::{Error, Result};
use crate::lang::t;
use crate::plan::check_disjoint;
use crate::{tf, within_all_bitcoin};

/// Above this fee rate, assume a typo.
pub const MAX_SANE_FEE_RATE: f64 = 500.0;
/// Largest sane share of the amount spent on fees.
pub const MAX_FEE_RATIO: f64 = 0.10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Pass,
    Warn,
    Fail,
}

/// What a check looked at. Callers and tests match on it; people read
/// [`CheckKind::name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Schema,
    Network,
    Content,
    Independence,
    Recipients,
    Origin,
    Destination,
    Totals,
    /// Of the transaction with this index (from 0).
    Reading(usize),
    Txid(usize),
    Inputs(usize),
    LockTime(usize),
    Signatures(usize),
    Amounts(usize),
    Fee(usize),
    Weight(usize),
    FeeRate(usize),
    Outputs(usize),
    Addresses(usize),
}

impl CheckKind {
    /// The name shown to the user, in plain words.
    pub fn name(self) -> String {
        let of_tx = |index: usize, it: &str, en: &str| {
            let n = crate::batch::number(index);
            tf!({ "transazione {n}, {it}" }, { "transaction {n}, {en}" })
        };
        match self {
            CheckKind::Schema => t("formato del file", "file format").to_string(),
            CheckKind::Network => t("rete", "network").to_string(),
            CheckKind::Content => t("contenuto", "content").to_string(),
            CheckKind::Independence => t("indipendenza", "independence").to_string(),
            CheckKind::Recipients => t("destinatari", "recipients").to_string(),
            CheckKind::Origin => {
                t("da dove escono le monete", "where the coins come from").to_string()
            }
            CheckKind::Destination => {
                t("dove finiscono le monete", "where the coins end up").to_string()
            }
            CheckKind::Totals => t("totali", "totals").to_string(),
            CheckKind::Reading(i) => of_tx(i, "lettura", "reading"),
            CheckKind::Txid(i) => of_tx(i, "identificativo", "identifier"),
            CheckKind::Inputs(i) => of_tx(i, "monete spese", "coins spent"),
            CheckKind::LockTime(i) => of_tx(i, "blocco temporale", "time lock"),
            CheckKind::Signatures(i) => of_tx(i, "firme", "signatures"),
            CheckKind::Amounts(i) => of_tx(i, "importi", "amounts"),
            CheckKind::Fee(i) => of_tx(i, "fee", "fee"),
            CheckKind::Weight(i) => of_tx(i, "peso", "weight"),
            CheckKind::FeeRate(i) => of_tx(i, "prezzo al peso", "price per weight"),
            CheckKind::Outputs(i) => of_tx(i, "uscite", "outputs"),
            CheckKind::Addresses(i) => of_tx(i, "indirizzi", "addresses"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub kind: CheckKind,
    /// [`CheckKind::name`], in the language of the moment it was made.
    pub name: String,
    pub severity: Severity,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    fn record(&mut self, severity: Severity, kind: CheckKind, detail: impl Into<String>) {
        self.checks.push(Check {
            kind,
            name: kind.name(),
            severity,
            detail: detail.into(),
        });
    }

    fn pass(&mut self, kind: CheckKind, detail: impl Into<String>) {
        self.record(Severity::Pass, kind, detail);
    }

    fn warn(&mut self, kind: CheckKind, detail: impl Into<String>) {
        self.record(Severity::Warn, kind, detail);
    }

    fn fail(&mut self, kind: CheckKind, detail: impl Into<String>) {
        self.record(Severity::Fail, kind, detail);
    }

    pub fn is_ok(&self) -> bool {
        !self.checks.iter().any(|c| c.severity == Severity::Fail)
    }

    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.severity == Severity::Fail)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.severity == Severity::Warn)
    }
}

/// What the caller knows about the batch. Every field is optional because a
/// third party may have only the file.
#[derive(Debug, Clone, Default)]
pub struct Expectations {
    pub network: Option<Network>,
    /// Expected destination addresses, one per transaction, in order.
    pub recipients: Option<Vec<String>>,
    /// Outpoints the batch may legitimately spend (from `utxos.json`).
    pub spendable: Option<Vec<OutPoint>>,
    /// Public descriptors from `wallet.json`, needed to check that change goes
    /// back to this wallet.
    pub wallet_descriptors: Option<(String, String)>,
    /// Derivation depth for recognizing the wallet's addresses, never below
    /// 300: too shallow, and real change reads as leaving the wallet. A
    /// wallet with a longer history passes its last active index plus a
    /// margin.
    pub ownership_depth: Option<u32>,
}

/// Twenty is the usual gap limit; 300 covers any realistic history.
const DEFAULT_OWNERSHIP_DEPTH: u32 = 300;

pub fn verify(batch: &SignedBatch, expect: &Expectations) -> Report {
    let mut report = Report::default();

    if !check_format(batch, &mut report) {
        return report;
    }
    check_network(batch, expect, &mut report);

    if batch.txs.is_empty() {
        report.fail(
            CheckKind::Content,
            t(
                "il file non contiene nessuna transazione",
                "the file contains no transactions",
            ),
        );
        return report;
    }

    let Some(decoded) = decode(batch, &mut report) else {
        return report;
    };

    let network = batch_network(batch);
    check_independence(batch, &decoded, &mut report);
    for (declared, parsed) in batch.txs.iter().zip(decoded.iter()) {
        check_transaction(declared, parsed, network, &mut report);
    }
    check_recipients(batch, &decoded, expect, &mut report);
    check_origin(&decoded, expect, &mut report);
    check_destination(batch, &decoded, expect, &mut report);
    report_totals(batch, &mut report);

    report
}

// Check names are shown to the user, so they say what was checked in plain
// words.
fn check_format(batch: &SignedBatch, report: &mut Report) -> bool {
    if batch.version != BATCH_SCHEMA_VERSION {
        let found = batch.version;
        report.fail(
            CheckKind::Schema,
            tf!(
                { "versione {found} non supportata (attesa {BATCH_SCHEMA_VERSION})" },
                { "version {found} is not supported (expected {BATCH_SCHEMA_VERSION})" }
            ),
        );
        return false;
    }
    let found = batch.version;
    report.pass(
        CheckKind::Schema,
        tf!({ "versione {found}" }, { "version {found}" }),
    );
    true
}

fn check_network(batch: &SignedBatch, expect: &Expectations, report: &mut Report) {
    match Network::from_str(&batch.network) {
        Ok(network) => {
            if let Some(expected) = expect.network {
                if network != expected {
                    report.fail(
                        CheckKind::Network,
                        tf!(
                            { "queste transazioni sono per {network}, non per {expected}" },
                            { "these transactions are for {network}, not for {expected}" }
                        ),
                    );
                } else {
                    report.pass(CheckKind::Network, network.to_string());
                }
            } else {
                report.pass(CheckKind::Network, network.to_string());
            }
        }
        Err(_) => {
            let declared = &batch.network;
            report.fail(
                CheckKind::Network,
                tf!({ "rete sconosciuta '{declared}'" }, {
                    "unknown network '{declared}'"
                }),
            )
        }
    }
}

/// The transactions decoded from their bytes, in file order; `None` if any
/// cannot be read (each failure is already reported).
fn decode(batch: &SignedBatch, report: &mut Report) -> Option<Vec<Transaction>> {
    let mut decoded: Vec<Transaction> = Vec::with_capacity(batch.txs.len());
    for tx in &batch.txs {
        let parsed: Transaction = match deserialize_hex(&tx.hex) {
            Ok(parsed) => parsed,
            Err(e) => {
                report.fail(
                    CheckKind::Reading(tx.index),
                    tf!({ "i byte della transazione non si leggono: {e}" }, {
                        "the transaction bytes cannot be read: {e}"
                    }),
                );
                continue;
            }
        };

        let computed = parsed.compute_txid();
        let label = CheckKind::Txid(tx.index);
        match Txid::from_str(&tx.txid) {
            Ok(declared) if declared == computed => {
                report.pass(label, computed.to_string());
            }
            Ok(declared) => report.fail(
                label,
                tf!({ "dichiarato {declared}, calcolato {computed}" }, {
                    "declared {declared}, computed {computed}"
                }),
            ),
            Err(_) => {
                let declared = &tx.txid;
                report.fail(
                    label,
                    tf!({ "txid dichiarato malformato: {declared}" }, {
                        "declared txid is malformed: {declared}"
                    }),
                )
            }
        }

        decoded.push(parsed);
    }
    (decoded.len() == batch.txs.len()).then_some(decoded)
}

/// No two transactions share an input and none spends another's output, so
/// they can be broadcast in any order.
fn check_independence(batch: &SignedBatch, decoded: &[Transaction], report: &mut Report) {
    let spending: Vec<_> = batch
        .txs
        .iter()
        .zip(decoded)
        .map(|(tx, parsed)| {
            let inputs = parsed.input.iter().map(|i| i.previous_output).collect();
            (tx.index, parsed.compute_txid(), inputs)
        })
        .collect();

    let independence = CheckKind::Independence;
    match check_disjoint(&spending) {
        Ok(()) => {
            let count = batch.txs.len();
            report.pass(
                independence,
                tf!(
                    {
                        "nessuna delle {count} transazioni spende una moneta usata da un'altra, e \
                      nessuna dipende da un'altra: si mandano in rete in qualsiasi ordine"
                    },
                    {
                        "none of the {count} transactions spends a coin used by another, and none \
                      depends on another: send them in any order"
                    }
                ),
            )
        }
        Err(e) => report.fail(independence, e.to_string()),
    }
}

/// One transaction against what the file declares about it.
fn check_transaction(
    declared: &SignedTx,
    parsed: &Transaction,
    network: Network,
    report: &mut Report,
) {
    if !check_inputs(declared, parsed, report) {
        return;
    }
    check_lock_time(declared.index, parsed, report);
    check_signatures(declared, parsed, report);
    check_amounts(declared, parsed, report);
    check_addresses(declared, parsed, network, report);
}

/// The screens show the file's address text, so it must name what the
/// signed bytes pay.
fn check_addresses(
    declared: &SignedTx,
    parsed: &Transaction,
    network: Network,
    report: &mut Report,
) {
    let mut wrong = Vec::new();
    let (listed, paid) = (declared.outputs.len(), parsed.output.len());
    if listed != paid {
        wrong.push(tf!(
            { "il file elenca {listed} uscite, la transazione ne paga {paid}" },
            { "the file lists {listed} outputs, the transaction pays {paid}" }
        ));
    }
    for (position, (output, txout)) in declared.outputs.iter().zip(&parsed.output).enumerate() {
        let actual = crate::batch::shown_address(&txout.script_pubkey, network);
        if output.address != actual {
            let number = position + 1;
            let shown = &output.address;
            wrong.push(tf!(
                { "uscita {number}: il file dice {shown}, la transazione paga {actual}" },
                { "output {number}: the file says {shown}, the transaction pays {actual}" }
            ));
        }
    }
    if !wrong.is_empty() {
        report.fail(CheckKind::Addresses(declared.index), wrong.join("; "));
    }
}

fn check_inputs(declared: &SignedTx, parsed: &Transaction, report: &mut Report) -> bool {
    let actual_inputs: Vec<String> = parsed
        .input
        .iter()
        .map(|i| i.previous_output.to_string())
        .collect();
    let declared_inputs: Vec<String> = declared.inputs.iter().map(|i| i.outpoint.clone()).collect();
    if actual_inputs != declared_inputs {
        let declared_list = declared_inputs.join(", ");
        let actual_list = actual_inputs.join(", ");
        report.fail(
            CheckKind::Inputs(declared.index),
            tf!(
                { "il file dichiara [{declared_list}] ma la transazione spende [{actual_list}]" },
                {
                    "the file declares [{declared_list}] but the transaction spends \
                  [{actual_list}]"
                }
            ),
        );
        return false;
    }
    true
}

fn check_lock_time(index: usize, parsed: &Transaction, report: &mut Report) {
    // Only a warning, since the coins are not at risk, but it delays broadcast
    // or marks the payment as presigned. This program always writes zero.
    if parsed.lock_time != LockTime::ZERO {
        let lock_time = parsed.lock_time;
        report.warn(
            CheckKind::LockTime(index),
            tf!(
                {
                    "nLockTime è {lock_time}, non zero: non viene da questo programma, e \
                  se è un'altezza vecchia rivela che la transazione è stata prefirmata"
                },
                {
                    "nLockTime is {lock_time}, not zero: it does not come from this \
                  program, and an old height reveals the transaction was presigned"
                }
            ),
        );
    }
}

fn check_signatures(declared: &SignedTx, parsed: &Transaction, report: &mut Report) {
    let values: Vec<Amount> = declared
        .inputs
        .iter()
        .map(|i| Amount::from_sat(i.value_sat))
        .collect();
    let signature_label = CheckKind::Signatures(declared.index);
    let checks = crate::signatures::check(parsed, &values);
    let invalid: Vec<String> = checks
        .iter()
        .filter_map(|c| match &c.outcome {
            crate::signatures::Outcome::Invalid { reason } => {
                let n = c.index;
                Some(format!("input {n}: {reason}"))
            }
            _ => None,
        })
        .collect();
    let unsupported: Vec<String> = checks
        .iter()
        .filter_map(|c| match &c.outcome {
            crate::signatures::Outcome::Unsupported { reason } => {
                let n = c.index;
                Some(format!("input {n}: {reason}"))
            }
            _ => None,
        })
        .collect();

    if !invalid.is_empty() {
        let list = invalid.join("; ");
        report.fail(
            signature_label,
            tf!(
                {
                    "{list}. Una firma che non torna significa una transazione che la rete \
                     rifiuterà: va rifatta la cerimonia, non trasmessa."
                },
                {
                    "{list}. A signature that does not check out means a transaction the \
                     network will reject: redo the ceremony, do not broadcast."
                }
            ),
        );
    } else if !unsupported.is_empty() {
        // PlebLine signs only P2WPKH: anything else did not come from it.
        let list = unsupported.join("; ");
        report.fail(
            signature_label,
            tf!(
                {
                    "{list}. PlebLine firma solo input P2WPKH: una firma che non so \
                     verificare non l'ha fatta lui, e non va trasmessa."
                },
                {
                    "{list}. PlebLine signs only P2WPKH inputs: a signature I cannot verify \
                     was not made by it, and must not be broadcast."
                }
            ),
        );
    } else {
        let count = checks.len();
        report.pass(
            signature_label,
            tf!(
                {
                    "{count} firme valide. Coprono destinatari, importi e gli importi degli \
                     input: nessuno di questi può essere cambiato senza invalidarle"
                },
                {
                    "{count} valid signatures. They cover recipients, amounts and the input \
                     values: none of these can be changed without breaking them"
                }
            ),
        );
    }
}

fn check_amounts(declared: &SignedTx, parsed: &Transaction, report: &mut Report) {
    // The file can declare any value, so a sum that overflows is a failure.
    let input_total = within_all_bitcoin(declared.inputs.iter().map(|i| i.value_sat));
    let output_total = within_all_bitcoin(parsed.output.iter().map(|o| o.value.to_sat()));
    let (Some(input_total), Some(output_total)) = (input_total, output_total) else {
        report.fail(
            CheckKind::Amounts(declared.index),
            t(
                "gli importi sommano più di tutti i bitcoin che esisteranno mai: nessuna \
                 transazione vera è così",
                "the amounts add up to more than all the bitcoin there will ever be: no real \
                 transaction looks like this",
            ),
        );
        return;
    };
    let Some(fee) = input_total.checked_sub(output_total) else {
        report.fail(
            CheckKind::Amounts(declared.index),
            tf!(
                { "gli output ({output_total} sat) superano gli input ({input_total} sat)" },
                { "the outputs ({output_total} sat) exceed the inputs ({input_total} sat)" }
            ),
        );
        return;
    };

    if fee != declared.fee_sat {
        let declared_fee = declared.fee_sat;
        report.fail(
            CheckKind::Fee(declared.index),
            tf!(
                { "il file dichiara {declared_fee} sat, il calcolo dà {fee} sat" },
                { "the file declares {declared_fee} sat, the computation gives {fee} sat" }
            ),
        );
    }

    let vsize = parsed.vsize() as u64;
    if vsize != declared.vsize {
        let declared_vsize = declared.vsize;
        report.fail(
            CheckKind::Weight(declared.index),
            tf!({ "dichiarato {declared_vsize}, calcolato {vsize}" }, {
                "declared {declared_vsize}, computed {vsize}"
            }),
        );
    }

    let fee_rate = fee as f64 / vsize as f64;
    let rate_label = CheckKind::FeeRate(declared.index);
    if fee_rate < 1.0 {
        report.fail(
            rate_label,
            tf!(
                {
                    "{fee_rate:.2} sat/vB: sotto il minimo di propagazione, non entrerebbe in \
                  mempool"
                },
                {
                    "{fee_rate:.2} sat/vB: below the relay minimum, it would not enter the \
                  mempool"
                }
            ),
        );
    } else if fee_rate > MAX_SANE_FEE_RATE {
        report.fail(
            rate_label,
            tf!(
                { "{fee_rate:.2} sat/vB: implausibilmente alto, probabile errore" },
                { "{fee_rate:.2} sat/vB: implausibly high, likely a mistake" }
            ),
        );
    } else {
        report.pass(
            rate_label,
            tf!(
                { "{fee_rate:.2} sat/vB ({fee} sat di fee su un peso di {vsize})" },
                { "{fee_rate:.2} sat/vB ({fee} sat of fee on a weight of {vsize})" }
            ),
        );
    }

    let sent: u64 = declared
        .outputs
        .iter()
        .filter(|o| !o.is_change)
        .fold(0, |sum, o| sum.saturating_add(o.value_sat));
    if sent > 0 && fee as f64 > sent as f64 * MAX_FEE_RATIO {
        let percent = (MAX_FEE_RATIO * 100.0) as u32;
        report.warn(
            CheckKind::Fee(declared.index),
            tf!(
                { "{fee} sat di fee su {sent} sat inviati: oltre il {percent}% dell'importo" },
                { "{fee} sat of fee on {sent} sat sent: over {percent}% of the amount" }
            ),
        );
    }

    let declared_values: Vec<u64> = declared.outputs.iter().map(|o| o.value_sat).collect();
    let actual_values: Vec<u64> = parsed.output.iter().map(|o| o.value.to_sat()).collect();
    if declared_values != actual_values {
        report.fail(
            CheckKind::Outputs(declared.index),
            tf!(
                { "importi dichiarati {declared_values:?}, effettivi {actual_values:?}" },
                { "declared amounts {declared_values:?}, actual {actual_values:?}" }
            ),
        );
    }
}

fn check_recipients(
    batch: &SignedBatch,
    decoded: &[Transaction],
    expect: &Expectations,
    report: &mut Report,
) {
    if let Some(recipients) = &expect.recipients {
        let network = batch_network(batch);
        let recipients_label = CheckKind::Recipients;
        if recipients.len() != batch.txs.len() {
            let wanted = recipients.len();
            let got = batch.txs.len();
            report.fail(
                recipients_label,
                tf!(
                    { "attese {wanted} transazioni, il batch ne contiene {got}" },
                    { "expected {wanted} transactions, the batch contains {got}" }
                ),
            );
        } else {
            let mut mismatched = Vec::new();
            for ((expected, tx), parsed) in recipients.iter().zip(&batch.txs).zip(decoded) {
                let found = tx.outputs.iter().zip(&parsed.output).any(|(o, txout)| {
                    !o.is_change
                        && crate::batch::shown_address(&txout.script_pubkey, network) == *expected
                });
                if !found {
                    let index = crate::batch::number(tx.index);
                    mismatched.push(tf!({ "#{index}: atteso {expected}" }, {
                        "#{index}: expected {expected}"
                    }));
                }
            }
            if mismatched.is_empty() {
                report.pass(
                    recipients_label,
                    t(
                        "tutti corrispondono a quelli che hai indicato",
                        "all match the expected addresses",
                    ),
                );
            } else {
                report.fail(recipients_label, mismatched.join("; "));
            }
        }
    }
}

fn check_origin(decoded: &[Transaction], expect: &Expectations, report: &mut Report) {
    if let Some(spendable) = &expect.spendable {
        let allowed: std::collections::BTreeSet<&OutPoint> = spendable.iter().collect();
        let mut unknown = Vec::new();
        for parsed in decoded {
            for op in parsed.input.iter().map(|i| &i.previous_output) {
                if !allowed.contains(op) {
                    unknown.push(op.to_string());
                }
            }
        }
        let origin = CheckKind::Origin;
        if unknown.is_empty() {
            report.pass(
                origin,
                t(
                    "ogni moneta spesa era fra quelle del tuo wallet",
                    "every coin spent was among the ones in your wallet",
                ),
            );
        } else {
            let list = unknown.join(", ");
            report.fail(
                origin,
                tf!({ "input estranei al set dichiarato: {list}" }, {
                    "inputs outside the declared set: {list}"
                }),
            );
        }
    }
}

// Change is usually the largest output, so with the descriptors each output
// must go to an expected recipient or back to the wallet. Addresses come from
// the bytes: the file's text is only what the screens show.
fn check_destination(
    batch: &SignedBatch,
    decoded: &[Transaction],
    expect: &Expectations,
    report: &mut Report,
) {
    if let Some((external, internal)) = &expect.wallet_descriptors {
        let destination = CheckKind::Destination;
        match crate::wallet::from_descriptors(external, internal, batch_network(batch)) {
            Ok(mut wallet) => {
                let depth = expect
                    .ownership_depth
                    .unwrap_or(DEFAULT_OWNERSHIP_DEPTH)
                    .max(DEFAULT_OWNERSHIP_DEPTH);
                let _ = wallet
                    .reveal_addresses_to(bdk_wallet::KeychainKind::External, depth)
                    .count();
                let _ = wallet
                    .reveal_addresses_to(bdk_wallet::KeychainKind::Internal, depth)
                    .count();

                // Without the definition, payments cannot be compared: only
                // the change is checked.
                let expected: Option<std::collections::BTreeSet<&String>> =
                    expect.recipients.as_ref().map(|r| r.iter().collect());

                let network = batch_network(batch);
                let mut escaping = Vec::new();
                for (declared, parsed) in batch.txs.iter().zip(decoded.iter()) {
                    // Every output the bytes pay, listed in the file or not.
                    for (position, txout) in parsed.output.iter().enumerate() {
                        let is_change = declared
                            .outputs
                            .get(position)
                            .is_some_and(|output| output.is_change);
                        let mine = wallet.is_mine(txout.script_pubkey.clone());
                        let address = crate::batch::shown_address(&txout.script_pubkey, network);
                        // Summaries leave change out, so an output marked as
                        // change must come back even if it pays an expected
                        // address.
                        let allowed = mine
                            || (!is_change
                                && expected.as_ref().is_none_or(|e| e.contains(&address)));
                        if allowed {
                            continue;
                        }
                        let index = crate::batch::number(declared.index);
                        let value = txout.value.to_sat();
                        escaping.push(tf!({ "#{index}: {value} sat verso {address}" }, {
                            "#{index}: {value} sat to {address}"
                        }));
                    }
                }

                if escaping.is_empty() {
                    report.pass(
                        destination,
                        if expected.is_some() {
                            t(
                                "ogni moneta va a chi deve, o torna a te",
                                "every coin goes where it should, or comes back to you",
                            )
                        } else {
                            t(
                                "il resto torna a te; i pagamenti non li confronto, perché qui \
                                 non c'è la definizione delle ricariche",
                                "the change comes back to you; the payments are not compared, \
                                 because the definition of the refills is not here",
                            )
                        },
                    );
                } else {
                    let list = escaping.join("; ");
                    report.fail(
                        destination,
                        tf!(
                            {
                                "monete che escono verso indirizzi né attesi né tuoi: {list}. \
                                 Un output marcato come resto che non torna al wallet è un \
                                 pagamento a qualcun altro."
                            },
                            {
                                "coins leaving towards addresses that are neither expected nor \
                                 yours: {list}. An output marked as change that does not return \
                                 to the wallet is a payment to someone else."
                            }
                        ),
                    );
                }
            }
            Err(e) => report.warn(
                destination,
                tf!({ "descriptor non utilizzabili, controllo saltato: {e}" }, {
                    "descriptors unusable, check skipped: {e}"
                }),
            ),
        }
    } else {
        report.warn(
            CheckKind::Destination,
            t(
                "non so quali indirizzi siano tuoi, quindi non posso dire se il resto torni \
                 davvero al tuo wallet. Serve il codice di sola lettura del wallet.",
                "I do not know which addresses are yours, so I cannot tell whether the change \
                 really comes back to your wallet. The wallet's read-only code is needed.",
            ),
        );
    }
}

fn report_totals(batch: &SignedBatch, report: &mut Report) {
    let mut per_recipient: BTreeMap<String, u64> = BTreeMap::new();
    for tx in &batch.txs {
        for output in tx.outputs.iter().filter(|o| !o.is_change) {
            let total = per_recipient.entry(output.address.clone()).or_default();
            *total = total.saturating_add(output.value_sat);
        }
    }
    let count = batch.txs.len();
    let sent = batch.total_amount();
    let fees = batch.total_fee();
    let distinct = per_recipient.len();
    let totals = if distinct == 0 {
        tf!(
            { "{count} transazioni, {fees} di fee: ogni moneta torna a te" },
            { "{count} transactions, {fees} in fees: every coin comes back to you" }
        )
    } else {
        tf!(
            {
                "{count} transazioni, {sent} in uscita, {fees} di fee, verso {distinct} destinatari diversi"
            },
            {
                "{count} transactions, {sent} going out, {fees} in fees, to {distinct} different recipients"
            }
        )
    };
    report.pass(CheckKind::Totals, totals);
}

/// Only used to build the comparison wallet; a mismatch is already reported.
fn batch_network(batch: &SignedBatch) -> Network {
    Network::from_str(&batch.network).unwrap_or(Network::Bitcoin)
}

pub fn load(path: &std::path::Path) -> Result<SignedBatch> {
    let batch: SignedBatch = crate::read_json(path)?;
    if let Some(reason) = implausible(&batch) {
        return Err(Error::Verification(format!("{}: {reason}", path.display())));
    }
    Ok(batch)
}

/// Contents a real signed file cannot have, refused before any sum or any
/// lookup by number: amounts beyond the total supply (the sums would
/// overflow) and two entries with the same index (sending "#N" could send
/// the other one).
fn implausible(batch: &SignedBatch) -> Option<String> {
    let mut indexes = std::collections::BTreeSet::new();
    for tx in &batch.txs {
        if !indexes.insert(tx.index) {
            let number = crate::batch::number(tx.index);
            return Some(tf!(
                { "due ricariche hanno lo stesso numero #{number}: non so quale sia quale" },
                { "two refills share number #{number}: I cannot tell which is which" }
            ));
        }
    }
    let txs = || batch.txs.iter();
    let sums = [
        within_all_bitcoin(txs().flat_map(|tx| tx.inputs.iter().map(|i| i.value_sat))),
        within_all_bitcoin(txs().flat_map(|tx| tx.outputs.iter().map(|o| o.value_sat))),
        within_all_bitcoin(txs().map(|tx| tx.fee_sat)),
    ];
    if sums.iter().any(Option::is_none) {
        return Some(
            t(
                "gli importi sommano più di tutti i bitcoin che esisteranno mai: non è un \
                 gruppo firmato vero",
                "the amounts add up to more than all the bitcoin there will ever be: this is \
                 not a real signed group",
            )
            .into(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{SignedInput, SignedOutput, SignedTx};
    use bdk_wallet::bitcoin::consensus::encode::serialize_hex;
    use bdk_wallet::bitcoin::{
        transaction::Version, Amount, ScriptBuf, Sequence, TxIn, TxOut, Witness,
    };

    use crate::testing::outpoint;

    /// Fake witness of the right shape: these tests cover the report logic, not
    /// the crypto (see `signatures` and `tests/batch_regtest.rs`).
    fn spending_tx(inputs: &[OutPoint], value: u64) -> Transaction {
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
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::from_hex("00140000000000000000000000000000000000000000")
                    .unwrap(),
            }],
        }
    }

    fn signed_tx(index: usize, inputs: &[(OutPoint, u64)], out_value: u64) -> SignedTx {
        let tx = spending_tx(
            &inputs.iter().map(|(op, _)| *op).collect::<Vec<_>>(),
            out_value,
        );
        let vsize = tx.vsize() as u64;
        let fee: u64 = inputs.iter().map(|(_, v)| *v).sum::<u64>() - out_value;
        SignedTx {
            index,
            label: None,
            txid: tx.compute_txid().to_string(),
            hex: serialize_hex(&tx),
            psbt: String::new(),
            inputs: inputs
                .iter()
                .map(|(op, v)| SignedInput {
                    outpoint: op.to_string(),
                    value_sat: *v,
                })
                .collect(),
            outputs: vec![SignedOutput {
                address: crate::batch::shown_address(&tx.output[0].script_pubkey, Network::Regtest),
                value_sat: out_value,
                is_change: false,
            }],
            fee_sat: fee,
            fee_rate_sat_vb: fee as f64 / vsize as f64,
            vsize,
        }
    }

    fn batch(txs: Vec<SignedTx>) -> SignedBatch {
        SignedBatch {
            version: BATCH_SCHEMA_VERSION,
            network: "regtest".into(),
            created_by: "test".into(),
            descriptor_checksum: "abcdefgh".into(),
            fee_rate_sat_vb: 5.0,
            txs,
        }
    }

    #[test]
    fn a_disjoint_batch_passes() {
        let b = batch(vec![
            signed_tx(0, &[(outpoint(1), 100_000)], 99_000),
            signed_tx(1, &[(outpoint(2), 100_000)], 99_000),
        ]);
        let report = verify(&b, &Expectations::default());

        // Fake signatures must be the only failures.
        let kinds: Vec<CheckKind> = report.failures().map(|c| c.kind).collect();
        assert!(
            kinds
                .iter()
                .all(|k| matches!(k, CheckKind::Signatures(0) | CheckKind::Signatures(1))),
            "oltre alle firme finte è fallito altro: {kinds:?}"
        );
    }

    /// Guards the fixture: a zero witness must never pass as a signature.
    #[test]
    fn a_fake_witness_does_not_pass_the_check() {
        let b = batch(vec![signed_tx(0, &[(outpoint(1), 100_000)], 99_000)]);
        let report = verify(&b, &Expectations::default());
        assert!(
            report
                .failures()
                .any(|c| c.kind == CheckKind::Signatures(0)),
            "un testimone di zeri è passato per una firma valida"
        );
    }

    #[test]
    fn a_locktime_other_than_zero_is_a_warning() {
        let about_locktime =
            |report: &Report| report.warnings().any(|c| c.kind == CheckKind::LockTime(0));

        let b = batch(vec![signed_tx(0, &[(outpoint(1), 100_000)], 99_000)]);
        assert!(
            !about_locktime(&verify(&b, &Expectations::default())),
            "con nLockTime a zero non deve esserci l'avviso"
        );

        let mut tx = spending_tx(&[outpoint(1)], 99_000);
        tx.lock_time = LockTime::from_height(800_000).unwrap();
        let mut declared = signed_tx(0, &[(outpoint(1), 100_000)], 99_000);
        declared.txid = tx.compute_txid().to_string();
        declared.hex = serialize_hex(&tx);

        let report = verify(&batch(vec![declared]), &Expectations::default());
        assert!(
            about_locktime(&report),
            "nLockTime a 800000 non ha dato avviso"
        );
    }

    #[test]
    fn two_txs_on_the_same_outpoint_fail() {
        let b = batch(vec![
            signed_tx(0, &[(outpoint(1), 100_000)], 99_000),
            signed_tx(1, &[(outpoint(1), 100_000)], 98_000),
        ]);
        let report = verify(&b, &Expectations::default());
        assert!(!report.is_ok());
        let shared = crate::DisjointnessError::SharedInput {
            outpoint: outpoint(1),
            tx_a: 0,
            tx_b: 1,
        };
        assert!(report
            .failures()
            .any(|c| c.kind == CheckKind::Independence && c.detail == shared.to_string()));
    }

    #[test]
    fn after_a_withdrawal_the_shared_coin_names_the_right_refills() {
        let b = batch(vec![
            signed_tx(0, &[(outpoint(1), 100_000)], 99_000),
            signed_tx(2, &[(outpoint(1), 100_000)], 98_000),
        ]);
        let report = verify(&b, &Expectations::default());
        let shared = crate::DisjointnessError::SharedInput {
            outpoint: outpoint(1),
            tx_a: 0,
            tx_b: 2,
        };
        assert!(
            report
                .failures()
                .any(|c| c.kind == CheckKind::Independence && c.detail == shared.to_string()),
            "il messaggio deve nominare la #1 e la #3, non la posizione nel file"
        );
    }

    #[test]
    fn a_tx_spending_anothers_output_fails() {
        let first = signed_tx(0, &[(outpoint(1), 100_000)], 99_000);
        let first_txid = Txid::from_str(&first.txid).unwrap();
        let second = signed_tx(1, &[(OutPoint::new(first_txid, 0), 99_000)], 98_000);
        let report = verify(&batch(vec![first, second]), &Expectations::default());
        assert!(!report.is_ok());
        let chained = crate::DisjointnessError::ChainedTx {
            outpoint: OutPoint::new(first_txid, 0),
            parent: 0,
            child: 1,
        };
        assert!(report
            .failures()
            .any(|c| c.kind == CheckKind::Independence && c.detail == chained.to_string()));
    }

    #[test]
    fn a_false_declared_fee_fails() {
        let mut b = batch(vec![signed_tx(0, &[(outpoint(1), 100_000)], 99_000)]);
        b.txs[0].fee_sat = 10;
        let report = verify(&b, &Expectations::default());
        assert!(!report.is_ok());
        assert!(report.failures().any(|c| c.kind == CheckKind::Fee(0)));
    }

    /// One refill whose only input carries `witness`, everything else consistent.
    fn with_witness(witness: Witness) -> SignedBatch {
        let mut tx = spending_tx(&[outpoint(1)], 99_000);
        tx.input[0].witness = witness;
        let vsize = tx.vsize() as u64;
        let mut b = batch(vec![signed_tx(0, &[(outpoint(1), 100_000)], 99_000)]);
        b.txs[0].hex = serialize_hex(&tx);
        b.txs[0].txid = tx.compute_txid().to_string();
        b.txs[0].vsize = vsize;
        b.txs[0].fee_sat = 1_000;
        b
    }

    #[test]
    fn an_unsigned_input_is_detected() {
        let report = verify(&with_witness(Witness::new()), &Expectations::default());
        assert!(report
            .failures()
            .any(|c| c.kind == CheckKind::Signatures(0)));
    }

    #[test]
    fn a_signature_that_cannot_be_checked_fails() {
        let not_p2wpkh = Witness::from_slice(&[vec![0u8; 71], vec![0u8; 33], vec![0u8; 1]]);
        let report = verify(&with_witness(not_p2wpkh), &Expectations::default());
        assert!(
            report
                .failures()
                .any(|c| c.kind == CheckKind::Signatures(0) && c.detail.contains("P2WPKH")),
            "una firma che non so verificare è passata con un avviso"
        );
    }

    #[test]
    fn an_input_outside_the_declared_set_fails() {
        let b = batch(vec![signed_tx(0, &[(outpoint(9), 100_000)], 99_000)]);
        let expect = Expectations {
            spendable: Some(vec![outpoint(1), outpoint(2)]),
            ..Default::default()
        };
        let report = verify(&b, &expect);
        assert!(report.failures().any(|c| c.kind == CheckKind::Origin));
    }

    #[test]
    fn a_destination_other_than_expected_fails() {
        let b = batch(vec![signed_tx(0, &[(outpoint(1), 100_000)], 99_000)]);
        let expect = Expectations {
            recipients: Some(vec!["un-altro-indirizzo".into()]),
            ..Default::default()
        };
        let report = verify(&b, &expect);
        assert!(report.failures().any(|c| c.kind == CheckKind::Recipients));
    }
}
