//! Scanning and broadcasting, from public descriptors only: no function
//! here can reach a private key.
//!
//! Packets leave from [`crate::chain`]; this module keeps what is testable
//! without a server: which coins to drop, confirmations, files.

use plebline_core::batch::SignedTx;
use plebline_core::bitcoin::consensus::encode::deserialize_hex;
use plebline_core::bitcoin::{Network, Transaction};
use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::utxos::{UtxoEntry, UtxoSet, UTXOS_SCHEMA_VERSION};
use plebline_core::{Error, Result};

use comfy_table::Cell;

use crate::ask::{Ask, Question};
use crate::chain::{Chain, Coin, ScanOptions, ScanResult};
use crate::ui::{self, Wizard};

/// Conventional gap limit. The scan summary says when to raise it.
pub const DEFAULT_STOP_GAP: usize = 20;

pub struct ExportInput {
    pub wallet: std::path::PathBuf,
    pub out: std::path::PathBuf,
}

pub struct BroadcastInput {
    pub batch: std::path::PathBuf,
    pub index: usize,
}

/// Public Esplora servers, in the order they are tried: one operator going
/// down must not stop the ceremony.
///
/// Onion addresses first: no exit node, no name resolution anywhere. Plain
/// `http` is fine there, Tor already authenticates and encrypts onion
/// services. Clearnet through Tor last (`mempool.space` that way: 90 s
/// measured). A node of one's own is typed by the user instead.
pub fn default_apis(network: Network) -> Result<&'static [&'static str]> {
    match network {
        Network::Signet => Ok(&[
            "http://mempoolhqx4isw62xs7abwphsq7ldayuidyx2v2oethdhhj6mlo2r6ad.onion/signet/api",
            "http://explorerzydxu5ecjrkwceayqybizmpjjznk5izmitf2modhcusuqlid.onion/signet/api",
            "https://blockstream.info/signet/api",
            "https://mempool.space/signet/api",
        ]),
        Network::Bitcoin => Ok(&[
            "http://explorerzydxu5ecjrkwceayqybizmpjjznk5izmitf2modhcusuqlid.onion/api",
            "http://mempoolhqx4isw62xs7abwphsq7ldayuidyx2v2oethdhhj6mlo2r6ad.onion/api",
            "https://blockstream.info/api",
            "https://mempool.space/api",
        ]),
        other => Err(Error::Refused(tf!(
            { "nessun Esplora pubblico per {other}: serve un nodo tuo" },
            { "no public Esplora for {other}: a node of your own is needed" }
        ))),
    }
}

/// The first of [`default_apis`].
pub fn default_api(network: Network) -> Result<&'static str> {
    default_apis(network).map(|list| list[0])
}

/// Scan attempts before giving up. A Tor circuit that stalls is common and
/// passing, and each attempt gets a fresh one (see [`crate::chain::Esplora`]).
pub const SCAN_ATTEMPTS: usize = 3;

/// The server or the connection failed, which is worth another try. A
/// refusal by [`crate::tor`] is never retried.
pub fn worth_retrying(problem: &Error) -> bool {
    matches!(problem, Error::Network(_))
}

/// Reads `wallet.json`, checks its network before any packet, and scans,
/// trying again by itself when the network fails: giving up here would
/// mean typing the words again.
pub fn scan(
    network: Network,
    api: &str,
    wallet_path: &std::path::Path,
    options: ScanOptions,
    chain: &mut dyn Chain,
) -> Result<ScanResult> {
    let file = plebline_core::watch::load(wallet_path, network)?;
    let checksum = &file.descriptor_checksum;
    ui::success(&tf!({ "Wallet {checksum}, rete {network}." }, {
        "Wallet {checksum}, network {network}."
    }));

    let mut attempt = 1;
    loop {
        match chain.scan(api, &file, options) {
            Err(problem) if attempt < SCAN_ATTEMPTS && worth_retrying(&problem) => {
                attempt += 1;
                ui::warning(&tf!(
                    {
                        "Non è andata: {problem}. Riprovo da solo (tentativo {attempt} di \
                         {SCAN_ATTEMPTS})."
                    },
                    {
                        "That did not work: {problem}. Trying again by myself (attempt {attempt} \
                         of {SCAN_ATTEMPTS})."
                    }
                ));
            }
            outcome => return outcome,
        }
    }
}

