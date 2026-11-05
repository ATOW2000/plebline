//! Screen output. The ceremony emits [`Line`]s and the [`Sink`] installed
//! on the current thread shows them: the terminal, the GUI, or a
//! [`Recorder`] in tests.
//!
//! The sink lives in a thread-local instead of being passed around. What is
//! printed never comes back into the ceremony the way an answer does, so
//! adding a sink argument to every function of the code that moves money
//! would buy nothing. It is per thread so the GUI's ceremony thread can
//! install its own.
//!
//! Sinks receive plain text and handle colour and wrapping themselves.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

/// Meaning of a screen line; each sink picks colour and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Blank,
    Title,
    /// Step title; numbers in [`Line::step`].
    Step,
    /// The reason for an action described just above.
    Note,
    Info,
    /// Labelled entry (`#0 bc1q…`); the label is kept apart so the text alone
    /// can be copied.
    Item,
    Success,
    Warning,
    Failure,
    /// A status light, rewritten in place (see [`Status::Working`]).
    Status(Status),
    /// A step under a status light, emitted where it actually happens.
    Trace,
    /// The one framed warning: "no network needed from here on".
    Alert,
    /// Pre-formatted fixed-width block (tables, QR, artwork). See [`Mono`].
    Mono(Mono),
    /// Error message; the remedy follows as [`Kind::Remedy`].
    Error,
    Remedy,
}

/// State of the status light next to a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// In progress. Replaced in place by a later `Done` or `Stopped`.
    Working,
    /// Retrying the same thing: replaces the whole previous block (trace,
    /// error, remedy) instead of stacking identical ones. Only the caller knows
    /// it is the same attempt.
    Retrying,
    Done,
    /// Given up. Error and remedy follow separately.
    Stopped,
}

impl Status {
    pub fn open(self) -> bool {
        matches!(self, Status::Working | Status::Retrying)
    }
}

/// Lets a GUI draw tables natively or skip the artwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mono {
    Table,
    Qr,
    Art,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    /// `(done, total)` for [`Kind::Step`], otherwise `None`.
    pub step: Option<(usize, usize)>,
    /// The label in front of the text, for [`Kind::Item`].
    pub label: Option<String>,
}

impl Line {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            step: None,
            label: None,
        }
    }
}

/// How the machine reaches the network, as the ceremony last knew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Not checked yet.
    Unknown,
    Tor,
    /// The user's own node, without Tor.
    OwnNode,
    /// Unplugged by the user, for signing.
    Unplugged,
}

/// What cancelling a question does at the current point of the ceremony.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cancel {
    /// Back to the step before.
    Back,
    /// Back to the wallet page.
    Home,
    /// No step before this one, so the ceremony stops; files already written
    /// are kept.
    Stop,
}

/// One step on the line of steps a window draws ([`Place::road`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub label: String,
    /// The wallet page, where every operation returns.
    pub home: bool,
}

/// Current position in the ceremony. Sent in full on every change, so the
/// latest one is complete and a sink that missed one loses nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub road: Vec<Stop>,
    /// Index into `road`.
    pub here: usize,
    pub link: Link,
    /// The seed words are in memory.
    pub words: bool,
    pub cancel: Cancel,
}

/// Receives the screen lines. Object-safe, so it can be chosen at run time.
pub trait Sink {
    fn line(&mut self, line: Line);

    /// Ignored by the terminal, whose step titles already show the position.
    fn place(&mut self, _place: &Place) {}

    /// A counter rewritten in place while a long step runs; `None` clears
    /// it. Only a terminal shows it.
    fn progress(&mut self, _text: Option<&str>) {}
}

/// `println!`, `owo-colors`, wrapped at 78 columns.
pub struct Terminal;

