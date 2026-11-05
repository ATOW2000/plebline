//! Everything that talks to the blockchain server. [`Esplora`] makes the
//! real requests; `Fake` returns prepared answers, which lets tests run
//! `run()` from start to end.
//!
//! Only code that sends packets lives here. Filtering and counting what comes
//! back is in [`crate::online`], where tests reach it without a fake copy.
//!
//! The user picks the server during the ceremony, so every call takes it as
//! an argument instead of the constructor.

use std::collections::BTreeMap;

use bdk_esplora::esplora_client;
use bdk_esplora::EsploraAsyncExt;
use bdk_wallet::chain::ChainPosition;
use bdk_wallet::KeychainKind;
use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::{Amount, Transaction, Txid};
use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::utxos::keychain_name;
use plebline_core::watch::WatchFile;
use plebline_core::{bdk_wallet, Error, Result};

use crate::out;
use crate::tor;
use crate::ui;

/// What one scan asks for. Each option costs extra requests.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions {
    /// Consecutive unused addresses before stopping.
    pub stop_gap: usize,
    /// Source transactions of each coin: needed offline to prove value and
    /// script, one request per coin.
    pub origins: bool,
    /// Fresh addresses to reveal; needs the scan just done.
    pub reveal: u32,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Height of the last block seen by the server.
    pub tip: u32,
    /// Derived from the descriptors, not copied from the file.
    pub descriptor_checksum: String,
    /// Revealed indexes per keychain, so offline recognizes gap-limit addresses.
    pub last_active_indices: BTreeMap<String, u32>,
    pub funds: Funds,
    /// All coins, unfiltered (unconfirmed and coinbase included): the caller
    /// filters.
    pub coins: Vec<Coin>,
    /// The fresh addresses asked for with [`ScanOptions::reveal`].
    pub revealed: Vec<Address>,
}

/// `bdk_wallet::Balance`, unsummed: incoming and immature mean different
/// things to the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Funds {
    pub confirmed: u64,
    pub trusted_pending: u64,
    pub untrusted_pending: u64,
    /// Coinbase short of 100 confirmations.
    pub immature: u64,
}

impl Funds {
    pub fn total(self) -> u64 {
        self.confirmed
            .saturating_add(self.trusted_pending)
            .saturating_add(self.untrusted_pending)
            .saturating_add(self.immature)
    }
}

#[derive(Debug, Clone)]
pub struct Coin {
    /// `<txid>:<vout>`
    pub outpoint: String,
    pub value_sat: u64,
    pub script_pubkey: String,
    /// `external` | `internal`
    pub keychain: String,
    pub derivation_index: u32,
    /// `None`: still in the mempool; presigning on it is fragile.
    pub height: Option<u32>,
    /// Only with [`ScanOptions::origins`].
    pub origin: Option<Origin>,
}

#[derive(Debug, Clone)]
pub struct Origin {
    pub hex: String,
    /// Coinbase is unsupported offline: say so before the ceremony.
    pub coinbase: bool,
}

#[derive(Debug, Clone)]
pub struct Address {
    pub index: u32,
    pub text: String,
}

/// Status of a signed transaction on the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    /// In a block: the coin is really spent.
    Confirmed { height: u32 },
    /// Known, not in a block: can still be replaced or expire.
    InMempool,
    /// Never seen: a presigned payment waiting. Its coins look free on chain
    /// but must not be reused.
    NeverSent,
}

/// Requests to the server. Object-safe, like [`crate::ask::Ask`].
pub trait Chain {
    /// Reachability probe, before the seed is typed: fetches only the tip
    /// height (hands over no address), which also confirms the route shown
    /// on screen.
    fn reach(&mut self, api: &str) -> Result<Reachability>;

    /// Scans the chain for this wallet.
    fn scan(&mut self, api: &str, wallet: &WatchFile, options: ScanOptions) -> Result<ScanResult>;

    /// A network error stays an error, never "not confirmed": that would
    /// delete ceremony files when nothing is known.
    fn tx_status(&mut self, api: &str, txid: Txid) -> Result<TxStatus>;

    /// Broadcasts. Irreversible.
    fn send(&mut self, api: &str, tx: &Transaction) -> Result<()>;
}

