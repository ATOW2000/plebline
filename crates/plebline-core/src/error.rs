//! Errors, each with a remedy that can be printed as is.
//!
//! `Display` and `remedy` are written by hand instead of with thiserror
//! because the text depends on the language chosen at startup.

use bdk_wallet::bitcoin::OutPoint;
use std::fmt;

use crate::batch::number;
use crate::lang::t;
use crate::tf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io {
        path: String,
        source: std::io::Error,
    },

    Json {
        path: String,
        source: serde_json::Error,
    },

    /// Serialization failed; nothing was written. An error instead of an
    /// `expect()` because after signing a panic (with `panic = "abort"`)
    /// would lose the ceremony.
    Serialize {
        path: String,
        source: serde_json::Error,
    },

    SchemaVersion {
        path: String,
        found: u32,
        expected: u32,
    },

    NetworkMismatch {
        found: String,
        expected: String,
    },

    InvalidUtxo {
        outpoint: String,
        reason: String,
    },

    Descriptor(String),

    Seed(String),

    BuildTx {
        index: usize,
        reason: String,
    },

    Signing(String),

    NotFinalized {
        index: usize,
        missing: usize,
    },

    InvariantViolated(DisjointnessError),

    /// What was typed or asked for cannot be used (an address, an amount, a
    /// fee rate, a split): nothing was written.
    Input(String),

    /// A signed file, or one about to be written, fails a check: what it
    /// says is not what it does.
    Verification(String),

    /// Stopped before doing anything, for a reason the message gives
    /// together with what to do: it has no separate remedy.
    Refused(String),

    /// The server refused a broadcast; the message carries the node's reason.
    Broadcast(String),

    /// Dialogue interrupted (Esc, Ctrl-C, no terminal). It has its own
    /// remedy, which says the files on disk are intact; the seed and the
    /// batch are not in question.
    Interaction(String),

    /// The user cancelled a question. Kept apart from `Interaction` so the
    /// flow can take it as "go back" instead of a failure.
    Cancelled,

    /// Refused before sending: querying Esplora without Tor links every wallet
    /// address to our IP.
    Clearnet {
        host: String,
    },

    /// Tor configured but not answering.
    TorDown {
        host: String,
        proxy: String,
    },

    /// The proxy answers but is not Tor (checked via `check.torproject.org`).
    NotTor {
        proxy: String,
    },

    /// Server unreachable or unusable response. Its remedy tells the user
    /// that the files written so far are still valid.
    Network(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::Io { path, source } => format!("{path}: {source}"),
            Error::Json { path, source } => tf!({ "{path} non è un JSON valido: {source}" }, {
                "{path} is not valid JSON: {source}"
            }),
            Error::Serialize { path, source } => tf!(
                { "non riesco a mettere in JSON quello che doveva finire in {path}: {source}" },
                { "cannot turn into JSON what was headed for {path}: {source}" }
            ),
            Error::SchemaVersion {
                path,
                found,
                expected,
            } => tf!(
                { "schema di {path} non supportato: versione {found}, attesa {expected}" },
                { "unsupported schema in {path}: found version {found}, expected {expected}" }
            ),
            Error::NetworkMismatch { found, expected } => tf!(
                { "rete incoerente: il file dichiara {found}, la sessione usa {expected}" },
                { "network mismatch: the file declares {found}, this session uses {expected}" }
            ),
            Error::InvalidUtxo { outpoint, reason } => format!("utxo {outpoint}: {reason}"),
            Error::Descriptor(detail) => tf!({ "descriptor non valido: {detail}" }, {
                "invalid descriptor: {detail}"
            }),
            Error::Seed(detail) => tf!({ "seed non valido: {detail}" }, {
                "invalid seed: {detail}"
            }),
            Error::BuildTx { index, reason } => {
                let index = number(*index);
                tf!(
                    { "costruzione della transazione #{index} fallita: {reason}" },
                    { "building transaction #{index} failed: {reason}" }
                )
            }
            Error::Signing(detail) => tf!({ "firma fallita: {detail}" }, {
                "signing failed: {detail}"
            }),
            Error::NotFinalized { index, missing } => {
                let index = number(*index);
                tf!(
                    { "transazione #{index} non finalizzata: mancano firme su {missing} input" },
                    {
                        "transaction #{index} is not finalized: {missing} inputs are missing \
                         signatures"
                    }
                )
            }
            Error::InvariantViolated(inner) => tf!({ "INVARIANTE VIOLATA: {inner}" }, {
                "INVARIANT VIOLATED: {inner}"
            }),
            Error::Input(detail)
            | Error::Verification(detail)
            | Error::Refused(detail)
            | Error::Broadcast(detail)
            | Error::Interaction(detail)
            | Error::Network(detail) => detail.clone(),
            Error::Cancelled => t("annullato", "cancelled").to_string(),
            Error::TorDown { host, proxy } => tf!(
                {
                    "non interrogo {host}: Tor non risponde su {proxy}, e senza Tor quel \
                  server imparerebbe ogni indirizzo di questo portafoglio insieme al tuo \
                  indirizzo IP"
                },
                {
                    "not querying {host}: Tor is not answering on {proxy}, and without Tor that \
                  server would learn every address of this wallet along with your IP address"
                }
            ),
            Error::NotTor { proxy } => tf!(
                {
                    "il proxy {proxy} risponde, ma quello che c'è dietro non è Tor: \
                  l'ho chiesto a check.torproject.org passando di lì, e la risposta è no"
                },
                {
                    "the proxy {proxy} answers, but what is behind it is not Tor: I asked \
                  check.torproject.org through it, and the answer is no"
                }
            ),
            Error::Clearnet { host } => tf!(
                {
                    "non interrogo {host} senza Tor: quel server imparerebbe ogni indirizzo \
                  di questo portafoglio, e da quale indirizzo IP glielo si è chiesto"
                },
                {
                    "not querying {host} without Tor: that server would learn every address of \
                  this wallet, and the IP address it was asked from"
                }
            ),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Json { source, .. } => Some(source),
            Error::Serialize { source, .. } => Some(source),
            Error::InvariantViolated(source) => Some(source),
            _ => None,
        }
    }
}