impl Sink for Terminal {
    fn line(&mut self, line: Line) {
        use owo_colors::OwoColorize;

        match line.kind {
            Kind::Blank => println!(),
            Kind::Title => {
                println!();
                println!("{}", line.text.bold());
                println!("{}", "─".repeat(line.text.chars().count()).dimmed());
            }
            Kind::Step => {
                let (done, total) = line.step.unwrap_or((0, 0));
                println!();
                println!(
                    "{} {}",
                    format!("[{done}/{total}]").cyan().bold(),
                    line.text.bold()
                );
            }
            Kind::Note => {
                for row in wrap(&line.text, 78) {
                    println!("  {}", row.dimmed());
                }
            }
            Kind::Info => println!("  {}", line.text),
            Kind::Item => println!(
                "  {} {}",
                line.label.as_deref().unwrap_or_default().cyan().bold(),
                line.text.bold()
            ),
            Kind::Success => println!("  {} {}", "✓".green().bold(), line.text),
            Kind::Warning => println!("  {} {}", "!".yellow().bold(), line.text),
            Kind::Failure => println!("  {} {}", "✗".red().bold(), line.text),
            // A terminal cannot rewrite a line, so a finished status is printed
            // as a new one.
            Kind::Status(status) => {
                let dot = match status {
                    Status::Done => "●".green().bold().to_string(),
                    // Same colour; the text and following error tell them apart.
                    Status::Working | Status::Retrying | Status::Stopped => {
                        "●".red().bold().to_string()
                    }
                };
                println!("  {dot} {}", line.text.bold());
            }
            Kind::Trace => println!("       {}", line.text.dimmed()),
            Kind::Alert => {
                let rule = "─".repeat(66);
                println!("  {}", rule.yellow());
                for row in line.text.split('\n') {
                    println!("  {}", format!("  {row}").yellow().bold());
                }
                println!("  {}", rule.yellow());
            }
            Kind::Mono(_) => println!("{}", line.text),
            Kind::Error => {
                println!();
                println!(
                    "{} {}",
                    plebline_core::lang::t("Errore:", "Error:").red().bold(),
                    line.text
                );
            }
            Kind::Remedy => {
                println!();
                for row in wrap(&line.text, 78) {
                    println!("  {}", row.yellow());
                }
                println!();
            }
        }
    }