/// Seconds a request may take. Without a limit, a hostile server can stall
/// the ceremony with the seed in memory; 120 s fits a long scan.
const TIMEOUT_SECS: u64 = 120;

/// On the direct route a name is resolved here, checked, and the client
/// connects only to those addresses: resolving again could give another.
fn pin_own_node(api: &str, builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder> {
    use std::net::ToSocketAddrs;
    let Ok(url) = url::Url::parse(api.trim()) else {
        return Ok(builder);
    };
    let (Some(url::Host::Domain(name)), Some(port)) = (url.host(), url.port_or_known_default())
    else {
        return Ok(builder);
    };
    let addresses: Vec<_> = (name, port)
        .to_socket_addrs()
        .map_err(|e| {
            Error::Network(tf!({ "non trovo l'indirizzo di {name}: {e}" }, {
                "cannot find the address of {name}: {e}"
            }))
        })?
        .collect();
    tor::check_resolved(name, &addresses)?;
    Ok(builder.resolve_to_addrs(name, &addresses))
}

/// The library's error in plain words. Its own text (`Reqwest(reqwest::Error
/// { kind: …`) only goes to a trace line, for whoever is helping the user.
fn network_error(e: esplora_client::Error) -> Error {
    technical_detail(&e);
    Error::Network(in_words(&e))
}

fn in_words(e: &esplora_client::Error) -> String {
    use esplora_client::Error as E;
    match e {
        E::Reqwest(inner) if inner.is_timeout() => tf!(
            { "il server non ha risposto in tempo: ho aspettato {TIMEOUT_SECS} secondi" },
            { "the server did not answer in time: I waited {TIMEOUT_SECS} seconds" }
        ),
        E::Reqwest(inner) if inner.is_connect() => t(
            "non sono riuscito a raggiungere il server",
            "I could not reach the server",
        )
        .to_string(),
        E::Reqwest(_) => t(
            "il collegamento con il server si è interrotto prima della risposta",
            "the connection to the server broke off before the answer",
        )
        .to_string(),
        E::HttpResponse { status: 429, .. } => t(
            "il server dice che gli arrivano troppe domande, e chiede di aspettare",
            "the server says it is getting too many questions, and asks to wait",
        )
        .to_string(),
        E::HttpResponse { status, .. } if *status >= 500 => tf!(
            { "il server ha un guasto suo (risponde con l'errore {status})" },
            { "the server has a fault of its own (it answers with error {status})" }
        ),
        E::HttpResponse { status, .. } => {
            tf!({ "il server ha rifiutato la domanda (errore {status})" }, {
                "the server refused the question (error {status})"
            })
        }
        E::TransactionNotFound(_) | E::HeaderHeightNotFound(_) | E::HeaderHashNotFound(_) => t(
            "il server non trova quello che gli ho chiesto",
            "the server cannot find what I asked for",
        )
        .to_string(),
        _ => t(
            "il server ha risposto, ma con dati che non si leggono",
            "the server answered, but with data that cannot be read",
        )
        .to_string(),
    }
}

/// The innermost cause (e.g. `connection closed before message completed`),
/// short, as a trace line.
fn technical_detail(e: &esplora_client::Error) {
    let detail = match e {
        esplora_client::Error::Reqwest(inner) => {
            let mut cause: &dyn std::error::Error = inner;
            while let Some(deeper) = cause.source() {
                cause = deeper;
            }
            cause.to_string()
        }
        other => other.to_string(),
    };
    let detail: String = detail.chars().take(200).collect();
    ui::trace(&tf!({ "dettaglio tecnico: {detail}" }, {
        "technical detail: {detail}"
    }));
}

/// Each client gets its own Tor circuit: Tor keeps apart streams with
/// different SOCKS credentials (`IsolateSOCKSAuth`, on by default), so a
/// retry does not ride the circuit that just stalled. Credentials already in
/// the proxy are the user's and stay.
fn isolated(proxy: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let Some((scheme, rest)) = proxy.split_once("://") else {
        return proxy.to_string();
    };
    if rest.contains('@') {
        return proxy.to_string();
    }
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let process = std::process::id();
    format!("{scheme}://plebline-{process}-{n}:x@{rest}")
}

/// The route the Tor rule allowed for one server. Only [`Esplora::gate`]
/// makes one and [`Esplora::client`] needs one, so no request can leave
/// without the rule.
struct Permit {
    route: tor::Route,
}

impl std::ops::Deref for Permit {
    type Target = tor::Route;

    fn deref(&self) -> &tor::Route {
        &self.route
    }
}

/// Result of [`Chain::reach`]: the route and the height, both shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reachability {
    pub route: String,
    pub height: u32,
    pub proof: RouteProof,
}

