//! How a request may leave: through Tor, or to your own node.
//!
//! Scanning a descriptor on Esplora reveals every past and future address
//! and the balance; without Tor, also the user's IP. The program opens its own
//! SOCKS5 connection rather than relying on `torsocks` (which hardened
//! runtime blocks on macOS).
//!
//! - [`Route::Tor`]: `socks5h`, so DNS resolution happens inside Tor too.
//! - [`Route::OwnNode`]: loopback or LAN, no proxy (Tor cannot route there
//!   and there is no third party).
//!
//! Anything else is refused before the packet leaves. This guards against
//! mistakes, not against the user: the proxy can be pointed anywhere.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use url::{Host, Url};

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

/// Default Tor SOCKS port. `socks5h` keeps DNS inside Tor.
pub const DEFAULT_PROXY: &str = "socks5h://127.0.0.1:9050";

/// Loopback answers at once or not at all.
const PROBE: Duration = Duration::from_millis(700);

/// How traffic can leave this machine. Buildable by hand for tests.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub proxy: Option<String>,
    /// Probed when the environment is read: "not installed" and "not
    /// running" have different remedies, and a refused connection mid-scan
    /// looks like a bug.
    pub proxy_answers: bool,
}

impl Env {
    pub fn read(proxy: Option<&str>) -> Self {
        let proxy = proxy.map(str::to_string);
        let proxy_answers = proxy.as_deref().is_some_and(reachable);
        Self {
            proxy,
            proxy_answers,
        }
    }
}

fn reachable(proxy: &str) -> bool {
    let Ok(url) = Url::parse(proxy.trim()) else {
        return false;
    };
    let Some(port) = url.port() else {
        return false;
    };
    let ip = match url.host() {
        Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
        // `socks5h` is not a scheme `url` knows, so an IPv4 host comes back
        // as text.
        Some(Host::Domain(name)) => match name.parse() {
            Ok(ip) => ip,
            // A hostname: resolving it here would leak DNS outside Tor. Let
            // the connection decide.
            Err(_) => return true,
        },
        None => return false,
    };
    TcpStream::connect_timeout(&SocketAddr::new(ip, port), PROBE).is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Through the SOCKS proxy, which is Tor.
    Tor { proxy: String },
    /// Direct: your own node, unreachable through Tor anyway.
    OwnNode,
}

impl Route {
    pub fn proxy(&self) -> Option<&str> {
        match self {
            Route::Tor { proxy } => Some(proxy),
            Route::OwnNode => None,
        }
    }

    /// Concurrent requests. Two through Tor: bursts from one exit get
    /// connections closed mid-scan (`IncompleteMessage`).
    pub fn parallel_requests(&self) -> usize {
        match self {
            Route::Tor { .. } => 2,
            Route::OwnNode => 5,
        }
    }

    /// The line shown before the request.
    pub fn description(&self) -> &'static str {
        match self {
            Route::Tor { .. } => t("La rete esce da Tor.", "Traffic leaves through Tor."),
            Route::OwnNode => t(
                "Server sulla tua rete: nessun estraneo in mezzo, e Tor non serve.",
                "Server on your own network: no stranger in between, and no Tor needed.",
            ),
        }
    }
}

/// The whole rule is in this one function, so no caller can apply only part
/// of it.
pub fn route(api: &str, env: &Env) -> Result<Route> {
    let parsed = host_of(api);

    // Own node first: allowed regardless of Tor state.
    if parsed.as_ref().is_some_and(is_own_node) {
        return Ok(Route::OwnNode);
    }

    let host = parsed.map_or_else(|| api.to_string(), |host| host.to_string());
    match &env.proxy {
        Some(proxy) if env.proxy_answers => Ok(Route::Tor {
            proxy: proxy.clone(),
        }),
        // Configured but not answering: the remedy is to start Tor.
        Some(proxy) => Err(Error::TorDown {
            host,
            proxy: proxy.clone(),
        }),
        None => Err(Error::Clearnet { host }),
    }
}

/// An answer from an onion address can only have come through Tor, so it is
/// shown as proof.
pub fn is_onion(api: &str) -> bool {
    matches!(host_of(api), Some(Host::Domain(name)) if name.ends_with(".onion"))
}