/// The coins in a block, and how many were not. Presigning on an
/// unconfirmed input is fragile: a replaced parent voids the signature.
pub fn confirmed(coins: &[Coin]) -> (Vec<Coin>, usize) {
    let (kept, dropped): (Vec<&Coin>, Vec<&Coin>) =
        coins.iter().partition(|coin| coin.height.is_some());
    (kept.into_iter().cloned().collect(), dropped.len())
}

/// Kept coins, and dropped ones with the reason.
#[derive(Debug)]
struct Kept {
    entries: Vec<UtxoEntry>,
    skipped_coinbase: usize,
}

/// Coins that go into `utxos.json`: an unconfirmed one counts zero
/// confirmations, a coinbase is dropped.
fn keep(coins: &[Coin], tip: u32) -> Result<Kept> {
    let mut kept = Kept {
        entries: Vec::new(),
        skipped_coinbase: 0,
    };

    for coin in coins {
        let origin = coin.origin.as_ref().ok_or_else(|| {
            let outpoint = &coin.outpoint;
            Error::Network(tf!(
                { "manca la transazione che ha creato {outpoint}: non scrivo un elenco a metà" },
                { "the transaction that created {outpoint} is missing: not writing a half list" }
            ))
        })?;

        // Coinbase is unsupported offline: say so now.
        if origin.coinbase {
            kept.skipped_coinbase += 1;
            continue;
        }

        kept.entries.push(UtxoEntry {
            outpoint: coin.outpoint.clone(),
            value_sat: coin.value_sat,
            script_pubkey: coin.script_pubkey.clone(),
            keychain: coin.keychain.clone(),
            derivation_index: coin.derivation_index,
            confirmations: coin.height.map(|h| tip.saturating_sub(h) + 1).unwrap_or(0),
            prev_tx_hex: origin.hex.clone(),
        });
    }

    Ok(kept)
}

/// Writes `utxos.json` from a scan: the usual gap limit, confirmed coins
/// only. The steps are public so another composition can scan wider or
/// keep other coins; this one is the window's.
pub fn export_utxos(
    network: Network,
    api: &str,
    input: ExportInput,
    chain: &mut dyn Chain,
) -> Result<bool> {
    let export = Export::begin(network, api, input, DEFAULT_STOP_GAP, chain)?;
    let (coins, unconfirmed) = confirmed(export.coins());
    export.write(&coins, unconfirmed)
}

/// An export under way: scanned, not yet written. The caller picks the
/// coins between [`Export::begin`] and [`Export::write`].
pub struct Export {
    network: Network,
    out: std::path::PathBuf,
    stop_gap: usize,
    wizard: Wizard,
    scanned: ScanResult,
}

impl Export {
    /// Title and scan, with each coin's source transaction (step 1 of 3).
    pub fn begin(
        network: Network,
        api: &str,
        input: ExportInput,
        stop_gap: usize,
        chain: &mut dyn Chain,
    ) -> Result<Self> {
        ui::title(t(
            "Aggiorno l'elenco delle monete",
            "Refreshing the list of coins",
        ));
        ui::note(t(
            "Chiedo alla rete quali monete risultano tue e ne scrivo l'elenco, che servirà per \
             firmare. Nessuna chiave è coinvolta: da qui non si può spendere niente.",
            "I ask the network which coins turn out to be yours and write down the list, which \
             signing will need. No key is involved: nothing can be spent from here.",
        ));

        let mut wizard = Wizard::new(3);

        wizard.step(t("Scansione della catena", "Scanning the chain"));
        // Source transactions let offline verify value and script.
        let scanned = scan(
            network,
            api,
            &input.wallet,
            ScanOptions {
                stop_gap,
                origins: true,
                reveal: 0,
            },
            chain,
        )?;
        let tip = scanned.tip;
        ui::info(&tf!({ "Altezza catena: {tip}" }, { "Chain height: {tip}" }));

        Ok(Self {
            network,
            out: input.out,
            stop_gap,
            wizard,
            scanned,
        })
    }

