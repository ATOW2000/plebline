//! Parsing the destination: a bare address or a BIP 21 URI, checked against
//! the network. It has its own module and tests because a payment to a wrong
//! but well-formed address goes through, to someone else.

use std::str::FromStr;

use bdk_wallet::bitcoin::address::NetworkUnchecked;
use bdk_wallet::bitcoin::{Address, Amount, Denomination, Network};

use crate::lang::t;
use crate::{tf, Error, Result};

/// A parsed destination. URI amount and label are suggestions: the caller
/// still asks for confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub address: Address,
    /// URI amount, in sat.
    pub amount_sat: Option<u64>,
    pub label: Option<String>,
}

/// Reads a destination from whatever the user pasted or scanned.
pub fn parse(input: &str, network: Network) -> Result<Destination> {
    let text = input.trim();
    if text.is_empty() {
        return Err(Error::Input(
            t("non hai scritto niente", "nothing was entered").into(),
        ));
    }

    match strip_scheme(text) {
        Some(rest) => parse_uri(rest, network),
        None => Ok(Destination {
            address: parse_address(text, network)?,
            amount_sat: None,
            label: None,
        }),
    }
}

/// One destination per line, blank lines skipped: a list sent from a phone.
/// One bad line rejects the whole list, because it may be a cut or a mix-up
/// and the other lines would then be trusted by position.
pub fn parse_list(input: &str, network: Network) -> Result<Vec<Destination>> {
    let mut list = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let number = index + 1;
        let destination = parse(line, network)
            .map_err(|e| Error::Input(tf!({ "riga {number}: {e}" }, { "line {number}: {e}" })))?;
        list.push(destination);
    }
    if list.is_empty() {
        return Err(Error::Input(
            t("non c'è nessun indirizzo", "there is no address in it").into(),
        ));
    }
    Ok(list)
}

/// Address for the given network; a network mismatch fails here, while
/// typing.
pub fn parse_address(text: &str, network: Network) -> Result<Address> {
    let text = text.trim();

    let unchecked = Address::<NetworkUnchecked>::from_str(text)
        // Uppercase is legal bech32 (compact QR mode). Retry lowercase only if
        // there was no lowercase at all, so hand typos are not "fixed".
        .or_else(|first| {
            if text.chars().any(|c| c.is_ascii_lowercase()) {
                Err(first)
            } else {
                Address::<NetworkUnchecked>::from_str(&text.to_ascii_lowercase())
            }
        })
        .map_err(|e| {
            Error::Input(tf!({ "indirizzo '{text}' non valido: {e}" }, {
                "invalid address '{text}': {e}"
            }))
        })?;

    unchecked.require_network(network).map_err(|_| {
        Error::Input(tf!(
            { "l'indirizzo '{text}' non appartiene alla rete {network}" },
            { "address '{text}' does not belong to the {network} network" }
        ))
    })
}

/// Strips a case-insensitive `bitcoin:` prefix.
fn strip_scheme(text: &str) -> Option<&str> {
    const SCHEME: &str = "bitcoin:";
    let head: String = text.chars().take(SCHEME.len()).collect();
    head.eq_ignore_ascii_case(SCHEME)
        .then(|| &text[SCHEME.len()..])
}

fn parse_uri(rest: &str, network: Network) -> Result<Destination> {
    let (address_text, query) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };

    let address = parse_address(address_text, network)?;
    let mut amount_sat = None;
    let mut label = None;

    for pair in query.unwrap_or_default().split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);

        match key.to_ascii_lowercase().as_str() {
            "amount" => {
                // BIP 21 amounts are in BTC, not sat.
                let amount = Amount::from_str_in(&value, Denomination::Bitcoin).map_err(|e| {
                    Error::Input(tf!({ "importo '{value}' non leggibile: {e}" }, {
                        "unreadable amount '{value}': {e}"
                    }))
                })?;
                if amount > Amount::MAX_MONEY {
                    return Err(Error::Input(tf!(
                        { "importo '{value}': più di tutti i bitcoin che esisteranno mai" },
                        { "amount '{value}': more than all the bitcoin there will ever be" }
                    )));
                }
                amount_sat = Some(amount.to_sat());
            }
            "label" | "message" if label.is_none() => {
                if !value.trim().is_empty() {
                    label = Some(value);
                }
            }
            "label" | "message" => {}
            other if other.starts_with("req-") => {
                // BIP 21: `req-` means "do not pay without understanding this".
                return Err(Error::Input(tf!(
                    {
                        "questo codice chiede una cosa che PlebLine non sa fare ({other}): \
                       non usarlo, e chiedi un indirizzo normale"
                    },
                    {
                        "this code requires something PlebLine cannot do ({other}): \
                       do not use it, ask for a plain address instead"
                    }
                )));
            }
            // Other parameters are ignored, per BIP 21.
            _ => {}
        }
    }

    Ok(Destination {
        address,
        amount_sat,
        label,
    })
}