/// Evidence for the route taken.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RouteProof {
    /// Onion address: reachable only through Tor, no exit node. The default,
    /// as the suggested servers are onion addresses.
    #[default]
    Onion,
    /// Clearnet server through Tor, confirmed by `check.torproject.org` via the
    /// same proxy.
    Exit { ip: String },
    /// Tor check did not answer. Not fatal (a third party being down proves
    /// nothing), but shown.
    Unconfirmed { why: String },
    /// Own node: Tor is not involved.
    OwnNode,
}

/// The real server: Esplora over HTTPS, through Tor unless it is the user's
/// own node.
///
/// The proxy is a property of the machine, so it is a field; the server
/// is not. Async inside (only the async client speaks SOCKS5), blocking
/// outside.
#[derive(Debug, Clone, Default)]
pub struct Esplora {
    /// `None`: no proxy, which only reaches an own node.
    proxy: Option<String>,
}

impl Esplora {
    pub fn new() -> Self {
        Self {
            proxy: Some(tor::DEFAULT_PROXY.to_string()),
        }
    }

    /// `None` does not disable protection: [`crate::tor`] then refuses every
    /// server that is not yours.
    pub fn through(proxy: Option<String>) -> Self {
        Self { proxy }
    }

    /// Applies the Tor rule before any request is prepared, and shows the
    /// route on screen.
    fn allowed(&self, api: &str) -> Result<Permit> {
        let permit = self.gate(api)?;
        ui::info(permit.description());
        Ok(permit)
    }

    /// [`Esplora::allowed`] without the screen line, for `reach`. The only
    /// place that applies the rule, and the only maker of a [`Permit`].
    fn gate(&self, api: &str) -> Result<Permit> {
        tor::route(api, &tor::Env::read(self.proxy.as_deref())).map(|route| Permit { route })
    }

