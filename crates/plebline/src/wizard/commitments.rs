//! Committed coins: those inside signed files, of this ceremony or of any
//! other on this machine, read from the files themselves.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use comfy_table::Cell;
use plebline_core::batch;
use plebline_core::bitcoin::consensus::encode::deserialize_hex;
use plebline_core::bitcoin::{OutPoint, Transaction};
use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::utxos::ValidatedUtxoSet;
use plebline_core::verify as verifier;
use plebline_core::{Error, Result};

use crate::ui;

/// Committed coins, read and shown in the coin table. Inputs of past
/// batches count too, or two ceremonies over the same utxos.json conflict.
/// `out`: the signed batch this run writes or sits next to.
pub fn read_commitments(
    utxo_set: &ValidatedUtxoSet,
    previous: &[PathBuf],
    out: &Path,
) -> Result<BTreeSet<OutPoint>> {
    let wallet_checksum = utxo_set.descriptor_checksum.as_deref();
    // Other ceremonies only when batches can be attributed by wallet.
    let mut sources = previous.to_vec();
    if wallet_checksum.is_some() {
        sources.extend(crate::project::all_signed_files());
    }
    let committed = committed_outpoints(&sources, out, wallet_checksum)?;
    describe_utxos(utxo_set, &committed);
    if wallet_checksum.is_none() {
        ui::warning(t(
            "utxos.json non dice da quale wallet vengono queste monete, quindi per ora ho \
             guardato le firme di questa cerimonia soltanto. Le altre cerimonie le guardo appena \
             il wallet è aperto, col suo nome vero.",
            "utxos.json does not say which wallet these coins come from, so for now I only \
             looked at the signatures of this ceremony. I look at the other ceremonies as soon \
             as the wallet is open, under its real name.",
        ));
    }
    Ok(committed)
}

/// Outpoints committed by earlier signed batches.
///
/// Also reads the destination file if it exists, even when the caller did
/// not list it. `sources` is chosen by the caller, never looked
/// up under `$HOME`. `wallet` (descriptor checksum) skips other wallets'
/// batches; `None` reads every source as this wallet's.
pub fn committed_outpoints(
    sources: &[PathBuf],
    out: &Path,
    wallet: Option<&str>,
) -> Result<BTreeSet<OutPoint>> {
    let mut committed = BTreeSet::new();
    let mut sources: Vec<PathBuf> = sources.to_vec();
    sources.sort();
    sources.dedup();
    if out.is_file() && !sources.iter().any(|p| p == out) {
        sources.push(out.to_path_buf());
    }

    for path in &sources {
        if !path.is_file() {
            continue;
        }
        let batch = verifier::load(path)?;
        // Skip batches signed by another wallet.
        if wallet.is_some_and(|checksum| batch.descriptor_checksum != checksum) {
            continue;
        }
        for tx in &batch.txs {
            committed.extend(coins_spent(path, tx)?);
        }
    }
    Ok(committed)
}

/// The coins a signed transaction spends, read from its signed bytes: the
/// `inputs` field is only a description of them. An unreadable or
/// contradicting file stops everything: a commitment skipped would let a
/// coin be signed twice.
pub(crate) fn coins_spent(path: &Path, tx: &batch::SignedTx) -> Result<Vec<OutPoint>> {
    let file = path.display();
    let signed: Transaction = deserialize_hex(&tx.hex).map_err(|e| {
        Error::Verification(tf!(
            {
                "{file} contiene una transazione illeggibile ({e}): non posso stabilire \
                 quali monete siano già state usate in una firma precedente"
            },
            {
                "{file} contains an unreadable transaction ({e}): I cannot tell which \
                 coins were already used by an earlier signature"
            }
        ))
    })?;
    let spent: Vec<OutPoint> = signed.input.iter().map(|i| i.previous_output).collect();
    let declared: Vec<&str> = tx.inputs.iter().map(|i| i.outpoint.as_str()).collect();
    let actual: Vec<String> = spent.iter().map(ToString::to_string).collect();
    if declared != actual {
        return Err(Error::Verification(tf!(
            {
                "{file} dichiara monete diverse da quelle che la transazione firmata spende: \
                 il file è stato modificato o è rovinato, e non posso sapere quali monete \
                 siano impegnate"
            },
            {
                "{file} declares coins other than those its signed transaction spends: the \
                 file was changed or is damaged, and I cannot tell which coins are committed"
            }
        )));
    }
    Ok(spent)
}