impl From<DisjointnessError> for Error {
    fn from(error: DisjointnessError) -> Self {
        Error::InvariantViolated(error)
    }
}

impl Error {
    /// For `map_err` on a file operation: `.map_err(Error::io(path))`.
    pub fn io(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
        move |source| Error::Io {
            path: path.display().to_string(),
            source,
        }
    }

    /// Suggested remedy, to print under the error message. Written for the
    /// window; a program with other options passes its own to
    /// `plebline::report_with`.
    pub fn remedy(&self) -> Option<&'static str> {
        let remedy = match self {
            Error::Io { .. } => t(
                "Controlla che il percorso sia giusto, e che il disco o la chiavetta siano \
                 montati, scrivibili e non pieni.",
                "Check that the path is right, and that the disk or stick is mounted, writable \
                 and not full.",
            ),
            Error::Json { .. } => t(
                "Il file non si legge: non modificarlo a mano. Rifallo con il programma, dal \
                 passo che lo crea.",
                "The file cannot be read: do not edit it by hand. Make it again with the \
                 program, from the step that creates it.",
            ),
            Error::Serialize { .. } => t(
                "Non è stato scritto niente e i file già sul disco sono intatti: non c'è \
                 nulla da trasmettere e nulla da recuperare. Riprova. Se succede di nuovo \
                 segnalalo: è un bug, non un errore di configurazione.",
                "Nothing was written and the files already on disk are intact: there is \
                 nothing to broadcast and nothing to recover. Try again. If it happens twice, \
                 report it: this is a bug, not a configuration mistake.",
            ),
            Error::SchemaVersion { .. } => t(
                "Il file è stato scritto da un'altra versione di PlebLine: usa la stessa \
                 versione per tutta la cerimonia.",
                "The file was written by another version of PlebLine: use one version for the \
                 whole ceremony.",
            ),
            Error::NetworkMismatch { .. } => t(
                "Il file è di un'altra rete: usa il PlebLine fatto per quella rete, oppure \
                 rifai il file su questa.",
                "The file belongs to another network: use the PlebLine built for that network, \
                 or make the file again on this one.",
            ),
            Error::InvalidUtxo { .. } => t(
                "L'elenco delle monete è incoerente o corrotto: rileggi la catena per rifarlo. \
                 Non fidarti di un file che non supera questo controllo.",
                "The list of coins is inconsistent or corrupted: read the chain again to make it \
                 anew. Do not trust a file that fails this check.",
            ),
            Error::Descriptor(_) => t(
                "Il codice di sola lettura del wallet non è valido: deve finire con il checksum \
                 dopo '#'. Non modificarlo a mano.",
                "The wallet's read-only code is not valid: it must end with the checksum after \
                 '#'. Do not edit it by hand.",
            ),
            Error::Seed(_) => t(
                "Reinserisci le parole del seed. Devono essere 12, 15, 18, 21 o 24, separate da \
                 uno spazio, tutte dall'elenco BIP39.",
                "Enter the seed words again. They must be 12, 15, 18, 21 or 24, separated by \
                 single spaces, all from the BIP39 word list.",
            ),
            Error::BuildTx { .. } => t(
                "Se le monete non bastano, prepara meno ricariche oppure spezza prima le monete \
                 in parti più piccole: ogni ricarica deve spendere monete sue.",
                "If there are not enough coins, prepare fewer refills or first break up the coins \
                 into smaller ones: each refill must spend coins of its own.",
            ),
            Error::Signing(_) => t(
                "Il seed inserito probabilmente non corrisponde al descriptor del wallet. \
                 Controlla la passphrase.",
                "The seed you entered probably does not match the wallet descriptor. \
                 Check the passphrase.",
            ),
            Error::NotFinalized { .. } => t(
                "Il wallet non ha potuto firmare tutti gli input: seed o passphrase errati.",
                "The wallet could not sign every input: wrong seed or passphrase.",
            ),
            Error::InvariantViolated(_) => t(
                "NON trasmettere nulla di questo batch. Rieseguilo da capo e, se si ripete, \
                 segnala il problema: è un bug, non un errore di configurazione.",
                "Do NOT broadcast anything from this batch. Run it again from scratch and, if it \
                 happens again, report it: this is a bug, not a configuration mistake.",
            ),
            Error::Interaction(_) | Error::Cancelled => t(
                "Niente è andato perduto: i file già scritti restano dove sono. Riapri il \
                 programma per riprendere da lì.",
                "Nothing was lost: the files already written stay where they are. Open the \
                 program again to pick up from there.",
            ),
            Error::Input(_) => t(
                "Correggi quel dato e riprova: non è stato scritto niente.",
                "Correct that and try again: nothing was written.",
            ),
            Error::Verification(_) => t(
                "Il batch non supera la verifica: non trasmetterlo.",
                "The batch fails verification: do not broadcast it.",
            ),
            Error::Refused(_) => return None,
            Error::Broadcast(_) => t(
                "La ricarica non è partita e le sue monete restano dove sono. Il motivo è quello \
                 del nodo: se dice che la tariffa è troppo bassa, riprova quando la rete è meno \
                 affollata.",
                "The refill did not leave and its coins stay where they are. The reason is the \
                 node's own: if it says the fee is too low, try again when the network is less \
                 busy.",
            ),
            Error::Network(_) => t(
                "Non è un problema delle tue monete: i file scritti finora restano validi. \
                 Controlla la connessione, oppure chiedi a un altro server, e riprova.",
                "This is not a problem with your coins: the files written so far stay valid. \
                 Check the connection, or ask another server, and try again.",
            ),
            Error::TorDown { .. } => t(
                "Tor c'è ma non risponde. Se la macchina si è appena accesa sta ancora \
                 costruendo i circuiti: aspetta e riprova. Un nodo tuo, sulla rete di casa, \
                 non passa da nessun estraneo e non chiede Tor.",
                "Tor is there but not answering. If the machine has just booted it is still \
                 building its circuits: wait and try again. A node of your own, on your home \
                 network, goes through no stranger and needs no Tor.",
            ),
            Error::NotTor { .. } => t(
                "Al posto di Tor risponde un altro proxy: da lì non passa niente, perché \
                 vedrebbe il traffico in chiaro e saprebbe da dove viene. Riavvia Tor, oppure \
                 interroga un nodo tuo sulla rete di casa.",
                "Another proxy answers in Tor's place: nothing goes through it, because it would \
                 see the traffic in the clear and know where it comes from. Restart Tor, or \
                 query a node of your own on your home network.",
            ),
            Error::Clearnet { .. } => t(
                "Serve Tor: su Tails c'è già, altrove va installato e acceso. Un nodo tuo, \
                 sulla rete di casa, non passa da nessun estraneo e non chiede Tor.",
                "Tor is needed: on Tails it is already there, elsewhere it must be installed and \
                 running. A node of your own, on your home network, goes through no stranger \
                 and needs no Tor.",
            ),
        };
        Some(remedy)
    }
}