    /// Built here rather than by `esplora_client::Builder`, which cannot turn
    /// off environment proxies and redirects: either would take the request
    /// to a host the rule never saw. Esplora never redirects.
    fn client(api: &str, route: &Permit) -> Result<esplora_client::AsyncClient> {
        let reqwest_error = |e| network_error(esplora_client::Error::Reqwest(e));
        let builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none());
        let builder = match route.proxy() {
            Some(proxy) => {
                builder.proxy(reqwest::Proxy::all(isolated(proxy)).map_err(reqwest_error)?)
            }
            None => pin_own_node(api, builder.no_proxy())?,
        };
        let client = builder.build().map_err(reqwest_error)?;
        Ok(esplora_client::AsyncClient::from_client(
            api.to_string(),
            client,
        ))
    }

    fn runtime() -> Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                Error::Network(tf!({ "non riesco ad avviare il motore di rete: {e}" }, {
                    "cannot start the network engine: {e}"
                }))
            })
    }

    /// Queried through the proxy, so it describes the same route.
    const CHECK: &'static str = "https://check.torproject.org/api/ip";

    /// Evidence for the route: own node or onion need nothing; a clearnet
    /// server through Tor asks Tor itself. A "not Tor" answer stops everything
    /// (the user's IP would leak); no answer is shown and the ceremony goes on.
    async fn prove(
        client: &esplora_client::AsyncClient,
        api: &str,
        route: &tor::Route,
    ) -> Result<RouteProof> {
        if matches!(route, tor::Route::OwnNode) {
            return Ok(RouteProof::OwnNode);
        }
        if tor::is_onion(api) {
            ui::trace(t(
                "è un indirizzo .onion: fuori da Tor non esiste",
                "it is an onion address: outside Tor it does not exist",
            ));
            return Ok(RouteProof::Onion);
        }

        let proxy = route.proxy().unwrap_or_default().to_string();
        let check = Self::CHECK;
        ui::trace(&tf!(
            { "non è un .onion: chiedo a {check} se sono dentro Tor" },
            { "not an onion address: asking {check} whether I am inside Tor" }
        ));

        // Same client as for the server, so the same proxy and route.
        let proof = match Self::body_of_check(client).await {
            Ok(body) => Self::read_check(&body, &proxy)?,
            Err(why) => RouteProof::Unconfirmed { why },
        };

        // `read_check` stays pure for tests; reporting happens here.
        match &proof {
            RouteProof::Exit { ip } => {
                ui::trace(&tf!({ "Tor conferma: sono uscito dal nodo {ip}" }, {
                    "Tor confirms it: I came out of node {ip}"
                }))
            }
            RouteProof::Unconfirmed { why } => {
                ui::trace(&tf!({ "Tor non ha confermato: {why}" }, {
                    "Tor did not confirm: {why}"
                }))
            }
            RouteProof::Onion | RouteProof::OwnNode => {}
        }
        Ok(proof)
    }

    /// A failure comes back as text for [`RouteProof::Unconfirmed`], so it does
    /// not stop the ceremony.
    async fn body_of_check(
        client: &esplora_client::AsyncClient,
    ) -> std::result::Result<String, String> {
        client
            .client()
            .get(Self::CHECK)
            .send()
            .await
            .map_err(|why| why.to_string())?
            .text()
            .await
            .map_err(|why| why.to_string())
    }

    /// Interprets Tor's answer. Pure, for tests.
    fn read_check(body: &str, proxy: &str) -> Result<RouteProof> {
        let check = Self::CHECK;
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
            return Ok(RouteProof::Unconfirmed {
                why: tf!({ "risposta illeggibile da {check}" }, {
                    "unreadable answer from {check}"
                }),
            });
        };

        match parsed.get("IsTor").and_then(serde_json::Value::as_bool) {
            Some(true) => Ok(RouteProof::Exit {
                ip: parsed
                    .get("IP")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
            }),
            // The proxy is not Tor and the user's IP would leak: stop.
            Some(false) => Err(Error::NotTor {
                proxy: proxy.to_string(),
            }),
            None => Ok(RouteProof::Unconfirmed {
                why: tf!({ "risposta senza IsTor da {check}" }, {
                    "answer with no IsTor from {check}"
                }),
            }),
        }
    }

    /// From the wallet when already scanned, else from the server.
    async fn full_transaction(
        wallet: &bdk_wallet::Wallet,
        client: &esplora_client::AsyncClient,
        txid: Txid,
    ) -> Result<Transaction> {
        if let Some(tx) = wallet.get_tx(txid) {
            return Ok(tx.tx_node.tx.as_ref().clone());
        }
        client
            .get_tx(&txid)
            .await
            .map_err(network_error)?
            .ok_or_else(|| {
                Error::Network(tf!(
                    { "la transazione {txid} non esiste sul server interrogato" },
                    { "transaction {txid} does not exist on the server queried" }
                ))
            })
    }
}

impl Chain for Esplora {
    fn reach(&mut self, api: &str) -> Result<Reachability> {
        // Printed before `gate`, which connects to the proxy and may refuse;
        // that step must still appear in the trace.
        if let Some(proxy) = self.proxy.as_deref() {
            ui::trace(&tf!({ "provo il proxy Tor su {proxy}" }, {
                "trying the Tor proxy at {proxy}"
            }));
        }

        // The rule applies even to the tip height. Here the route is a trace
        // line.
        let route = self.gate(api)?;
        ui::trace(match route.proxy() {
            Some(_) => t(
                "risponde: la richiesta esce da Tor",
                "it answers: the request leaves through Tor",
            ),
            None => t(
                "server sulla tua rete: Tor non serve e non c'entra",
                "server on your own network: Tor is not needed and not involved",
            ),
        });
        ui::trace(&tf!({ "chiedo a {api}" }, { "asking {api}" }));
        let client = Self::client(api, &route)?;
        let runtime = Self::runtime()?;

        // Check Tor before asking anything, so a proxy that is not Tor is
        // found out with nothing sent.
        let proof = runtime.block_on(Self::prove(&client, api, &route))?;

        ui::trace(t("chiedo l'ultimo blocco", "asking for the last block"));
        let height = runtime
            .block_on(client.get_height())
            .map_err(network_error)?;
        ui::trace(&tf!({ "risponde: blocco {height}" }, {
            "it answers: block {height}"
        }));
        Ok(Reachability {
            route: route.description().to_string(),
            height,
            proof,
        })
    }

