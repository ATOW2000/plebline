//! `plebline-guided`: the same steps every time up to the wallet page, then
//! the wallet page. A library so the window and the tests run the same
//! ceremony: `run` takes an [`Ask`] and a [`Chain`], and writes through
//! [`plebline::out`].
//!
//! `run_in` holds the step order up to the wallet, `home` what follows;
//! each step has its own module with tests. End-to-end tests are in `tests/`.
//!
//! The user is asked only what the program cannot know: seed and
//! passphrase, recipients and amounts, the fee, and the server (the largest
//! privacy decision, never made silently).

use plebline_core::bitcoin::Network;
use plebline_core::lang::{t, Lang};
use plebline_core::wallet::SeedWords;
use plebline_core::{Error, Result};
use zeroize::Zeroizing;

use plebline::ask::Ask;
use plebline::chain::Chain;
use plebline::out::Cancel;
use plebline::project::{self, Project};
use plebline::{ui, wizard};

mod ceremony;
mod close;
mod export;
#[cfg(test)]
mod fixtures;
mod home;
mod intro;
mod phone;
mod road;
mod seed;
mod seed_guide;
mod send;
mod sign;
mod snapshot;
mod wallet_page;

use ceremony::start_fresh;
use intro::{intro, step_lang, step_what_it_is};
use seed::{step_public_half, step_seed, wallet_fingerprint, warn_if_other_wallet};
use snapshot::{step_chain, step_network};

/// Always mainnet, except the separate `--features signet` test build,
/// which announces itself.
#[cfg(not(feature = "signet"))]
pub const NETWORK: Network = Network::Bitcoin;
#[cfg(feature = "signet")]
pub const NETWORK: Network = Network::Signet;

pub const TEST_NETWORK: bool = cfg!(feature = "signet");

/// Fixed name, so the program finds its own progress.
pub const CEREMONY: &str = "cassaforte";

/// `--version` line.
pub fn version_line() -> String {
    format!(
        "PlebLine {}{}",
        env!("CARGO_PKG_VERSION"),
        if TEST_NETWORK { " (signet)" } else { "" }
    )
}

/// Test-build warning, or `None` for the real binary.
pub fn test_network_warning() -> Option<&'static str> {
    TEST_NETWORK.then(|| {
        t(
            "PROVA: rete signet. Queste monete non valgono niente, e questo binario\n\
             non può toccare quelle vere.",
            "TEST: signet network. These coins are worth nothing, and this binary\n\
             cannot touch real ones.",
        )
    })
}

/// Language from `argv`, by hand: no clap here.
pub fn lang_from_argv() -> Option<Lang> {
    let args: Vec<String> = std::env::args().collect();
    for (index, arg) in args.iter().enumerate() {
        let value = match arg.split_once('=') {
            Some(("--lang", value)) => Some(value.to_string()),
            _ if arg == "--lang" => args.get(index + 1).cloned(),
            _ => None,
        };
        if let Some(value) = value {
            return value.parse().ok();
        }
    }
    None
}

/// The seed and passphrase, kept in memory until the network is plugged back
/// in, so they are typed once for both the public half and the signing.
pub(crate) struct Open {
    seed: SeedWords,
    passphrase: Option<Zeroizing<String>>,
}

impl Open {
    fn access(&self) -> wizard::WalletAccess<'_> {
        wizard::WalletAccess::Seed {
            seed: &self.seed,
            passphrase: self.passphrase.as_deref().map(String::as_str),
        }
    }

    /// Descriptor checksum of the wallet these words open, the name that
    /// `wallet.json` uses.
    fn checksum(&self) -> Result<String> {
        let wallet = plebline_core::wallet::from_seed(
            &self.seed,
            self.passphrase.as_deref().map(String::as_str),
            NETWORK,
        )?;
        Ok(wallet.descriptor_checksum(plebline_core::bdk_wallet::KeychainKind::External))
    }
}

/// Prints error, cause and remedy, with this path's own remedies: some of
/// the core's mention options that do not exist here.
pub fn report(error: &Error) {
    plebline::report_with(error, snapshot::remedy(error).as_deref());
}

/// Resolves the directory (a machine fact), then runs the ceremony
/// (`run_in` is testable in a temporary directory).
pub fn run(ask: &mut dyn Ask, chain: &mut dyn Chain) -> Result<()> {
    run_in(&project::open_or_create(CEREMONY)?, ask, chain)
}