/// Broken independence between the batch's transactions. A bug, not a
/// usage error: broadcasting one could invalidate another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisjointnessError {
    /// Two transactions spend the same outpoint.
    SharedInput {
        outpoint: OutPoint,
        tx_a: usize,
        tx_b: usize,
    },
    /// One transaction spends another's output: ordering dependency.
    ChainedTx {
        outpoint: OutPoint,
        parent: usize,
        child: usize,
    },
    /// A transaction would spend a coin already inside a refill signed
    /// before this batch.
    AlreadySigned { outpoint: OutPoint, index: usize },
    /// Required inputs dropped by the selector because already committed.
    RejectedRequired {
        index: usize,
        outpoints: Vec<OutPoint>,
    },
}

impl fmt::Display for DisjointnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            DisjointnessError::SharedInput {
                outpoint,
                tx_a,
                tx_b,
            } => {
                let (tx_a, tx_b) = (number(*tx_a), number(*tx_b));
                tf!(
                    {
                        "le transazioni #{tx_a} e #{tx_b} spendono entrambe {outpoint}. \
                      Trasmetterne una invaliderebbe l'altra."
                    },
                    {
                        "transactions #{tx_a} and #{tx_b} both spend {outpoint}. \
                      Broadcasting one would invalidate the other."
                    }
                )
            }
            DisjointnessError::ChainedTx {
                outpoint,
                parent,
                child,
            } => {
                let (parent, child) = (number(*parent), number(*child));
                tf!(
                    {
                        "la transazione #{child} spende {outpoint}, che è creato dalla #{parent}. \
                      La #{child} non sarebbe trasmissibile prima della #{parent}."
                    },
                    {
                        "transaction #{child} spends {outpoint}, which is created by #{parent}. \
                      #{child} would not be broadcastable before #{parent}."
                    }
                )
            }
            DisjointnessError::AlreadySigned { outpoint, index } => {
                let index = number(*index);
                tf!(
                    {
                        "la transazione #{index} spenderebbe {outpoint}, che sta già in una \
                         ricarica firmata prima: trasmessa una, l'altra non varrebbe più."
                    },
                    {
                        "transaction #{index} would spend {outpoint}, already inside a refill \
                         signed before: once one is broadcast, the other would be void."
                    }
                )
            }
            DisjointnessError::RejectedRequired { index, outpoints } => {
                let index = number(*index);
                let list = outpoints
                    .iter()
                    .map(|o| o.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                tf!(
                    {
                        "la transazione #{index} richiedeva monete già usate da un'altra \
                         transazione dello stesso gruppo: {list}"
                    },
                    {
                        "transaction #{index} required coins already used by another transaction \
                         in the same batch: {list}"
                    }
                )
            }
        };
        f.write_str(&text)
    }
}

impl std::error::Error for DisjointnessError {}
