//! Presentation: steps, tables, confirmations. Before anything irreversible
//! the user reads what will happen; messages explain why, not only what.
//! Every function hands a line to [`crate::out`].

use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, CellAlignment, Table};

use crate::ask::{Ask, AskError, AskResult, Question};
use crate::out::{self, Kind, Line, Mono, Status};

pub struct Wizard {
    total: usize,
    current: usize,
}

impl Wizard {
    pub fn new(total: usize) -> Self {
        Self { total, current: 0 }
    }

    /// `[2/5] Selezione UTXO`
    pub fn step(&mut self, title: &str) {
        self.current += 1;
        out::emit(Line {
            kind: Kind::Step,
            text: title.to_string(),
            step: Some((self.current, self.total)),
            label: None,
        });
    }
}

/// Blank line, via the sink (a stray `println!` would bypass the GUI).
pub fn blank() {
    out::say(Kind::Blank, "");
}

pub fn title(text: &str) {
    out::say(Kind::Title, text);
}

/// The reason for an action described just above.
pub fn note(text: &str) {
    out::say(Kind::Note, text);
}

pub fn info(text: &str) {
    out::say(Kind::Info, text);
}

/// Labelled entry (`#0 bc1q…`): the text alone is what gets copied.
pub fn item(label: &str, text: &str) {
    out::emit(Line {
        kind: Kind::Item,
        text: text.to_string(),
        step: None,
        label: Some(label.to_string()),
    });
}

pub fn qr(rendered: &str) {
    mono(Mono::Qr, rendered);
}

pub fn success(text: &str) {
    out::say(Kind::Success, text);
}

pub fn warning(text: &str) {
    out::say(Kind::Warning, text);
}

/// Opens a status light (red, in progress), closed by [`done`] or
/// [`stopped`], with [`trace`] lines under it. Tor requests take seconds:
/// silence then an answer looks like nothing was asked.
pub fn working(text: &str) {
    out::say(Kind::Status(Status::Working), text);
}

/// Retrying the same attempt: replaces the failed block. Include the count
/// in the text. A different server is a new [`working`].
pub fn retrying(text: &str) {
    out::say(Kind::Status(Status::Retrying), text);
}

/// A step, emitted where it actually happens. Short; the why goes in a
/// [`note`].
pub fn trace(text: &str) {
    out::say(Kind::Trace, text);
}

/// Turns the status light green; the text says what came back.
pub fn done(text: &str) {
    out::say(Kind::Status(Status::Done), text);
}

/// Stops the status light; error and remedy go via [`error_with_remedy`].
pub fn stopped(text: &str) {
    out::say(Kind::Status(Status::Stopped), text);
}

pub fn failure(text: &str) {
    out::say(Kind::Failure, text);
}

/// The one framed warning of the ceremony.
pub fn alert(text: &str) {
    out::say(Kind::Alert, text);
}

/// Fixed-width block: tables, QR codes, artwork.
pub fn mono(kind: Mono, text: &str) {
    out::say(Kind::Mono(kind), text);
}

/// Error plus remedy, never a stack trace.
pub fn error_with_remedy(message: &str, remedy: Option<&str>) {
    out::say(Kind::Error, message);
    if let Some(remedy) = remedy {
        out::say(Kind::Remedy, remedy);
    }
}

pub fn table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    // No ANSI in headers: comfy-table without `tty` counts bytes for width.
    table.set_header(headers.iter().map(|h| Cell::new(*h)));
    table
}

/// Shows a [`table`] as a [`Mono::Table`] block.
pub fn show(table: &Table) {
    mono(Mono::Table, &table.to_string());
}

pub fn right(text: impl ToString) -> Cell {
    Cell::new(text.to_string()).set_alignment(CellAlignment::Right)
}

/// Confirmation defaulting to "no". Cancel counts as "no"; a dialogue
/// failure stays an error.
pub fn confirm(ask: &mut dyn Ask, prompt: &str) -> AskResult<bool> {
    match ask.confirm(Question::new(prompt), false) {
        Ok(answer) => Ok(answer),
        Err(AskError::Cancelled) => Ok(false),
        Err(other) => Err(other),
    }
}

pub fn sats(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(c);
    }
    format!("{grouped} sat")
}

pub fn btc(value: u64) -> String {
    format!("{:.8} BTC", value as f64 / 100_000_000.0)
}

/// The txid's first 8 and last 4 characters on screen; files keep the whole.
pub fn short_outpoint(outpoint: &str) -> String {
    match outpoint.split_once(':') {
        Some((txid, vout)) if txid.len() > 12 && txid.is_ascii() => {
            format!("{}…{}:{vout}", &txid[..8], &txid[txid.len() - 4..])
        }
        _ => outpoint.to_string(),
    }
}

/// By characters, not bytes: the address may come from a file, and a byte
/// cut inside a character would panic.
pub fn short_address(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    if chars.len() <= 24 {
        return address.to_string();
    }
    let head: String = chars[..12].iter().collect();
    let tail: String = chars[chars.len() - 8..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grouped by spaces: dots would read as the decimals of the BTC beside.
    #[test]
    fn amounts_are_grouped_by_thousands() {
        assert_eq!(sats(1_234_567), "1 234 567 sat");
        assert_eq!(sats(999), "999 sat");
        assert_eq!(sats(1_000), "1 000 sat");
    }

    #[test]
    fn a_real_address_keeps_its_ends() {
        let address = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        assert_eq!(short_address(address), "bc1qw508d6qe…7kv8f3t4");
        assert_eq!(short_address("bc1qshort"), "bc1qshort");
    }

    /// Text from a hostile file must not panic on a character boundary.
    #[test]
    fn non_ascii_text_does_not_panic() {
        // One ASCII letter first, so a byte cut lands inside a character.
        let address = format!("bc1qx{}", "é".repeat(24));
        assert_eq!(short_address(&address), "bc1qxééééééé…éééééééé");
        let outpoint = format!("x{}:0", "é".repeat(14));
        assert_eq!(short_outpoint(&outpoint), outpoint);
    }
}