    fn scan(&mut self, api: &str, file: &WatchFile, options: ScanOptions) -> Result<ScanResult> {
        // Rule first: this request hands the whole wallet to the server.
        let route = self.allowed(api)?;

        // Already checked against the expected network on reading.
        let network = file.parsed_network()?;
        let mut wallet =
            plebline_core::wallet::from_descriptors(&file.external, &file.internal, network)?;

        ui::info(&tf!({ "Interrogo {api}" }, { "Querying {api}" }));
        let client = Self::client(api, &route)?;
        let runtime = Self::runtime()?;

        // Show progress, or a stuck-looking program gets killed.
        let scanned = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = scanned.clone();

        let request = wallet
            .start_full_scan()
            .inspect(move |_keychain, index, _spk| {
                let seen = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                out::progress(Some(&format!(
                    "{} {seen}, {} {index}",
                    t("indirizzi controllati:", "addresses checked:"),
                    t("indice", "index"),
                )));
            });

        let update = runtime
            .block_on(client.full_scan(request, options.stop_gap, route.parallel_requests()))
            .map_err(|e| network_error(*e))?;
        let total = scanned.load(std::sync::atomic::Ordering::Relaxed);
        out::progress(None);
        ui::success(&tf!({ "{total} indirizzi controllati." }, {
            "{total} addresses checked."
        }));
        wallet.apply_update(update).map_err(|e| {
            Error::Network(tf!(
                { "non riesco ad applicare il risultato della scansione: {e}" },
                { "cannot apply the scan result: {e}" }
            ))
        })?;

        // A lying server can claim outputs worth more than all the bitcoin
        // there is, and BDK's balance would panic adding them up.
        let mut held = Amount::ZERO;
        for utxo in wallet.list_unspent() {
            held = held
                .checked_add(utxo.txout.value)
                .filter(|sum| *sum <= Amount::MAX_MONEY)
                .ok_or_else(|| {
                    Error::Network(t(
                        "il server dice che il portafoglio ha più bitcoin di quanti ne esistano: \
                         non è una risposta vera",
                        "the server says the wallet holds more bitcoin than exist: that is not \
                         a true answer",
                    ).into())
                })?;
        }
        let tip = wallet.local_chain().tip().height();
        let balance = wallet.balance();

        let mut coins = Vec::new();
        for utxo in wallet.list_unspent() {
            let height = match utxo.chain_position {
                ChainPosition::Confirmed { ref anchor, .. } => Some(anchor.block_id.height),
                ChainPosition::Unconfirmed { .. } => None,
            };
            let origin = if options.origins {
                let prev = runtime.block_on(Self::full_transaction(
                    &wallet,
                    &client,
                    utxo.outpoint.txid,
                ))?;
                Some(Origin {
                    hex: serialize_hex(&prev),
                    coinbase: prev.is_coinbase(),
                })
            } else {
                None
            };
            coins.push(Coin {
                outpoint: utxo.outpoint.to_string(),
                value_sat: utxo.txout.value.to_sat(),
                script_pubkey: utxo.txout.script_pubkey.to_hex_string(),
                keychain: keychain_name(utxo.keychain).to_string(),
                derivation_index: utxo.derivation_index,
                height,
                origin,
            });
        }

        // Beyond the used ones, for gap-limit addresses.
        let mut last_active_indices = BTreeMap::new();
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            let revealed = wallet.derivation_index(keychain).unwrap_or(0);
            last_active_indices.insert(keychain_name(keychain).to_string(), revealed);
        }

        let mut generated = Vec::new();
        for _ in 0..options.reveal {
            let info = wallet.reveal_next_address(KeychainKind::External);
            generated.push(Address {
                index: info.index,
                text: info.address.to_string(),
            });
        }