/// The commitments to decide with, once the wallet is open. The name in
/// `utxos.json` chose which signed files were read: under another wallet's
/// name this wallet's refills are skipped and their coins could be signed
/// twice, so a different name is refused. A list with no name read this
/// ceremony only: the open wallet's own name widens the reading.
pub fn commitments_of_open_wallet(
    wallet: &plebline_core::bdk_wallet::Wallet,
    set: &ValidatedUtxoSet,
    shown: BTreeSet<OutPoint>,
    previous: &[PathBuf],
    out: &Path,
) -> Result<BTreeSet<OutPoint>> {
    let actual = wallet.descriptor_checksum(plebline_core::bdk_wallet::KeychainKind::External);
    match &set.descriptor_checksum {
        Some(declared) if *declared == actual => {
            ui::success(&tf!({ "Checksum del descriptor coincide ({actual})." }, {
                "Descriptor checksum matches ({actual})."
            }));
            Ok(shown)
        }
        Some(declared) => Err(Error::Refused(tf!(
            {
                "utxos.json dice che queste monete sono del wallet {declared}, ma il wallet \
                 aperto è {actual}. Sotto un altro nome le ricariche già firmate da questo \
                 wallet non si vedrebbero, e le loro monete potrebbero essere firmate due volte: \
                 rifai l'elenco delle monete con questo wallet."
            },
            {
                "utxos.json says these coins belong to wallet {declared}, but the open wallet \
                 is {actual}. Under another name the refills this wallet already signed would \
                 not be seen, and their coins could be signed twice: make the coin list again \
                 with this wallet."
            }
        ))),
        None => {
            let mut sources = previous.to_vec();
            sources.extend(crate::project::all_signed_files());
            let committed = widen_to_wallet(&shown, &sources, out, &actual)?;
            warn_if_widened(set, &shown, &committed, &actual);
            Ok(committed)
        }
    }
}

/// `shown` plus every signed file of wallet `actual` among `sources`. Only
/// adds: what was shown committed stays committed.
fn widen_to_wallet(
    shown: &BTreeSet<OutPoint>,
    sources: &[PathBuf],
    out: &Path,
    actual: &str,
) -> Result<BTreeSet<OutPoint>> {
    let mut committed = shown.clone();
    committed.extend(committed_outpoints(sources, out, Some(actual))?);
    Ok(committed)
}

fn warn_if_widened(
    set: &ValidatedUtxoSet,
    shown: &BTreeSet<OutPoint>,
    committed: &BTreeSet<OutPoint>,
    actual: &str,
) {
    let more = set
        .utxos
        .iter()
        .filter(|u| committed.contains(&u.outpoint) && !shown.contains(&u.outpoint))
        .count();
    if more > 0 {
        ui::warning(&tf!(
            {
                "Ora che il wallet è aperto ({actual}) ho guardato anche le altre cerimonie: \
                 {more} monete sono dentro ricariche già firmate, e non verranno usate."
            },
            {
                "Now that the wallet is open ({actual}) I also looked at the other ceremonies: \
                 {more} coins are inside refills already signed, and will not be used."
            }
        ));
    }
}