/// Minimal percent decoding, to avoid a dependency. Also used for dragged
/// `file://` paths.
pub fn percent_decode(value: &str) -> String {
    let mut byte = Vec::new();
    let mut rest = value.as_bytes();

    while let Some((first, tail)) = rest.split_first() {
        match first {
            b'%' if tail.len() >= 2 => {
                let digits = std::str::from_utf8(&tail[..2]).ok();
                match digits.and_then(|c| u8::from_str_radix(c, 16).ok()) {
                    Some(value) => {
                        byte.push(value);
                        rest = &tail[2..];
                        continue;
                    }
                    // A stray `%` stays literal.
                    None => byte.push(b'%'),
                }
            }
            b'+' => byte.push(b' '),
            other => byte.push(*other),
        }
        rest = tail;
    }

    String::from_utf8_lossy(&byte).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNET: Network = Network::Signet;

    /// A real signet address.
    const TB: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    #[test]
    fn a_bare_address_passes() {
        let d = parse(TB, SIGNET).unwrap();
        assert_eq!(d.address.to_string(), TB);
        assert_eq!(d.amount_sat, None);
        assert_eq!(d.label, None);
    }

    #[test]
    fn surrounding_spaces_do_not_count() {
        // Pasted text almost always comes with a space or a newline.
        assert!(parse(&format!("  {TB}\n"), SIGNET).is_ok());
    }

    #[test]
    fn the_wrong_network_stops_here() {
        let mainnet = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        let error = parse(mainnet, SIGNET).unwrap_err().to_string();
        assert!(error.contains("signet"), "{error}");
    }

    #[test]
    fn a_bip21_uri_carries_address_and_amount() {
        let d = parse(&format!("bitcoin:{TB}?amount=0.001"), SIGNET).unwrap();
        assert_eq!(d.address.to_string(), TB);
        assert_eq!(d.amount_sat, Some(100_000));
    }

    #[test]
    fn the_scheme_may_be_uppercase() {
        assert!(parse(&format!("BITCOIN:{TB}"), SIGNET).is_ok());
    }

    #[test]
    fn an_all_caps_address_from_a_qr_is_read() {
        let upper = format!("bitcoin:{}", TB.to_ascii_uppercase());
        let d = parse(&upper, SIGNET).unwrap();
        assert_eq!(d.address.to_string(), TB);
    }

    #[test]
    fn the_label_is_read_and_decoded() {
        let d = parse(&format!("bitcoin:{TB}?label=Mario%20Rossi"), SIGNET).unwrap();
        assert_eq!(d.label.as_deref(), Some("Mario Rossi"));
    }

    #[test]
    fn an_unknown_req_parameter_stops_everything() {
        let error = parse(&format!("bitcoin:{TB}?req-qualcosa=1"), SIGNET).unwrap_err();
        assert!(error.to_string().contains("req-qualcosa"));
    }

    #[test]
    fn an_unknown_plain_parameter_is_ignored() {
        assert!(parse(&format!("bitcoin:{TB}?pj=https://x.example"), SIGNET).is_ok());
    }

    #[test]
    fn an_unreadable_amount_is_an_error_not_a_zero() {
        assert!(parse(&format!("bitcoin:{TB}?amount=tanti"), SIGNET).is_err());
    }

    #[test]
    fn an_empty_input_says_so() {
        assert!(parse("   ", SIGNET).is_err());
    }

    #[test]
    fn a_malformed_address_is_not_fixed_up() {
        // One letter off: rejected, with no attempt to correct it.
        let mangled = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsy";
        assert!(parse(mangled, SIGNET).is_err());
    }

    /// Another real signet address (BIP 173 P2WSH vector).
    const TB2: &str = "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7";

    #[test]
    fn a_list_keeps_order_and_skips_blank_lines() {
        let text = format!("\n{TB}\r\n\n  bitcoin:{TB2}?amount=0.001  \n");
        let list = parse_list(&text, SIGNET).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].address.to_string(), TB);
        assert_eq!(list[1].address.to_string(), TB2);
        assert_eq!(list[1].amount_sat, Some(100_000));
    }

    #[test]
    fn one_bad_line_refuses_the_whole_list_and_names_it() {
        let text = format!("{TB}\n{}\n", &TB2[..TB2.len() - 1]);
        let error = parse_list(&text, SIGNET).unwrap_err().to_string();
        assert!(
            error.contains("riga 2") || error.contains("line 2"),
            "{error}"
        );
    }

    #[test]
    fn an_empty_list_is_not_a_list() {
        assert!(parse_list(" \n\n", SIGNET).is_err());
    }

    /// Input from the phone is untrusted: one bad line rejects the whole list.
    #[test]
    fn a_hostile_line_from_the_phone_is_refused() {
        let mixed = format!("{}{}", &TB[..4], TB[4..].to_ascii_uppercase());
        // Cyrillic `с` in place of the Latin `c`.
        let lookalike = TB.replacen('c', "с", 1);
        let cases = [
            ("maiuscole e minuscole", mixed),
            ("lettera che sembra uguale", lookalike),
            ("spazio in mezzo", format!("{} {}", &TB[..10], &TB[10..])),
            ("due indirizzi sulla riga", format!("{TB} {TB2}")),
            ("carattere nullo", format!("{TB}\0")),
            ("riga lunghissima", "tb1q".repeat(50_000)),
            ("importo negativo", format!("bitcoin:{TB}?amount=-1")),
            ("importo oltre u64", format!("bitcoin:{TB}?amount=1e30")),
            (
                "importo oltre tutti i bitcoin",
                format!("bitcoin:{TB}?amount=21000000.00000001"),
            ),
            ("solo lo schema", "bitcoin:".to_string()),
        ];
        for (what, line) in cases {
            let text = format!("{TB2}\n{line}\n");
            assert!(parse_list(&text, SIGNET).is_err(), "accettata: {what}");
        }
    }

    #[test]
    fn all_the_bitcoin_there_is_can_be_proposed_and_no_more() {
        let d = parse(&format!("bitcoin:{TB}?amount=21000000"), SIGNET).unwrap();
        assert_eq!(d.amount_sat, Some(Amount::MAX_MONEY.to_sat()));
    }

    /// Broken `%` escapes become U+FFFD or stay literal; the URI still parses.
    #[test]
    fn broken_percent_escapes_are_harmless() {
        assert_eq!(percent_decode("%ff%fe"), "\u{fffd}\u{fffd}");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("%é1"), "%é1");
        assert!(parse(&format!("bitcoin:{TB}?label=%ff%00%"), SIGNET).is_ok());
    }
}