    /// Every coin the scan found, unconfirmed and coinbase included.
    pub fn coins(&self) -> &[Coin] {
        &self.scanned.coins
    }

    /// Steps 2 and 3: keeps `coins` (coinbase dropped) and writes the file.
    /// `skipped_unconfirmed` is only reported.
    pub fn write(mut self, coins: &[Coin], skipped_unconfirmed: usize) -> Result<bool> {
        let network = self.network;
        let tip = self.scanned.tip;

        self.wizard
            .step(t("Raccolta degli UTXO", "Collecting the UTXOs"));
        let Kept {
            entries,
            skipped_coinbase,
        } = keep(coins, tip)?;

        if entries.is_empty() {
            ui::failure(t(
                "Nessun UTXO spendibile trovato.",
                "No spendable UTXO found.",
            ));
            report_skips(skipped_unconfirmed, skipped_coinbase);
            let gap = self.stop_gap;
            ui::note(&tf!(
                {
                    "Se ti aspettavi delle monete: controlla di essere sulla rete giusta \
                     ({network}), e che gli indirizzi usati stiano entro i primi {gap} del wallet."
                },
                {
                    "If you expected coins: check you are on the right network ({network}), and \
                     that the addresses used fall within the first {gap} of the wallet."
                }
            ));
            return Ok(false);
        }

        let set = UtxoSet {
            version: UTXOS_SCHEMA_VERSION,
            network: network.to_string(),
            generated_at: now(),
            tip_height: tip,
            descriptor_checksum: Some(self.scanned.descriptor_checksum),
            // Beyond the used ones, for gap-limit addresses.
            last_active_indices: self.scanned.last_active_indices,
            utxos: entries,
        };

        print_utxos(&set);
        report_skips(skipped_unconfirmed, skipped_coinbase);

        self.wizard.step(t("Scrittura", "Writing"));
        crate::wizard::write_json(&self.out, &set)?;
        let out = self.out.display();
        let count = set.utxos.len();
        ui::success(&tf!({ "Scritto {out}: {count} UTXO." }, {
            "Written {out}: {count} UTXOs."
        }));

        // Coins = maximum independent presigned payments: say it here.
        ui::blank();
        ui::info(&tf!(
            {
                "Con {count} UTXO puoi prefirmare al massimo {count} ricariche indipendenti: \
                 ognuna deve spendere una moneta diversa."
            },
            {
                "With {count} UTXOs you can pre-sign at most {count} independent refills: each \
                 one must spend a different coin."
            }
        ));

        Ok(true)
    }
}