    fn progress(&mut self, text: Option<&str>) {
        // A carriage return means nothing outside a terminal.
        if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            return;
        }
        match text {
            Some(text) => print!("\r  {text}   "),
            None => print!("\r{}\r", " ".repeat(60)),
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

/// Word wrap for the terminal only; GUIs get the whole text.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

/// Keeps lines instead of showing them, for tests. `Rc` because the
/// installed copy lives in the thread-local and the caller reads through a
/// clone.
#[derive(Clone, Default)]
pub struct Recorder {
    lines: Rc<RefCell<Vec<Line>>>,
    places: Rc<RefCell<Vec<Place>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The lines written so far.
    pub fn lines(&self) -> Vec<Line> {
        self.lines.borrow().clone()
    }

    /// Every place sent so far, in order.
    pub fn places(&self) -> Vec<Place> {
        self.places.borrow().clone()
    }

    /// The text of every line, without kinds, one per line.
    pub fn text(&self) -> String {
        self.lines
            .borrow()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Sink for Recorder {
    fn line(&mut self, line: Line) {
        self.lines.borrow_mut().push(line);
    }

    fn place(&mut self, place: &Place) {
        self.places.borrow_mut().push(place.clone());
    }
}

thread_local! {
    /// Defaults to the terminal, so a binary that prints there installs nothing.
    static SINK: RefCell<Box<dyn Sink>> = RefCell::new(Box::new(Terminal));
}

/// Redirects this thread's lines (and no other thread's).
pub fn to(sink: Box<dyn Sink>) {
    SINK.with(|current| *current.borrow_mut() = sink);
}

/// Restores the terminal and returns the previous sink, which is how tests
/// get a [`Recorder`] back.
pub fn take() -> Box<dyn Sink> {
    SINK.with(|current| std::mem::replace(&mut *current.borrow_mut(), Box::new(Terminal)))
}

/// The borrow lasts only for the call.
pub fn emit(mut line: Line) {
    // Our artwork carries its own colours; any other line may hold text
    // from a file or a phone. Blocks keep their line breaks.
    if line.kind != Kind::Mono(Mono::Art) {
        let block = matches!(
            line.kind,
            Kind::Mono(_) | Kind::Alert | Kind::Error | Kind::Note | Kind::Remedy
        );
        if let Cow::Owned(shown) = neutralize(&line.text, block) {
            line.text = shown;
        }
        if let Some(Cow::Owned(shown)) = line.label.as_deref().map(printable) {
            line.label = Some(shown);
        }
    }
    SINK.with(|current| current.borrow_mut().line(line));
}

/// Text from files and phones, made safe for one line of a terminal.
///
/// A terminal obeys control characters instead of showing them: escape
/// sequences can clear the screen, hide text or rename the window, a
/// carriage return overwrites the line, bidi overrides reverse it, and a
/// line break can imitate a line printed by the program. They are shown
/// escaped, as `\u{1b}`. This affects the screen only; what is stored,
/// signed or verified is untouched.
pub fn printable(text: &str) -> Cow<'_, str> {
    neutralize(text, false)
}

fn neutralize(text: &str, keep_line_breaks: bool) -> Cow<'_, str> {
    let obeyed = |c: char| {
        (c.is_control() && !(keep_line_breaks && c == '\n'))
            || matches!(
                c,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
    };
    if !text.chars().any(obeyed) {
        return Cow::Borrowed(text);
    }
    let mut shown = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        if obeyed(c) {
            shown.extend(c.escape_unicode());
        } else {
            shown.push(c);
        }
    }
    Cow::Owned(shown)
}

pub fn say(kind: Kind, text: impl Into<String>) {
    emit(Line::new(kind, text));
}

pub fn place(place: &Place) {
    SINK.with(|current| current.borrow_mut().place(place));
}

pub fn progress(text: Option<&str>) {
    SINK.with(|current| current.borrow_mut().progress(text));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No colour codes and no forced line breaks.
    #[test]
    fn the_recorder_receives_the_bare_text() {
        let notebook = Recorder::new();
        to(Box::new(notebook.clone()));
        let long = "una nota lunga ben oltre le settantotto colonne a cui il terminale \
                     andrebbe a capo, che qui deve arrivare tutta intera in una riga sola";
        say(Kind::Note, long);
        say(Kind::Success, "fatto");
        let written = notebook.lines();
        to(Box::new(Terminal));

        assert_eq!(written.len(), 2);
        assert_eq!(written[0].kind, Kind::Note);
        assert_eq!(written[0].text, long, "il testo non deve essere impaginato");
        assert!(
            !written[1].text.contains('\u{1b}'),
            "niente colori nel testo"
        );
        assert_eq!(written[1].text, "fatto", "niente ✓ nel testo: è un Kind");
    }

    #[test]
    fn the_step_numbers_are_not_in_the_text() {
        let notebook = Recorder::new();
        to(Box::new(notebook.clone()));
        emit(Line {
            kind: Kind::Step,
            text: "Selezione UTXO".into(),
            step: Some((2, 5)),
            label: None,
        });
        let written = notebook.lines();
        to(Box::new(Terminal));

        assert_eq!(written[0].step, Some((2, 5)));
        assert_eq!(written[0].text, "Selezione UTXO");
    }

    #[test]
    fn wrapping_happens_between_words_never_inside_one() {
        assert_eq!(
            wrap("uno due tre quattro", 9),
            vec!["uno due", "tre", "quattro"]
        );
    }

    /// The terminal is the default sink.
    #[test]
    fn with_nothing_said_it_writes_to_the_terminal() {
        say(Kind::Info, "");
    }

    /// Accents and emoji are left alone.
    #[test]
    fn terminal_commands_become_visible_text() {
        assert_eq!(printable("a\u{1b}[2Jb"), "a\\u{1b}[2Jb");
        assert_eq!(printable("riga\rsopra"), "riga\\u{d}sopra");
        assert_eq!(printable("x\u{202e}cba"), "x\\u{202e}cba");
        assert_eq!(printable("\u{9b}31m"), "\\u{9b}31m");
        assert_eq!(printable("due\nrighe"), "due\\u{a}righe");
        assert_eq!(printable("\u{7f}\u{0}\u{7}"), "\\u{7f}\\u{0}\\u{7}");
        for plain in [
            "perché ☕ 50.000 sat",
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            "",
        ] {
            assert!(matches!(printable(plain), Cow::Borrowed(_)), "{plain}");
        }
    }

    /// Blocks keep their line breaks, single lines lose them; our artwork
    /// keeps its colours.
    #[test]
    fn every_line_is_cleaned_on_its_way_to_the_screen() {
        let notebook = Recorder::new();
        to(Box::new(notebook.clone()));
        say(Kind::Info, "nota\n  ✓ falso\u{1b}[0m");
        say(Kind::Mono(Mono::Table), "┌─┐\n│\u{1b}[8m│");
        say(Kind::Mono(Mono::Art), "\u{1b}[1mPlebLine\u{1b}[0m");
        emit(Line {
            kind: Kind::Item,
            text: "tb1q".into(),
            step: None,
            label: Some("#1\u{1b}[2K".into()),
        });
        let lines = notebook.lines();
        to(Box::new(Terminal));

        assert_eq!(lines[0].text, "nota\\u{a}  ✓ falso\\u{1b}[0m");
        assert_eq!(lines[1].text, "┌─┐\n│\\u{1b}[8m│");
        assert_eq!(lines[2].text, "\u{1b}[1mPlebLine\u{1b}[0m");
        assert_eq!(lines[3].label.as_deref(), Some("#1\\u{1b}[2K"));
    }
}