        Ok(ScanResult {
            tip,
            descriptor_checksum: wallet.descriptor_checksum(KeychainKind::External),
            last_active_indices,
            funds: Funds {
                confirmed: balance.confirmed.to_sat(),
                trusted_pending: balance.trusted_pending.to_sat(),
                untrusted_pending: balance.untrusted_pending.to_sat(),
                immature: balance.immature.to_sat(),
            },
            coins,
            revealed: generated,
        })
    }

    fn tx_status(&mut self, api: &str, txid: Txid) -> Result<TxStatus> {
        let route = self.allowed(api)?;
        let client = Self::client(api, &route)?;
        let runtime = Self::runtime()?;

        // `None`: unknown to the server. Normal for a presigned payment.
        let status = match runtime
            .block_on(client.get_tx_info(&txid))
            .map_err(network_error)?
        {
            Some(info) if info.status.confirmed => TxStatus::Confirmed {
                height: info.status.block_height.unwrap_or(0),
            },
            Some(_) => TxStatus::InMempool,
            None => TxStatus::NeverSent,
        };
        Ok(status)
    }

    fn send(&mut self, api: &str, tx: &Transaction) -> Result<()> {
        // Most revealing packet: this transaction, from this address.
        let route = self.allowed(api)?;
        let client = Self::client(api, &route)?;
        let runtime = Self::runtime()?;

        runtime.block_on(client.broadcast(tx)).map_err(|e| {
            technical_detail(&e);
            // The node's own reason (e.g. `min relay fee not met`) is the answer.
            let why = match &e {
                esplora_client::Error::HttpResponse { message, .. } => message.clone(),
                other => in_words(other),
            };
            Error::Broadcast(tf!({ "trasmissione rifiutata: {why}" }, {
                "broadcast rejected: {why}"
            }))
        })
    }
}

/// The fake chain and hand-built snapshots, for tests.
#[cfg(any(test, feature = "test-support"))]
mod fake {
    use super::*;

    /// Hand-built snapshots for tests.
    impl ScanResult {
        /// Wallet exists, chain shows nothing (e.g. an unconfirmed split).
        pub fn empty(tip: u32, descriptor_checksum: impl Into<String>) -> Self {
            Self {
                tip,
                descriptor_checksum: descriptor_checksum.into(),
                last_active_indices: BTreeMap::new(),
                funds: Funds::default(),
                coins: Vec::new(),
                revealed: Vec::new(),
            }
        }

        /// Adds a coin and updates the balance to match.
        pub fn with(mut self, coin: Coin) -> Self {
            if coin.height.is_some() {
                self.funds.confirmed += coin.value_sat;
            } else {
                self.funds.trusted_pending += coin.value_sat;
            }
            let known = self
                .last_active_indices
                .entry(coin.keychain.clone())
                .or_insert(0);
            *known = (*known).max(coin.derivation_index);
            self.coins.push(coin);
            self
        }
    }

    enum Answer {
        Shows(Box<ScanResult>),
        Down(String),
    }

    /// What a real failure looks like to the flow: [`Error::Network`].
    fn fake_down(reason: &str) -> Error {
        Error::Network(tf!({ "la rete non risponde: {reason}" }, {
            "the network is not answering: {reason}"
        }))
    }

    /// Prepared chain for testing `run()` end to end. Records which servers
    /// were queried and what was broadcast.
    #[derive(Default)]
    pub struct Fake {
        answers: std::collections::VecDeque<Answer>,
        statuses: BTreeMap<String, TxStatus>,
        failing: BTreeMap<String, String>,
        refused: BTreeMap<String, String>,
        servers: Vec<String>,
        sent: Vec<String>,
        probes: Vec<String>,
        probe_failure: Option<String>,
        down: BTreeMap<String, String>,
        height: u32,
        proof: RouteProof,
    }

    impl Fake {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_scan(mut self, scanned: ScanResult) -> Self {
            self.answers.push_back(Answer::Shows(Box::new(scanned)));
            self
        }

        pub fn scan_fails(mut self, reason: impl Into<String>) -> Self {
            self.answers.push_back(Answer::Down(reason.into()));
            self
        }

        /// Status of a transaction. Asking an unprepared txid fails the test.
        pub fn with_status(mut self, txid: impl Into<String>, status: TxStatus) -> Self {
            self.statuses.insert(txid.into(), status);
            self
        }

        /// Network failure for this txid: must never read as "not confirmed".
        pub fn failing_status(
            mut self,
            txid: impl Into<String>,
            reason: impl Into<String>,
        ) -> Self {
            self.failing.insert(txid.into(), reason.into());
            self
        }