/// Shows one transaction of a signed file, asks, sends: the window's path,
/// composed of [`prepare`], [`confirm`] and [`send`].
pub fn broadcast(
    network: Network,
    api: &str,
    input: BroadcastInput,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<bool> {
    let outgoing = prepare(network, &input)?;
    let Some(confirmed) = confirm(network, &outgoing, ask)? else {
        return Ok(false);
    };
    send(api, confirmed, chain)?;
    Ok(true)
}

/// A transaction read from its signed file, checked and shown: the only
/// thing [`send`] takes, so nothing reaches the network without [`prepare`].
pub struct Outgoing {
    tx: SignedTx,
    parsed: Transaction,
    /// The other transactions of the file, still broadcastable afterwards.
    others: Vec<usize>,
}

/// Reads the file, checks network and txid, shows the transaction. No
/// packet leaves.
pub fn prepare(network: Network, input: &BroadcastInput) -> Result<Outgoing> {
    ui::title(t("Invio alla rete", "Sending to the network"));

    let signed = plebline_core::verify::load(&input.batch)?;
    if signed.network != network.to_string() {
        return Err(Error::NetworkMismatch {
            found: signed.network.clone(),
            expected: network.to_string(),
        });
    }

    let index = input.index;
    let tx = signed
        .txs
        .iter()
        .find(|tx| tx.index == index)
        .ok_or_else(|| {
            let last = signed.txs.len().saturating_sub(1);
            Error::Input(tf!(
                { "nessuna transazione con indice {index}: il batch va da 0 a {last}" },
                { "no transaction with index {index}: the batch goes from 0 to {last}" }
            ))
        })?;

    // The bytes decide: if they give another txid, nothing is broadcast.
    let parsed: Transaction = deserialize_hex(&tx.hex).map_err(|e| {
        Error::Verification(tf!({ "hex non decodificabile: {e}" }, {
            "hex cannot be decoded: {e}"
        }))
    })?;
    let computed = parsed.compute_txid();
    if computed.to_string() != tx.txid {
        let declared = &tx.txid;
        return Err(Error::Verification(tf!(
            { "il file dichiara txid {declared} ma i byte danno {computed}: non trasmetto" },
            { "the file declares txid {declared} but the bytes give {computed}: not broadcasting" }
        )));
    }

    print_transaction(tx);

    let others = signed
        .txs
        .iter()
        .map(|tx| tx.index)
        .filter(|i| *i != index)
        .collect();
    Ok(Outgoing {
        tx: tx.clone(),
        parsed,
        others,
    })
}

/// Asks before sending; `None` says it was cancelled. On mainnet the
/// amount must be typed, not just Enter (a split, which pays nobody, asks
/// yes or no). Test networks skip this friction.
pub fn confirm<'a>(
    network: Network,
    outgoing: &'a Outgoing,
    ask: &mut dyn Ask,
) -> Result<Option<Confirmed<'a>>> {
    if !confirm_broadcast(network, &outgoing.tx, ask)? {
        ui::info(t("Annullato.", "Cancelled."));
        return Ok(None);
    }
    Ok(Some(Confirmed(outgoing)))
}

/// A yes from [`confirm`]: the only thing [`send`] takes, so nothing reaches
/// the network without the question, and on mainnet without the amount.
pub struct Confirmed<'a>(&'a Outgoing);

/// Sends, and says what remains broadcastable.
pub fn send(api: &str, confirmed: Confirmed<'_>, chain: &mut dyn Chain) -> Result<()> {
    let outgoing = confirmed.0;
    chain.send(api, &outgoing.parsed)?;

    let txid = outgoing.parsed.compute_txid();
    ui::success(&tf!({ "Trasmessa: {txid}" }, { "Broadcast: {txid}" }));

    let remaining: Vec<String> = outgoing
        .others
        .iter()
        .map(|i| format!("#{}", plebline_core::batch::number(*i)))
        .collect();
    if !remaining.is_empty() {
        ui::blank();
        let list = remaining.join(", ");
        ui::note(&tf!(
            {
                "Restano trasmissibili le transazioni {list}, in qualsiasi ordine e in \
                 qualsiasi momento: questa non le ha invalidate."
            },
            {
                "Transactions {list} remain broadcastable, in any order and at any time: this \
                 one did not invalidate them."
            }
        ));
    }

    Ok(())
}