/// The steps up to the wallet. Cancelling one goes back to the one before;
/// cancelling the first stops the ceremony.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Before {
    Language,
    Welcome,
    Origin,
    Network,
    Words,
}

fn run_in(ceremony: &Project, ask: &mut dyn Ask, chain: &mut dyn Chain) -> Result<()> {
    road::begin();
    let asks_language = lang_from_argv().is_none();
    let mut at = Before::Language;
    // Chosen at the network step, reused after: asked once.
    let mut server = None;

    let open = loop {
        match at {
            // Language, unless `--lang` was given; then the greeting.
            Before::Language => {
                if asks_language {
                    road::cancel(Cancel::Stop);
                    step_lang(ask)?;
                    // Sent again: the step names went out in the default language.
                    road::begin();
                }
                if let Some(warning) = test_network_warning() {
                    ui::alert(warning);
                }
                at = Before::Welcome;
            }

            // The optional explanation, the start button, then the first question.
            Before::Welcome => {
                road::cancel(if asks_language {
                    Cancel::Back
                } else {
                    Cancel::Stop
                });
                match back(step_what_it_is(ask))? {
                    Some(()) => {
                        intro();
                        at = Before::Origin;
                    }
                    None if asks_language => at = Before::Language,
                    None => return Err(Error::Cancelled),
                }
            }

            // The same steps every time up to the wallet, where the user picks each
            // operation. The files on disk never pick it: if they did, every second
            // ceremony would land in "verify and send".

            // "Do you have the words?" is always asked. `wallet.json` never
            // decides anything: it is rewritten from what is typed, and a
            // different previous wallet is announced.
            Before::Origin => {
                road::at(road::Point::Start);
                road::cancel(Cancel::Back);
                match back(seed_guide::step_wallet_origin(ceremony, ask))? {
                    Some(true) => at = Before::Network,
                    Some(false) => return Ok(()),
                    None => at = Before::Welcome,
                }
            }

            // Server choice and a reachability check come before the words, so a Tor
            // that is not running shows up before anything is typed. The server
            // chosen here is reused.
            Before::Network => {
                road::at(road::Point::Network);
                road::cancel(Cancel::Back);
                match back(step_network(ask, chain))? {
                    Some(Some(chosen)) => {
                        server = Some(chosen);
                        at = Before::Words;
                    }
                    // Nothing written, nothing typed.
                    Some(None) => return Ok(()),
                    None => at = Before::Origin,
                }
            }

            Before::Words => {
                road::at(road::Point::Words);
                road::cancel(Cancel::Back);
                match back(open_wallet(ceremony, &mut server, ask, chain))? {
                    Some(Some(open)) => break open,
                    Some(None) => return Ok(()),
                    // The words were dropped on the way out.
                    None => {
                        road::words(false);
                        at = Before::Network;
                    }
                }
            }
        }
    };

    // The wallet: every operation starts there and comes back there, until
    // "close and delete".
    let mut session = home::Session {
        open: Some(open),
        server,
        online: true,
        returned: false,
        copied: Default::default(),
    };
    home::run(ceremony, &mut session, ask, chain)
}

/// Asks for the words, writes the public half and reads the coins, up to
/// the wallet page. `None` when the chain step stops the ceremony.
fn open_wallet(
    ceremony: &Project,
    server: &mut Option<String>,
    ask: &mut dyn Ask,
    chain: &mut dyn Chain,
) -> Result<Option<Open>> {
    let open = step_seed(ask)?;
    road::words(true);
    let previous = wallet_fingerprint(ceremony);
    step_public_half(ceremony, &open, ask)?;
    warn_if_other_wallet(ceremony, previous.as_deref());

    // The coin scan is shown under the wallet point.
    road::at(road::Point::Wallet);

    // From zero every time: nothing from earlier sessions is checked or
    // committed, and the coins are read again however recent.
    start_fresh(ceremony)?;
    let Some(chosen) = step_chain(ceremony, server.clone(), ask, chain)? else {
        return Ok(None);
    };
    *server = Some(chosen);
    Ok(Some(open))
}

fn exists(ceremony: &Project, file: &str) -> Option<std::path::PathBuf> {
    ceremony.has(file).then(|| ceremony.path(file))
}

/// `None` for a cancel, so the caller can go back to the step before or to
/// the wallet; other errors pass. A cancel caught nowhere stops the
/// ceremony, and the files written stay.
pub(crate) fn back<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::Cancelled) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