        /// A broadcast the server refuses.
        pub fn refusing_broadcast(
            mut self,
            txid: impl Into<String>,
            reason: impl Into<String>,
        ) -> Self {
            self.refused.insert(txid.into(), reason.into());
            self
        }

        /// The pre-seed probe fails; distinct from [`Fake::scan_fails`].
        pub fn probe_fails(mut self, reason: impl Into<String>) -> Self {
            self.probe_failure = Some(reason.into());
            self
        }

        /// The pre-seed probe fails for this server only.
        pub fn probe_fails_on(mut self, api: impl Into<String>, reason: impl Into<String>) -> Self {
            self.down.insert(api.into(), reason.into());
            self
        }

        /// Default [`RouteProof::Onion`].
        pub fn with_proof(mut self, proof: RouteProof) -> Self {
            self.proof = proof;
            self
        }

        pub fn with_height(mut self, height: u32) -> Self {
            self.height = height;
            self
        }

        /// The servers queried, in order and with repetitions.
        pub fn servers(&self) -> &[String] {
            &self.servers
        }

        /// Probes only, not counted in [`Fake::servers`]: a probe reveals nothing.
        pub fn probes(&self) -> &[String] {
            &self.probes
        }

        /// The txids that went out on the network, in order.
        pub fn sent(&self) -> &[String] {
            &self.sent
        }

        /// Zero at the end proves the path taken.
        pub fn remaining(&self) -> usize {
            self.answers.len()
        }
    }

    impl Chain for Fake {
        fn reach(&mut self, api: &str) -> Result<Reachability> {
            self.probes.push(api.to_string());
            match self.probe_failure.as_ref().or_else(|| self.down.get(api)) {
                Some(reason) => Err(fake_down(reason)),
                None => Ok(Reachability {
                    route: t("La rete esce da Tor.", "Traffic leaves through Tor.").to_string(),
                    height: self.height,
                    proof: self.proof.clone(),
                }),
            }
        }

        fn scan(
            &mut self,
            api: &str,
            _wallet: &WatchFile,
            _look: ScanOptions,
        ) -> Result<ScanResult> {
            self.servers.push(api.to_string());
            match self.answers.pop_front() {
                Some(Answer::Shows(scanned)) => Ok(*scanned),
                Some(Answer::Down(reason)) => Err(fake_down(&reason)),
                None => {
                    panic!("il flusso guarda la catena, ma non c'è nessuna fotografia preparata")
                }
            }
        }

        fn tx_status(&mut self, api: &str, txid: Txid) -> Result<TxStatus> {
            self.servers.push(api.to_string());
            let key = txid.to_string();
            if let Some(reason) = self.failing.get(&key) {
                return Err(fake_down(reason));
            }
            match self.statuses.get(&key) {
                Some(status) => Ok(*status),
                None => panic!(
                    "il flusso chiede dov'è {key}, e la catena finta non lo sa: \
                     preparalo con .with_status(…), anche come NeverSent"
                ),
            }
        }