fn confirm_broadcast(network: Network, tx: &SignedTx, ask: &mut dyn Ask) -> Result<bool> {
    let leaving: u64 = tx
        .outputs
        .iter()
        .filter(|o| !o.is_change)
        .map(|o| o.value_sat)
        .sum();

    ui::blank();
    ui::note(t(
        "La trasmissione non è reversibile. Le altre transazioni del batch restano valide: \
         questa non le tocca.",
        "Broadcasting cannot be undone. The other transactions in the batch stay valid: this \
         one does not touch them.",
    ));

    if network != Network::Bitcoin {
        ui::blank();
        return ui::confirm(
            ask,
            t(
                "Trasmetto questa transazione?",
                "Broadcast this transaction?",
            ),
        )
        .map_err(Error::from);
    }

    if leaving == 0 {
        // A split: the coins only move between this wallet's addresses.
        let fee = ui::sats(tx.fee_sat);
        ui::blank();
        ui::warning(&tf!(
            { "MAINNET: dal portafoglio non esce niente, tranne {fee} di commissione." },
            { "MAINNET: nothing leaves the wallet except {fee} in fees." }
        ));
        ui::blank();
        return ui::confirm(
            ask,
            t(
                "Trasmetto questa transazione?",
                "Broadcast this transaction?",
            ),
        )
        .map_err(Error::from);
    }

    let btc = ui::btc(leaving);
    ui::blank();
    ui::warning(&tf!(
        { "MAINNET: stanno uscendo {leaving} sat ({btc}). Sono monete vere." },
        { "MAINNET: {leaving} sat ({btc}) are leaving. These are real coins." }
    ));
    ui::blank();

    let typed = ask.text(
        Question::new(t(
            "Scrivi l'importo in sat per confermare:",
            "Type the amount in sat to confirm:",
        ))
        .help(t(
            "le cifre, anche separate da spazi o punti; un numero diverso annulla",
            "the digits, with spaces or dots between them if you like; any other number \
             cancels",
        )),
        None,
    )?;

    let matches = typed.trim().replace(['.', ',', '_', ' '], "") == leaving.to_string();
    if !matches {
        ui::failure(t(
            "L'importo non corrisponde: non trasmetto niente.",
            "The amount does not match: nothing was broadcast.",
        ));
    }
    Ok(matches)
}

fn report_skips(unconfirmed: usize, coinbase: usize) {
    if unconfirmed > 0 {
        ui::warning(&tf!(
            {
                "{unconfirmed} UTXO non confermati esclusi. Prefirmare su un input non \
                 confermato è fragile: se la transazione che lo crea viene rimpiazzata, la \
                 firma non vale più niente."
            },
            {
                "{unconfirmed} unconfirmed UTXOs skipped. Pre-signing on an unconfirmed input is \
                 fragile: if the transaction that creates it gets replaced, the signature is \
                 worth nothing."
            }
        ));
    }
    if coinbase > 0 {
        ui::warning(&tf!(
            {
                "{coinbase} UTXO da coinbase esclusi: non sono ancora supportati. Spendili una \
                 volta verso il cold storage e riesporta."
            },
            {
                "{coinbase} coinbase UTXOs skipped: not supported yet. Spend them once towards \
                 cold storage and export again."
            }
        ));
    }
}

fn print_utxos(set: &UtxoSet) {
    let mut table = ui::table(&[
        t("Moneta", "Coin"),
        t("Quanto vale", "Worth"),
        t("Conferme", "Confirmations"),
    ]);
    for utxo in &set.utxos {
        table.add_row(vec![
            Cell::new(ui::short_outpoint(&utxo.outpoint)),
            ui::right(ui::sats(utxo.value_sat)),
            ui::right(utxo.confirmations),
        ]);
    }
    ui::show(&table);

    let total: u64 = set.utxos.iter().map(|u| u.value_sat).sum();
    let count = set.utxos.len();
    let amount = ui::sats(total);
    let btc = ui::btc(total);
    ui::info(&tf!({ "{count} UTXO, {amount} ({btc}) in totale." }, {
        "{count} UTXOs, {amount} ({btc}) in total."
    }));
}