/// What past commitments leave available: the decision taken before
/// building, kept pure for tests.
#[derive(Debug, PartialEq, Eq)]
pub enum Commitments {
    /// Nothing was signed before, so there is nothing to report or check.
    Nothing,
    /// Some coins are spoken for, and what is left is enough.
    Enough { committed: usize, free: usize },
    /// Some coins are spoken for, and what is left is not enough.
    Short {
        committed: usize,
        free: usize,
        wanted: usize,
    },
}

/// `committed` is the raw count from past batches (shown); `free` is
/// counted on the coins actually on hand (decides).
pub fn weigh_commitments(
    coins: impl IntoIterator<Item = OutPoint>,
    committed: &BTreeSet<OutPoint>,
    wanted: usize,
) -> Commitments {
    if committed.is_empty() {
        return Commitments::Nothing;
    }
    let free = coins
        .into_iter()
        .filter(|outpoint| !committed.contains(outpoint))
        .count();
    let committed = committed.len();
    if free < wanted {
        Commitments::Short {
            committed,
            free,
            wanted,
        }
    } else {
        Commitments::Enough { committed, free }
    }
}

impl Commitments {
    /// The two numbers to warn about, when there is anything to warn about.
    pub fn counts(&self) -> Option<(usize, usize)> {
        match self {
            Commitments::Nothing => None,
            Commitments::Enough { committed, free }
            | Commitments::Short {
                committed, free, ..
            } => Some((*committed, *free)),
        }
    }

    /// Wording next to the decision, so they cannot disagree.
    pub fn shortage(&self) -> Option<Error> {
        let Commitments::Short { free, wanted, .. } = self else {
            return None;
        };
        Some(Error::Refused(tf!(
            {
                "servono {wanted} monete libere ma ne restano {free}: trasmetti le \
                 ricariche già firmate, oppure dividi le monete per averne di più"
            },
            {
                "{wanted} free coins are needed but only {free} remain: broadcast the \
                 refills already signed, or split the coins to get more"
            }
        )))
    }
}

/// How many coins there are, and how much they are worth.
#[derive(Debug, Default, PartialEq, Eq)]
struct CoinCount {
    count: usize,
    sats: u64,
}

/// Coins on hand, split by past signatures: free, or committed to a signed
/// but unbroadcast payment (still on chain). `spent` counts commitments whose
/// coin is no longer listed: those were confirmed, and take nothing away.
#[derive(Debug, Default, PartialEq, Eq)]
struct CoinSummary {
    free: CoinCount,
    committed: CoinCount,
    spent: usize,
}

/// Pure: overlays commitments on the coins at hand.
fn summarize_coins(
    coins: impl IntoIterator<Item = (OutPoint, u64)>,
    committed: &BTreeSet<OutPoint>,
) -> CoinSummary {
    let mut stock = CoinSummary::default();
    for (outpoint, sats) in coins {
        let tally = if committed.contains(&outpoint) {
            &mut stock.committed
        } else {
            &mut stock.free
        };
        tally.count += 1;
        tally.sats += sats;
    }
    // Saturating: the two counts come from different sources.
    stock.spent = committed.len().saturating_sub(stock.committed.count);
    stock
}