/// Host of an http(s) URL, read by the parser the HTTP client connects with:
/// a rule that read it differently would let `a.example\@192.168.1.1` or
/// `134744072` pass for a local node while the connection goes to a public
/// host. `None` falls on the Tor side.
fn host_of(api: &str) -> Option<Host> {
    let url = Url::parse(api.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.host().map(|host| host.to_owned())
}

/// Loopback or local network?
fn is_own_node(host: &Host) -> bool {
    match host {
        Host::Ipv4(v4) => is_local((*v4).into()),
        Host::Ipv6(v6) => is_local((*v6).into()),
        // `localhost` and local-only suffixes, checked again once resolved
        // ([`check_resolved`]). Not `.onion`: that must go through the Tor
        // check.
        Host::Domain(name) if name == "localhost" => true,
        Host::Domain(name) => match name.rsplit_once('.') {
            Some((_, suffix)) => {
                matches!(suffix, "local" | "lan" | "home" | "internal" | "localhost")
            }
            // A dotless name is resolved locally (`umbrel`, `nodo`).
            None => true,
        },
    }
}

fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            // `is_unique_local`/`is_unicast_link_local` are unstable: fc00::/7 and
            // fe80::/10 checked by hand.
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// A name let through as your own node must resolve only to local
/// addresses: a search domain or a lying resolver could otherwise send the
/// request, in the clear, to a public host. The caller connects to exactly
/// these addresses.
pub(crate) fn check_resolved(name: &str, addresses: &[SocketAddr]) -> Result<()> {
    match addresses.iter().find(|a| !is_local(a.ip())) {
        _ if addresses.is_empty() => Err(Error::Network(tf!(
            { "{name} non corrisponde a nessun indirizzo" },
            { "{name} resolves to no address" }
        ))),
        Some(public) => {
            let ip = public.ip();
            Err(Error::Refused(tf!(
                {
                    "{name} ha il nome di un nodo di casa ma porta a {ip}, fuori dalla rete \
                     di casa: senza Tor non lo interrogo. Scrivi l'indirizzo IP locale del \
                     tuo nodo."
                },
                {
                    "{name} looks like a home node but leads to {ip}, outside your home \
                     network: I will not query it without Tor. Type your node's local IP \
                     address."
                }
            )))
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tor_up() -> Env {
        Env {
            proxy: Some(DEFAULT_PROXY.to_string()),
            proxy_answers: true,
        }
    }

    /// Tor installed but not running.
    fn tor_down() -> Env {
        Env {
            proxy: Some(DEFAULT_PROXY.to_string()),
            proxy_answers: false,
        }
    }

    fn no_tor() -> Env {
        Env::default()
    }

    #[test]
    fn the_public_server_without_tor_is_refused() {
        let error = route("https://blockstream.info/api", &no_tor()).expect_err("deve rifiutare");
        assert!(
            matches!(&error, Error::Clearnet { host } if host == "blockstream.info"),
            "{error:?}"
        );
    }

    #[test]
    fn with_tor_up_it_goes_through_the_proxy() {
        let route = route("https://blockstream.info/api", &tor_up()).unwrap();
        assert_eq!(route.proxy(), Some(DEFAULT_PROXY));
    }

    /// "Not running" and "not installed" get different remedies.
    #[test]
    fn tor_installed_but_not_running_says_so() {
        let error = route("https://blockstream.info/api", &tor_down()).expect_err("rifiuta");
        let text = error.to_string();
        assert!(text.contains("blockstream.info"), "{text}");
        assert!(
            text.contains("non risponde") || text.contains("not answering"),
            "{text}"
        );
        assert!(error.remedy().is_some());
    }

    #[test]
    fn the_refusal_says_which_server() {
        let error = route("https://mempool.space/signet/api", &no_tor()).expect_err("rifiuta");
        assert!(error.to_string().contains("mempool.space"), "{error}");
    }

    #[test]
    fn your_own_node_needs_no_tor_and_no_proxy() {
        for api in [
            "http://umbrel.local:3002",
            "http://127.0.0.1:3002",
            "http://localhost:3002/api",
            "http://192.168.1.40:3002",
            "http://10.0.0.5/api",
            "http://172.16.3.9:3002",
            "http://[::1]:3002",
            "http://nodo:3002",
        ] {
            let route = route(api, &no_tor()).unwrap_or_else(|e| panic!("{api} rifiutato: {e}"));
            assert_eq!(route, Route::OwnNode, "{api}");
            assert_eq!(route.proxy(), None, "{api} non deve passare da un proxy");
        }
    }

    /// 172.32 is public, outside 172.16/12.
    #[test]
    fn an_address_that_only_looks_private_is_not_your_node() {
        assert!(route("http://172.32.0.1:3002", &no_tor()).is_err());
        assert!(route("http://11.0.0.1:3002", &no_tor()).is_err());
    }

    /// Onion goes to the Tor branch, not the own-node exception.
    #[test]
    fn a_home_node_name_must_lead_home() {
        let at = |ip: &str| SocketAddr::new(ip.parse().unwrap(), 3002);

        check_resolved("umbrel.local", &[at("192.168.1.20"), at("fe80::1")])
            .expect("indirizzi di casa");
        let error = check_resolved("umbrel.local", &[at("192.168.1.20"), at("8.8.8.8")])
            .expect_err("un indirizzo pubblico non è casa");
        assert!(matches!(error, Error::Refused(_)), "{error:?}");
        assert!(error.to_string().contains("8.8.8.8"), "{error}");
        assert!(matches!(
            check_resolved("nodo", &[]),
            Err(Error::Network(_))
        ));
    }

    #[test]
    fn an_onion_address_is_recognized_as_one() {
        assert!(is_onion(
            "http://explorerzydxu5ecjrkwceayqybizmpjjznk5izmitf2modhcusuqlid.onion/api"
        ));
        assert!(is_onion("http://qualcosa.onion:8080/api"));
        assert!(!is_onion("https://onion.example.com/api"));
        assert!(!is_onion("https://mempool.space/api"));
        assert!(!is_onion("http://127.0.0.1:3002"));
    }

    #[test]
    fn an_onion_address_still_asks_for_tor() {
        let onion = "http://explorerzydxu5ecjrkwceayqybizmpjjznk5izmitf2modhcusuqlid.onion/api";
        assert!(route(onion, &no_tor()).is_err());
        assert!(matches!(
            route(onion, &tor_up()).unwrap(),
            Route::Tor { .. }
        ));
    }

    #[test]
    fn closed_port_is_reported_without_waiting() {
        assert!(!reachable("socks5h://127.0.0.1:1"));
        assert!(!reachable("socks5h://127.0.0.1:non-un-numero"));
        assert!(!reachable("senza-porta"));
        // An IPv6 proxy is probed too, not taken for a name.
        assert!(!reachable("socks5h://[::1]:1"));
    }

    #[test]
    fn reading_the_machine_reports_a_closed_proxy_port() {
        let env = Env::read(Some("socks5h://127.0.0.1:1"));
        assert!(!env.proxy_answers);
        assert!(route("https://blockstream.info/api", &env).is_err());
    }

    fn host(api: &str) -> Option<String> {
        host_of(api).map(|host| host.to_string())
    }

    #[test]
    fn the_host_comes_out_of_the_url_whatever_it_carries() {
        let cases = [
            ("https://mempool.space/signet/api", "mempool.space"),
            ("http://umbrel.local:3002", "umbrel.local"),
            ("https://user:pass@esempio.org/api", "esempio.org"),
            ("http://[::1]:3002/api", "[::1]"),
            ("http://[fd00::1]/api", "[fd00::1]"),
            ("https://ESEMPIO.ORG/api", "esempio.org"),
        ];
        for (api, expected) in cases {
            assert_eq!(host(api).as_deref(), Some(expected), "{api}");
        }
    }

    /// An `@` after the path is not credentials.
    #[test]
    fn an_at_sign_in_the_path_does_not_move_the_host() {
        assert_eq!(
            host("https://esempio.org/api/a@b").as_deref(),
            Some("esempio.org")
        );
    }

    /// Spellings that a hand-written parser took for a local node, while
    /// the HTTP client connects to a public host.
    #[test]
    fn spellings_of_a_public_host_do_not_pass_for_your_own_node() {
        for (api, public) in [
            (
                "http://blockstream.info\\@192.168.1.1/api",
                "blockstream.info",
            ),
            ("http://134744072:3002", "8.8.8.8"),
            ("http://0x8080808/api", "8.8.8.8"),
            ("http://blockstream%2Einfo/api", "blockstream.info"),
            ("http://mempool\u{3002}space/api", "mempool.space"),
        ] {
            assert_eq!(host(api).as_deref(), Some(public), "{api}");
            assert!(
                matches!(route(api, &no_tor()), Err(Error::Clearnet { .. })),
                "{api} senza Tor va rifiutato"
            );
            assert!(
                matches!(route(api, &tor_up()), Ok(Route::Tor { .. })),
                "{api} con Tor passa dal proxy"
            );
        }
    }

    /// Unparsable input, or a scheme the client does not speak, counts as a
    /// remote server, so it needs Tor.
    #[test]
    fn what_cannot_be_read_does_not_pass_for_your_own_node() {
        for rubbish in [
            "",
            "://",
            "http://",
            "umbrel.local:3002",
            "ftp://127.0.0.1/api",
        ] {
            assert!(
                route(rubbish, &no_tor()).is_err(),
                "{rubbish} non deve passare"
            );
        }
    }
}