fn print_transaction(tx: &SignedTx) {
    let index = plebline_core::batch::number(tx.index);
    let label = tx.label.clone().unwrap_or_else(|| "—".to_string());
    let txid = &tx.txid;
    ui::info(&tf!({ "Transazione #{index}  {label}" }, {
        "Transaction #{index}  {label}"
    }));
    ui::info(&format!("txid {txid}"));

    let mut table = ui::table(&[
        t("Cosa", "What"),
        t("Dove", "Where"),
        t("Quanto", "How much"),
    ]);
    for input in &tx.inputs {
        table.add_row(vec![
            Cell::new(t("spende la moneta", "spends the coin")),
            Cell::new(ui::short_outpoint(&input.outpoint)),
            ui::right(ui::sats(input.value_sat)),
        ]);
    }
    for output in &tx.outputs {
        table.add_row(vec![
            Cell::new(if output.is_change {
                t("ti torna indietro", "comes back to you")
            } else {
                t("paga", "pays")
            }),
            Cell::new(&output.address),
            ui::right(ui::sats(output.value_sat)),
        ]);
    }
    table.add_row(vec![
        Cell::new("fee"),
        Cell::new(format!("{:.2} sat/vB", tx.fee_rate_sat_vb)),
        ui::right(ui::sats(tx.fee_sat)),
    ]);
    ui::show(&table);
}