/// Coin table. Empty `committed` means unchecked: no state column.
fn describe_utxos(set: &ValidatedUtxoSet, committed: &BTreeSet<OutPoint>) {
    let count = set.utxos.len();
    let total = ui::sats(set.total().to_sat());
    ui::success(&tf!(
        { "{count} UTXO validati contro le transazioni sorgente, {total} in totale." },
        { "{count} UTXOs validated against their source transactions, {total} in total." }
    ));
    ui::note(t(
        "Ogni importo e ogni scriptPubKey è stato ricavato dalla transazione grezza, \
         non copiato dal file: se il file mentisse, il controllo sarebbe fallito qui.",
        "Every amount and every scriptPubKey was derived from the raw transaction, not copied \
         from the file: if the file were lying, the check would have failed here.",
    ));

    let looked = !committed.is_empty();
    let mut headers = vec![
        t("Moneta", "Coin"),
        t("Quanto vale", "Worth"),
        t("Ricevuta su", "Received on"),
        t("Conferme", "Confirmations"),
    ];
    if looked {
        headers.push(t("Stato", "State"));
    }

    let mut table = ui::table(&headers);
    for utxo in &set.utxos {
        let mut row = vec![
            Cell::new(ui::short_outpoint(&utxo.outpoint.to_string())),
            ui::right(ui::sats(utxo.value.to_sat())),
            Cell::new(keychain_label(utxo.keychain)),
            ui::right(utxo.confirmations),
        ];
        if looked {
            row.push(Cell::new(if committed.contains(&utxo.outpoint) {
                t("usata da una firma", "used by a signature")
            } else {
                t("libera", "free")
            }));
        }
        table.add_row(row);
    }
    ui::show(&table);

    if looked {
        describe_stock(&summarize_coins(
            set.utxos.iter().map(|u| (u.outpoint, u.value.to_sat())),
            committed,
        ));
    }
}

/// Totals under the table. A coin "used by a signature" is not spent: it
/// is still on chain, reserved by a signed file.
fn describe_stock(stock: &CoinSummary) {
    let mut table = ui::table(&[
        t("Monete", "Coins"),
        t("Quante", "How many"),
        t("Quanto valgono", "Worth"),
    ]);
    table.add_row(vec![
        Cell::new(t("libere", "free")),
        ui::right(stock.free.count),
        ui::right(ui::sats(stock.free.sats)),
    ]);
    table.add_row(vec![
        Cell::new(t("usate da una firma", "used by a signature")),
        ui::right(stock.committed.count),
        ui::right(ui::sats(stock.committed.sats)),
    ]);
    table.add_row(vec![
        Cell::new(t("in tutto", "in total")),
        ui::right(stock.free.count + stock.committed.count),
        ui::right(ui::sats(stock.free.sats + stock.committed.sats)),
    ]);
    ui::show(&table);

    let spoken = stock.committed.count;
    if spoken > 0 {
        ui::warning(&tf!(
            {
                "{spoken} di queste monete sono già dentro una transazione firmata che non è \
                 ancora stata trasmessa, di questa cerimonia o di un'altra su questa macchina."
            },
            {
                "{spoken} of these coins are already inside a signed transaction that has not \
                 been broadcast yet, in this ceremony or in another one on this machine."
            }
        ));
        ui::note(t(
            "Sono ancora tue e sono ancora sulla catena: finché quella transazione non parte \
             nessuno le ha spese, e il saldo che vedi le comprende. Il programma però non le \
             riusa: spenderle qui annullerebbe quella ricarica, perché delle due transazioni \
             ne arriverebbe una sola.",
            "They are still yours and still on the chain: until that transaction goes out \
             nobody has spent them, and the balance you see includes them. The program still \
             does not reuse them: spending them here would cancel that refill, because only \
             one of the two transactions would arrive.",
        ));
    }

    let gone = stock.spent;
    if gone > 0 {
        ui::info(&tf!(
            {
                "Altre {gone} monete usate da firme precedenti non compaiono più fra quelle \
                 disponibili: di solito vuol dire che quelle transazioni sono già state \
                 trasmesse e confermate."
            },
            {
                "Another {gone} coins used by earlier signatures are no longer among the \
                 available ones: usually that means those transactions have already been \
                 broadcast and confirmed."
            }
        ));
    }
}