        fn send(&mut self, api: &str, tx: &Transaction) -> Result<()> {
            self.servers.push(api.to_string());
            let txid = tx.compute_txid().to_string();
            if let Some(reason) = self.refused.get(&txid) {
                return Err(Error::Broadcast(tf!(
                    { "trasmissione rifiutata: {reason}" },
                    { "broadcast rejected: {reason}" }
                )));
            }
            self.sent.push(txid);
            Ok(())
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use fake::Fake;

#[cfg(test)]
mod tests {
    use super::*;

    fn coin(value: u64, mined_at: Option<u32>) -> Coin {
        Coin {
            outpoint: format!("{value:064}:0"),
            value_sat: value,
            script_pubkey: "0014".repeat(6),
            keychain: "external".into(),
            derivation_index: 0,
            height: mined_at,
            origin: None,
        }
    }

    /// No network involved: the proxy is a closed local port.
    #[test]
    fn with_tor_down_the_step_that_failed_is_still_said() {
        let notebook = crate::out::Recorder::new();
        crate::out::to(Box::new(notebook.clone()));

        // Port 1: refused at once, no timeout.
        let mut chain = Esplora::through(Some("socks5h://127.0.0.1:1".to_string()));
        let outcome = chain.reach("http://qualcosa.onion/api");

        crate::out::to(Box::new(crate::out::Terminal));

        assert!(outcome.is_err(), "la regola deve rifiutare: {outcome:?}");
        let traces: Vec<String> = notebook
            .lines()
            .into_iter()
            .filter(|line| line.kind == crate::out::Kind::Trace)
            .map(|line| line.text)
            .collect();
        assert!(
            !traces.is_empty(),
            "anche fallendo subito, il passo che è stato fatto va detto"
        );
        assert!(
            traces[0].contains("127.0.0.1:1"),
            "e dice quale porta ha provato: {traces:?}"
        );
    }

    #[test]
    fn tor_confirming_gives_the_exit_node() {
        let answer = r#"{"IsTor":true,"IP":"185.220.101.1"}"#;
        assert_eq!(
            Esplora::read_check(answer, "socks5h://127.0.0.1:9050").expect("va bene"),
            RouteProof::Exit {
                ip: "185.220.101.1".to_string()
            }
        );
    }

    /// The only answer that stops everything.
    #[test]
    fn tor_saying_no_stops_the_ceremony() {
        let answer = r#"{"IsTor":false,"IP":"93.42.10.7"}"#;
        let error = Esplora::read_check(answer, "socks5h://127.0.0.1:9050")
            .expect_err("un proxy che non è Tor non è un dettaglio");
        assert!(
            matches!(error, Error::NotTor { .. }),
            "serve la variante sua, con il suo rimedio: {error}"
        );
        // Remedy about the proxy, not Tor installation.
        let remedy = error.remedy().unwrap_or_default();
        assert!(remedy.contains("proxy"), "{remedy}");
    }

    #[test]
    fn an_unusable_answer_is_neither_a_yes_nor_a_no() {
        for answer in [
            "<html>we are down</html>",
            r#"{"IP":"185.220.101.1"}"#,
            r#"{"IsTor":"forse"}"#,
        ] {
            assert!(
                matches!(
                    Esplora::read_check(answer, "socks5h://127.0.0.1:9050"),
                    Ok(RouteProof::Unconfirmed { .. })
                ),
                "«{answer}» non è né un sì né un no"
            );
        }
    }

    #[test]
    fn the_snapshots_balance_follows_the_coins() {
        let scanned = ScanResult::empty(800_000, "abcdefgh")
            .with(coin(100_000, Some(799_000)))
            .with(coin(50_000, None));
        assert_eq!(scanned.funds.confirmed, 100_000);
        assert_eq!(scanned.funds.trusted_pending, 50_000);
        assert_eq!(scanned.funds.total(), 150_000);
        assert_eq!(scanned.coins.len(), 2);
    }

    #[test]
    #[should_panic(expected = "nessuna fotografia preparata")]
    fn looking_without_having_prepared_anything_stops_the_test() {
        let mut chain = Fake::new();
        let _ = chain.scan("http://esempio", &fake_wallet(), ScanOptions::default());
    }

    #[test]
    #[should_panic(expected = "la catena finta non lo sa")]
    fn an_unprepared_txid_stops_the_test() {
        let mut chain = Fake::new();
        let txid: Txid = "0".repeat(64).parse().expect("txid valido");
        let _ = chain.tx_status("http://esempio", txid);
    }

    fn fake_wallet() -> WatchFile {
        WatchFile {
            version: plebline_core::watch::WATCH_SCHEMA_VERSION,
            network: "bitcoin".into(),
            external: "wpkh(xpub)".into(),
            internal: "wpkh(xpub)".into(),
            descriptor_checksum: "abcdefgh".into(),
            created_by: "test".into(),
        }
    }

    /// The network client is named in this file only. Searches identifiers,
    /// not the word "esplora" (an Italian verb too).
    #[test]
    fn all_the_network_goes_through_here() {
        let culprits = crate::testing::files_naming(&["bdk_esplora", "esplora_client"], "chain.rs");
        assert!(
            culprits.is_empty(),
            "queste interrogazioni non passano dal connettore: {culprits:?}"
        );
        // The search does read the files: it finds the client here.
        assert!(crate::testing::code_of("chain.rs").contains("esplora_client"));
    }
}