/// Timestamps the export; offline never reads the clock.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plebline_core::batch::SignedOutput;

    fn coin(value: u64, mined_at: Option<u32>, coinbase: bool) -> Coin {
        Coin {
            outpoint: format!("{value:064}:0"),
            value_sat: value,
            script_pubkey: "0014".repeat(6),
            keychain: "external".into(),
            derivation_index: 0,
            height: mined_at,
            origin: Some(crate::chain::Origin {
                hex: "0200".into(),
                coinbase,
            }),
        }
    }

    #[test]
    fn confirmations_are_counted_from_the_tip() {
        // Its own block counts: ten confirmations.
        let kept = keep(&[coin(100_000, Some(799_991), false)], 800_000).expect("tenuta");
        assert_eq!(kept.entries.len(), 1);
        assert_eq!(kept.entries[0].confirmations, 10);
    }

    /// Unconfirmed inputs are fragile: a replaced parent voids the signature.
    #[test]
    fn an_unconfirmed_coin_is_dropped() {
        let (kept, dropped) = confirmed(&[coin(100_000, None, false)]);
        assert!(kept.is_empty());
        assert_eq!(dropped, 1);
    }

    /// Kept on purpose (another composition), it counts zero confirmations.
    #[test]
    fn an_unconfirmed_coin_kept_counts_zero_confirmations() {
        let kept = keep(&[coin(100_000, None, false)], 800_000).expect("tenuta");
        assert_eq!(kept.entries.len(), 1);
        assert_eq!(kept.entries[0].confirmations, 0);
    }

    #[test]
    fn a_coinbase_is_dropped_even_when_confirmed() {
        let kept = keep(&[coin(100_000, Some(700_000), true)], 800_000).expect("tenuta");
        assert!(kept.entries.is_empty());
        assert_eq!(kept.skipped_coinbase, 1);
    }

    /// Unconfirmed is reported before coinbase: the user can wait.
    #[test]
    fn an_unconfirmed_coinbase_is_counted_only_once() {
        let (kept, dropped) = confirmed(&[coin(100_000, None, true)]);
        assert_eq!(dropped, 1);
        let kept = keep(&kept, 800_000).expect("tenuta");
        assert_eq!(kept.skipped_coinbase, 0);
    }

    #[test]
    fn a_coin_without_its_origin_stops_the_export() {
        let mut orphan = coin(100_000, Some(700_000), false);
        orphan.origin = None;
        let error = keep(&[orphan], 800_000).expect_err("rifiutata");
        assert!(error.to_string().contains("elenco a metà"), "{error}");
    }

    #[test]
    fn the_coins_fields_reach_the_file_whole() {
        let kept = keep(&[coin(123_456, Some(799_999), false)], 800_000).expect("tenuta");
        let written = &kept.entries[0];
        assert_eq!(written.value_sat, 123_456);
        assert_eq!(written.outpoint, format!("{:064}:0", 123_456));
        assert_eq!(written.keychain, "external");
        assert_eq!(written.prev_tx_hex, "0200");
    }

    /// A mainnet file of one transaction with these outputs.
    fn on_mainnet(dir: &crate::testing::Temp, outputs: Vec<SignedOutput>) -> std::path::PathBuf {
        use plebline_core::batch::{SignedBatch, BATCH_SCHEMA_VERSION};

        let hex = crate::testing::spending_hex(&format!("{}:0", "11".repeat(32)));
        let tx: Transaction = deserialize_hex(&hex).expect("hex di prova");
        let batch = SignedBatch {
            version: BATCH_SCHEMA_VERSION,
            network: Network::Bitcoin.to_string(),
            created_by: "prova".into(),
            descriptor_checksum: "prova".into(),
            fee_rate_sat_vb: 1.0,
            txs: vec![SignedTx {
                index: 0,
                label: None,
                txid: tx.compute_txid().to_string(),
                hex,
                psbt: String::new(),
                inputs: Vec::new(),
                outputs,
                fee_sat: 500,
                fee_rate_sat_vb: 1.0,
                vsize: tx.vsize() as u64,
            }],
        };
        let path = dir.path.join("batch.json");
        std::fs::write(&path, serde_json::to_string(&batch).expect("json")).expect("scritto");
        path
    }

    fn output(value_sat: u64, is_change: bool) -> SignedOutput {
        SignedOutput {
            address: "bc1qprova".into(),
            value_sat,
            is_change,
        }
    }

    /// A refill on mainnet goes out only against its amount, typed: Enter
    /// or a wrong number sends nothing.
    #[test]
    fn on_mainnet_a_refill_asks_for_the_amount_typed() {
        let dir = crate::testing::Temp::new("mainnet-importo");
        let path = on_mainnet(&dir, vec![output(1_000, false)]);
        let mut chain = crate::chain::Fake::new();
        let mut ask = crate::ask::Scripted::new([crate::ask::Reply::text("1")]);

        let sent = broadcast(
            Network::Bitcoin,
            "http://server-finto",
            BroadcastInput {
                batch: path.clone(),
                index: 0,
            },
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert!(!sent, "l'importo scritto non torna: niente deve partire");
        assert!(chain.sent().is_empty(), "è partita senza l'importo");
        assert_eq!(ask.asked().len(), 1);
        assert_eq!(ask.asked()[0].kind, crate::ask::Kind::Text);

        // Grouped as the screen shows amounts: the help says it is fine.
        let mut ask = crate::ask::Scripted::new([crate::ask::Reply::text("1 000")]);
        let sent = broadcast(
            Network::Bitcoin,
            "http://server-finto",
            BroadcastInput {
                batch: path,
                index: 0,
            },
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");
        assert!(sent);
        assert_eq!(chain.sent().len(), 1);
    }

    /// Nothing leaves in a split but the fee: a yes or no, not an amount.
    #[test]
    fn on_mainnet_a_split_asks_yes_or_no() {
        let dir = crate::testing::Temp::new("mainnet-divisione");
        let path = on_mainnet(&dir, vec![output(40_000, true), output(40_000, true)]);
        let mut chain = crate::chain::Fake::new();
        let mut ask = crate::ask::Scripted::new([crate::ask::Reply::No]);

        let sent = broadcast(
            Network::Bitcoin,
            "http://server-finto",
            BroadcastInput {
                batch: path,
                index: 0,
            },
            &mut ask,
            &mut chain,
        )
        .expect("nessun errore");

        assert!(!sent, "il no lascia tutto com'era");
        assert!(chain.sent().is_empty());
        assert_eq!(ask.asked().len(), 1);
        assert_eq!(ask.asked()[0].kind, crate::ask::Kind::Confirm);
    }
}