/// "Keychain" in the user's words: receiving vs change addresses.
fn keychain_label(keychain: plebline_core::bdk_wallet::KeychainKind) -> &'static str {
    match keychain {
        plebline_core::bdk_wallet::KeychainKind::External => t("ricevuta", "received"),
        plebline_core::bdk_wallet::KeychainKind::Internal => t("resto", "change"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{batch_of, outpoint, set_of, Temp};
    use crate::wizard::files::write_json;

    #[test]
    fn with_nothing_signed_before_there_is_nothing_to_say() {
        let weighed = weigh_commitments(set_of(&[1, 2]), &BTreeSet::new(), 2);

        assert_eq!(weighed, Commitments::Nothing);
        assert_eq!(weighed.counts(), None, "non c'è niente da mostrare");
        assert!(weighed.shortage().is_none(), "e niente da rifiutare");
    }

    #[test]
    fn a_committed_coin_is_not_free() {
        let weighed = weigh_commitments(set_of(&[1, 2, 3]), &set_of(&[2]), 2);

        assert_eq!(
            weighed,
            Commitments::Enough {
                committed: 1,
                free: 2
            }
        );
        assert!(weighed.shortage().is_none(), "due libere bastano per due");
    }

    /// Signing over committed coins is refused.
    #[test]
    fn a_ceremony_the_free_coins_cannot_cover_is_refused() {
        let weighed = weigh_commitments(set_of(&[1, 2]), &set_of(&[1]), 2);

        assert_eq!(
            weighed,
            Commitments::Short {
                committed: 1,
                free: 1,
                wanted: 2
            }
        );
        let refusal = weighed.shortage().expect("una moneta non basta per due");
        assert!(
            matches!(refusal, Error::Refused(_)),
            "rifiuto sbagliato: {refusal:?}"
        );
        let said = refusal.to_string();
        assert!(
            said.contains('2') && said.contains('1'),
            "il rifiuto non dice i due numeri che lo motivano: {said}"
        );
    }

    /// Commitments to coins no longer listed do not reduce what is free.
    #[test]
    fn commitments_outside_the_set_take_away_no_free_coin() {
        let weighed = weigh_commitments(set_of(&[1, 2]), &set_of(&[8, 9]), 2);

        assert_eq!(
            weighed,
            Commitments::Enough {
                committed: 2,
                free: 2
            }
        );
    }

    /// The coin `i`, worth `sats`, in the shape `summarize_coins` reads.
    fn coin(i: usize, sats: u64) -> (OutPoint, u64) {
        (outpoint(i), sats)
    }

    #[test]
    fn with_nothing_signed_before_every_coin_is_free() {
        let stock = summarize_coins(
            [coin(1, 10_000), coin(2, 25_000)],
            &std::collections::BTreeSet::new(),
        );

        assert_eq!(stock.free.count, 2);
        assert_eq!(stock.free.sats, 35_000, "il totale libero è la somma");
        assert_eq!(stock.committed, CoinCount::default(), "niente è impegnato");
        assert_eq!(stock.spent, 0);
    }

    /// Values are shown next to counts.
    #[test]
    fn a_coin_used_by_an_earlier_signature_is_counted_apart_with_its_value() {
        let stock = summarize_coins(
            [coin(1, 10_000), coin(2, 25_000), coin(3, 5_000)],
            &set_of(&[2]),
        );

        assert_eq!(stock.free.count, 2);
        assert_eq!(stock.free.sats, 15_000);
        assert_eq!(stock.committed.count, 1);
        assert_eq!(
            stock.committed.sats, 25_000,
            "la moneta usata da una firma vale ancora, ed è ancora sulla catena"
        );
        assert_eq!(stock.spent, 0, "quella moneta c'è: non è sparita");
    }

    /// Confirmed commitments are counted apart, not as committed.
    #[test]
    fn a_commitment_whose_coin_is_no_longer_listed_is_counted_as_spent() {
        let stock = summarize_coins([coin(1, 10_000), coin(2, 25_000)], &set_of(&[8, 9]));

        assert_eq!(stock.free.count, 2, "nessuna delle due è impegnata");
        assert_eq!(stock.free.sats, 35_000);
        assert_eq!(stock.committed, CoinCount::default());
        assert_eq!(
            stock.spent, 2,
            "i due impegni vanno detti, ma contati a parte"
        );
    }

    /// The coins a file commits come from its signed bytes. A file whose
    /// description says other coins is refused: trusting the description
    /// would leave the coin the bytes really spend free for a second refill.
    #[test]
    fn a_file_describing_other_coins_than_it_spends_is_refused() {
        let dir = Temp::new("impegni-descrizione-falsa");
        let out = dir.path.join(crate::project::BATCH);
        let mut batch = batch_of(&[1]);
        batch.txs[0].inputs[0].outpoint = outpoint(2).to_string();
        write_json(&out, &batch).expect("scritto");

        let refusal = committed_outpoints(&[], &out, None).expect_err("va rifiutato");
        assert!(matches!(refusal, Error::Verification(_)), "{refusal}");
    }

    #[test]
    fn the_destination_file_commits_its_own_inputs() {
        let dir = Temp::new("impegni-destinazione");
        let out = dir.path.join(crate::project::BATCH);
        write_json(&out, &batch_of(&[1, 2])).expect("scritto");

        let committed = committed_outpoints(&[], &out, None).expect("letti");

        assert_eq!(
            committed,
            set_of(&[1, 2]),
            "se il file di destinazione c'è, i suoi impegni valgono anche se nessuno lo \
             elenca"
        );
    }

    #[test]
    fn previous_batches_and_the_destination_are_read_together() {
        let dir = Temp::new("impegni-precedenti");
        let previous = dir.path.join("batch-1.json");
        write_json(&previous, &batch_of(&[3])).expect("scritto");
        let out = dir.path.join(crate::project::BATCH);
        write_json(&out, &batch_of(&[4])).expect("scritto");

        let committed = committed_outpoints(&[previous], &out, None).expect("letti");

        assert_eq!(committed, set_of(&[3, 4]));
    }

    #[test]
    fn a_destination_not_there_yet_commits_nothing() {
        let dir = Temp::new("impegni-nessuno");

        let committed =
            committed_outpoints(&[], &dir.path.join(crate::project::BATCH), None).expect("letti");

        assert!(committed.is_empty());
    }

    /// An unreadable outpoint stops everything: it cannot be respected.
    #[test]
    fn an_unreadable_outpoint_stops_the_ceremony() {
        let dir = Temp::new("impegni-illeggibile");
        let out = dir.path.join(crate::project::BATCH);
        let mut batch = batch_of(&[1]);
        batch.txs[0].inputs[0].outpoint = "non-un-outpoint".into();
        write_json(&out, &batch).expect("scritto");

        let error = committed_outpoints(&[], &out, None).expect_err("outpoint illeggibile");

        assert!(
            matches!(error, Error::Verification(_)),
            "errore sbagliato: {error:?}"
        );
    }

    /// Other wallets' batches are read but not counted.
    #[test]
    fn another_wallets_batch_is_not_a_commitment_of_this_one() {
        let dir = Temp::new("impegni-altro-wallet");
        let mine = dir.path.join("batch-1.json");
        write_json(&mine, &batch_of(&[1])).expect("scritto");

        let theirs = dir.path.join("batch-2.json");
        let mut foreign = batch_of(&[7]);
        foreign.descriptor_checksum = "bbbbbbbb".into();
        write_json(&theirs, &foreign).expect("scritto");

        let committed = committed_outpoints(
            &[mine, theirs],
            &dir.path.join(crate::project::BATCH),
            Some("aaaaaaaa"),
        )
        .expect("letti");

        assert_eq!(
            committed,
            set_of(&[1]),
            "la moneta 7 è impegnata da un altro cold storage: qui non vincola niente"
        );
    }

    /// Without a checksum, every source counts as this wallet's.
    #[test]
    fn with_no_checksum_every_source_is_read() {
        let dir = Temp::new("impegni-senza-checksum");
        let mine = dir.path.join("batch-1.json");
        write_json(&mine, &batch_of(&[1])).expect("scritto");

        let theirs = dir.path.join("batch-2.json");
        let mut foreign = batch_of(&[7]);
        foreign.descriptor_checksum = "bbbbbbbb".into();
        write_json(&theirs, &foreign).expect("scritto");

        let committed =
            committed_outpoints(&[mine, theirs], &dir.path.join(crate::project::BATCH), None)
                .expect("letti");

        assert_eq!(committed, set_of(&[1, 7]));
    }

    #[test]
    fn a_source_repeated_commits_the_same_thing_once() {
        let dir = Temp::new("impegni-ripetuti");
        let out = dir.path.join(crate::project::BATCH);
        write_json(&out, &batch_of(&[1, 2])).expect("scritto");

        let committed = committed_outpoints(&[out.clone(), out.clone()], &out, Some("aaaaaaaa"))
            .expect("letti");

        assert_eq!(committed, set_of(&[1, 2]));
    }

    /// A coin list with no wallet name read this ceremony only; once the
    /// wallet is open, its own signed files elsewhere count too, another
    /// wallet's do not, and nothing shown before is dropped.
    #[test]
    fn the_open_wallet_widens_what_a_nameless_list_could_see() {
        let dir = Temp::new("lista-senza-nome");
        let out = dir.path.join(crate::project::BATCH);
        write_json(&out, &batch_of(&[1])).expect("scritto");
        let elsewhere = dir.path.join("altra-cerimonia.json");
        write_json(&elsewhere, &batch_of(&[2])).expect("scritto");
        let theirs = dir.path.join("altro-wallet.json");
        let mut foreign = batch_of(&[3]);
        foreign.descriptor_checksum = "bbbbbbbb".into();
        write_json(&theirs, &foreign).expect("scritto");

        let shown = committed_outpoints(&[], &out, None).expect("letti");
        assert_eq!(shown, set_of(&[1]));

        let sources = [elsewhere, theirs];
        let widened = widen_to_wallet(&shown, &sources, &out, "aaaaaaaa").expect("letti");
        assert_eq!(widened, set_of(&[1, 2]));

        // This ceremony's batch from another wallet: still committed.
        let widened = widen_to_wallet(&shown, &sources, &out, "bbbbbbbb").expect("letti");
        assert_eq!(widened, set_of(&[1, 3]));
    }

    /// A signed file that cannot be read stops the ceremony: skipping it
    /// would free the coins it commits.
    #[test]
    fn a_damaged_signed_file_stops_the_ceremony() {
        let dir = Temp::new("firmato-rotto");
        let out = dir.path.join(crate::project::BATCH);
        let archive = dir.path.join("batch-1.json");
        write_json(&out, &batch_of(&[1])).expect("scritto");

        let whole = serde_json::to_string(&batch_of(&[2])).expect("serializzabile");
        for damaged in [&whole[..whole.len() / 2], "", "{}", "[]"] {
            std::fs::write(&archive, damaged).expect("scrivibile");
            assert!(
                committed_outpoints(std::slice::from_ref(&archive), &out, None).is_err(),
                "letto come buono: {damaged:?}"
            );
        }

        let mut absurd = batch_of(&[2]);
        absurd.txs[0].inputs[0].value_sat = u64::MAX;
        write_json(&archive, &absurd).expect("scritto");
        assert!(committed_outpoints(std::slice::from_ref(&archive), &out, None).is_err());
    }

    /// A folder with a signed file's name commits nothing, and stops nothing.
    #[test]
    fn a_folder_named_like_a_signed_file_is_not_one() {
        let dir = Temp::new("firmato-cartella");
        let out = dir.path.join(crate::project::BATCH);
        write_json(&out, &batch_of(&[1])).expect("scritto");
        let folder = dir.path.join("batch-1.json");
        std::fs::create_dir_all(&folder).expect("cartella creabile");

        let committed =
            committed_outpoints(std::slice::from_ref(&folder), &out, None).expect("letti");
        assert_eq!(committed, set_of(&[1]));
    }
}
