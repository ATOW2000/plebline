//! Questions to the user. The flow holds a `&mut dyn Ask` and does not know
//! who answers: the window's implementation, or `Scripted` in tests.
//!
//! Each caller decides what cancelling means, so [`AskError`] keeps a cancel
//! apart from a failure.

use plebline_core::address::Destination;
use plebline_core::bitcoin::Network;
use plebline_core::wallet::SeedWords;

/// Why a question got no answer: a user choice, or a failure.
#[derive(Debug)]
pub enum AskError {
    /// Esc or Ctrl-C.
    Cancelled,
    /// No dialogue possible: no terminal, non-interactive input, I/O error.
    Unavailable(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::Cancelled => f.write_str(plebline_core::lang::t("annullato", "cancelled")),
            AskError::Unavailable(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for AskError {}

/// Cancelling stays `Cancelled` whatever the step, so the flow can take it
/// as "go back".
impl From<AskError> for plebline_core::Error {
    fn from(error: AskError) -> Self {
        match error {
            AskError::Cancelled => plebline_core::Error::Cancelled,
            AskError::Unavailable(reason) => plebline_core::Error::Interaction(reason),
        }
    }
}

pub type AskResult<T> = std::result::Result<T, AskError>;

/// Prompt, help line and invalid-type message.
#[derive(Clone, Copy)]
pub struct Question<'a> {
    pub prompt: &'a str,
    pub help: Option<&'a str>,
    pub invalid: Option<&'a str>,
}

impl<'a> Question<'a> {
    pub fn new(prompt: &'a str) -> Self {
        Self {
            prompt,
            help: None,
            invalid: None,
        }
    }

    pub fn help(mut self, text: &'a str) -> Self {
        self.help = Some(text);
        self
    }

    /// Numbers only: asked again until the answer parses.
    pub fn invalid(mut self, text: &'a str) -> Self {
        self.invalid = Some(text);
        self
    }
}

/// Who answers the ceremony's questions. Object-safe, so the flow takes
/// `&mut dyn Ask`; hence one number method per type.
pub trait Ask {
    /// The default is what a careless Enter gives: `false` for anything
    /// irreversible.
    fn confirm(&mut self, question: Question<'_>, default: bool) -> AskResult<bool>;

    fn text(&mut self, question: Question<'_>, default: Option<&str>) -> AskResult<String>;

    /// Recipient address, parsed and checked here once for every interface,
    /// because a wrong address still makes a valid payment. Re-asks on bad
    /// input; the GUI overrides it (paste button, dragged image, USB search).
    fn address(&mut self, question: Question<'_>, network: Network) -> AskResult<Destination> {
        loop {
            let response = self.text(question, None)?;
            match crate::destination::interpret(&response, network) {
                Ok(parsed) => {
                    // Always echo what was parsed, in groups of four: nobody read the QR by
                    // eye, and a clipboard may hold something else.
                    if let Some(file) = &parsed.from_file {
                        crate::ui::note(&plebline_core::tf!(
                            { "letto dal QR in {}", file.display() },
                            { "read from the QR in {}", file.display() }
                        ));
                    }
                    crate::ui::note(&crate::destination::in_groups(
                        &parsed.destination.address.to_string(),
                    ));
                    return Ok(parsed.destination);
                }
                Err(reason) => crate::ui::warning(&reason),
            }
        }
    }

    /// Returns the index, not the label: no silent fallback to entry 0.
    fn select(
        &mut self,
        question: Question<'_>,
        options: &[&str],
        page_size: Option<usize>,
    ) -> AskResult<usize>;

    /// Masked, no confirmation: the settings for a passphrase.
    fn password(&mut self, question: Question<'_>) -> AskResult<String>;

    fn number_u32(&mut self, question: Question<'_>, default: Option<u32>) -> AskResult<u32>;
    fn number_u64(&mut self, question: Question<'_>, default: Option<u64>) -> AskResult<u64>;
    fn number_usize(&mut self, question: Question<'_>, default: Option<usize>) -> AskResult<usize>;
    fn number_f64(&mut self, question: Question<'_>, default: Option<f64>) -> AskResult<f64>;

    /// A bounded number: the window shows a slider. Bounds
    /// only draw the scale. `default: None` forces the user to choose (the
    /// batch fee must never be accepted by inattention).
    fn range_usize(
        &mut self,
        question: Question<'_>,
        range: std::ops::RangeInclusive<usize>,
        default: Option<usize>,
    ) -> AskResult<usize> {
        let _ = range;
        self.number_usize(question, default)
    }

    /// [`Ask::range_usize`] for decimals.
    fn range_f64(
        &mut self,
        question: Question<'_>,
        range: std::ops::RangeInclusive<f64>,
        default: Option<f64>,
    ) -> AskResult<f64> {
        let _ = range;
        self.number_f64(question, default)
    }

    /// Seed words, asked so that none stays on screen afterwards.
    fn seed(&mut self, count: usize) -> AskResult<SeedWords>;
}

/// A prepared answer. Typed, so a script that is off by one fails at the
/// question where it slips instead of quietly following another path.
#[derive(Debug, Clone)]
#[cfg(any(test, feature = "test-support"))]
pub enum Reply {
    /// Answer to a confirmation.
    Yes,
    No,
    /// Text, shown or masked.
    Text(String),
    /// An entry of the list, by index.
    Choice(usize),
    /// Matched by a unique piece of the label; zero or several matches fail.
    ChoiceLike(String),
    /// Text; if it does not parse, the question is re-asked, as on a terminal.
    Number(String),
    /// The seed words.
    Seed(String),
    /// Esc or Ctrl-C.
    Cancelled,
}

#[cfg(any(test, feature = "test-support"))]
impl Reply {
    pub fn text(value: impl Into<String>) -> Self {
        Reply::Text(value.into())
    }

    pub fn like(value: impl Into<String>) -> Self {
        Reply::ChoiceLike(value.into())
    }

    pub fn number(value: impl ToString) -> Self {
        Reply::Number(value.to_string())
    }

    pub fn seed(words: impl Into<String>) -> Self {
        Reply::Seed(words.into())
    }

    fn kind(&self) -> &'static str {
        match self {
            Reply::Yes | Reply::No => "una conferma",
            Reply::Text(_) => "un testo",
            Reply::Choice(_) | Reply::ChoiceLike(_) => "una scelta",
            Reply::Number(_) => "un numero",
            Reply::Seed(_) => "un seed",
            Reply::Cancelled => "un annullamento",
        }
    }
}

/// The type of question, kept in the record and named in mismatch messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(any(test, feature = "test-support"))]
pub enum Kind {
    Confirm,
    Text,
    Select,
    Password,
    Number,
    Seed,
}

#[cfg(any(test, feature = "test-support"))]
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Confirm => "una conferma",
            Kind::Text => "un testo",
            Kind::Select => "una scelta",
            Kind::Password => "una password",
            Kind::Number => "un numero",
            Kind::Seed => "un seed",
        }
    }
}

#[derive(Debug, Clone)]
#[cfg(any(test, feature = "test-support"))]
pub struct Asked {
    pub kind: Kind,
    pub prompt: String,
    pub options: Vec<String>,
}

/// Scripted answers, plus a record of what was asked, so tests can assert
/// the order of questions (e.g. unplug the network before signing).
#[cfg(any(test, feature = "test-support"))]
pub struct Scripted {
    replies: std::collections::VecDeque<Reply>,
    asked: Vec<Asked>,
}

#[cfg(any(test, feature = "test-support"))]
impl Scripted {
    pub fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            asked: Vec::new(),
        }
    }

    pub fn asked(&self) -> &[Asked] {
        &self.asked
    }

    /// Question texts, for assertions.
    pub fn prompts(&self) -> Vec<&str> {
        self.asked.iter().map(|a| a.prompt.as_str()).collect()
    }

    /// Zero at the end proves the script matched the path taken.
    pub fn remaining(&self) -> usize {
        self.replies.len()
    }

    fn record(&mut self, kind: Kind, prompt: &str, options: &[&str]) {
        self.asked.push(Asked {
            kind,
            prompt: prompt.to_string(),
            options: options.iter().map(|o| o.to_string()).collect(),
        });
    }

    /// Next answer, or a failure naming the uncovered question.
    fn next(&mut self, kind: Kind, prompt: &str) -> Reply {
        match self.replies.pop_front() {
            Some(reply) => reply,
            None => panic!(
                "il copione è finito, ma il flusso chiede ancora {}: «{prompt}»",
                kind.name()
            ),
        }
    }

    /// The entry a script picks, by index or by a piece of its label.
    fn chosen(prompt: &str, options: &[&str], reply: &Reply) -> Option<usize> {
        match reply {
            Reply::Choice(index) => {
                assert!(
                    *index < options.len(),
                    "il copione sceglie la voce {index} di «{prompt}», che ne ha {}",
                    options.len()
                );
                Some(*index)
            }
            Reply::ChoiceLike(needle) => {
                let found: Vec<usize> = options
                    .iter()
                    .enumerate()
                    .filter(|(_, option)| option.contains(needle.as_str()))
                    .map(|(index, _)| index)
                    .collect();
                match found.as_slice() {
                    [only] => Some(*only),
                    [] => {
                        panic!("«{needle}» non compare in nessuna voce di «{prompt}»: {options:?}")
                    }
                    many => panic!(
                        "«{needle}» compare in {} voci di «{prompt}»: {options:?}",
                        many.len()
                    ),
                }
            }
            Reply::Cancelled => None,
            other => Self::mismatch(Kind::Select, prompt, other),
        }
    }

    fn mismatch(kind: Kind, prompt: &str, reply: &Reply) -> ! {
        panic!(
            "il flusso chiede {} («{prompt}»), ma il copione aveva {}: {reply:?}",
            kind.name(),
            reply.kind()
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Ask for Scripted {
    fn confirm(&mut self, question: Question<'_>, _default: bool) -> AskResult<bool> {
        self.record(Kind::Confirm, question.prompt, &[]);
        match self.next(Kind::Confirm, question.prompt) {
            Reply::Yes => Ok(true),
            Reply::No => Ok(false),
            Reply::Cancelled => Err(AskError::Cancelled),
            other => Self::mismatch(Kind::Confirm, question.prompt, &other),
        }
    }

    fn text(&mut self, question: Question<'_>, default: Option<&str>) -> AskResult<String> {
        self.record(Kind::Text, question.prompt, &[]);
        match self.next(Kind::Text, question.prompt) {
            // Enter means the default.
            Reply::Text(value) if value.is_empty() => Ok(default.unwrap_or_default().to_string()),
            Reply::Text(value) => Ok(value),
            Reply::Cancelled => Err(AskError::Cancelled),
            other => Self::mismatch(Kind::Text, question.prompt, &other),
        }
    }

    fn select(
        &mut self,
        question: Question<'_>,
        options: &[&str],
        _page_size: Option<usize>,
    ) -> AskResult<usize> {
        self.record(Kind::Select, question.prompt, options);
        let reply = self.next(Kind::Select, question.prompt);
        match Self::chosen(question.prompt, options, &reply) {
            Some(index) => Ok(index),
            None => Err(AskError::Cancelled),
        }
    }

    fn password(&mut self, question: Question<'_>) -> AskResult<String> {
        self.record(Kind::Password, question.prompt, &[]);
        match self.next(Kind::Password, question.prompt) {
            Reply::Text(value) => Ok(value),
            Reply::Cancelled => Err(AskError::Cancelled),
            other => Self::mismatch(Kind::Password, question.prompt, &other),
        }
    }

    fn number_u32(&mut self, question: Question<'_>, default: Option<u32>) -> AskResult<u32> {
        self.scripted_number(question, default)
    }

    fn number_u64(&mut self, question: Question<'_>, default: Option<u64>) -> AskResult<u64> {
        self.scripted_number(question, default)
    }

    fn number_usize(&mut self, question: Question<'_>, default: Option<usize>) -> AskResult<usize> {
        self.scripted_number(question, default)
    }

    fn number_f64(&mut self, question: Question<'_>, default: Option<f64>) -> AskResult<f64> {
        self.scripted_number(question, default)
    }

    fn seed(&mut self, count: usize) -> AskResult<SeedWords> {
        self.record(Kind::Seed, "seed", &[]);
        match self.next(Kind::Seed, "seed") {
            Reply::Seed(words) => {
                let found = words.split_whitespace().count();
                assert_eq!(
                    found, count,
                    "il copione dà {found} parole, il flusso ne ha chieste {count}"
                );
                Ok(SeedWords::from_interactive_input(&words)
                    .expect("il copione dà un numero di parole valido"))
            }
            Reply::Cancelled => Err(AskError::Cancelled),
            other => Self::mismatch(Kind::Seed, "seed", &other),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Scripted {
    /// Parses the text answer; re-asks on failure.
    fn scripted_number<T>(&mut self, question: Question<'_>, default: Option<T>) -> AskResult<T>
    where
        T: Clone + std::str::FromStr,
    {
        loop {
            self.record(Kind::Number, question.prompt, &[]);
            match self.next(Kind::Number, question.prompt) {
                Reply::Number(written) if written.is_empty() => {
                    match default.clone() {
                        Some(value) => return Ok(value),
                        // Enter with no default: asked again.
                        None => continue,
                    }
                }
                Reply::Number(written) => match written.trim().parse::<T>() {
                    Ok(value) => return Ok(value),
                    Err(_) => continue,
                },
                Reply::Cancelled => return Err(AskError::Cancelled),
                other => Self::mismatch(Kind::Number, question.prompt, &other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BIP39 test vector; no funds anywhere.
    const SEED: &str = "abandon abandon abandon abandon abandon abandon \
                        abandon abandon abandon abandon abandon about";

    fn q(prompt: &str) -> Question<'_> {
        Question::new(prompt)
    }

    #[test]
    fn the_answers_arrive_in_the_scripts_order() {
        let mut ask = Scripted::new([Reply::Yes, Reply::text("cassaforte"), Reply::number(3u32)]);
        assert!(ask.confirm(q("Procedo?"), false).unwrap());
        assert_eq!(ask.text(q("Come lo chiami?"), None).unwrap(), "cassaforte");
        assert_eq!(ask.number_u32(q("Quanti?"), None).unwrap(), 3);
        assert_eq!(ask.remaining(), 0);
    }

    #[test]
    fn the_choice_returns_the_index_not_the_text() {
        let mut ask = Scripted::new([Reply::like("nuovo")]);
        let options = ["Mandarne uno in rete", "Preparare un gruppo nuovo"];
        assert_eq!(ask.select(q("Cosa vuoi fare?"), &options, None).unwrap(), 1);
    }

    #[test]
    #[should_panic(expected = "non compare in nessuna voce")]
    fn a_choice_that_does_not_exist_stops_the_test() {
        let mut ask = Scripted::new([Reply::like("dividere")]);
        let _ = ask.select(q("Cosa vuoi fare?"), &["mandare", "preparare"], None);
    }

    #[test]
    #[should_panic(expected = "compare in 2 voci")]
    fn an_ambiguous_choice_stops_the_test() {
        let mut ask = Scripted::new([Reply::like("nodo")]);
        let _ = ask.select(
            q("A chi chiedo?"),
            &["il mio nodo", "un nodo pubblico"],
            None,
        );
    }

    #[test]
    #[should_panic(expected = "ma il copione aveva un testo")]
    fn a_script_out_of_step_stops_the_test() {
        let mut ask = Scripted::new([Reply::text("sì")]);
        let _ = ask.confirm(q("Trasmetto questa transazione?"), false);
    }

    #[test]
    #[should_panic(expected = "il copione è finito")]
    fn a_script_too_short_stops_the_test() {
        let mut ask = Scripted::new([]);
        let _ = ask.confirm(q("Procedo?"), false);
    }

    #[test]
    fn cancelling_is_distinct_from_a_failure() {
        let mut ask = Scripted::new([Reply::Cancelled]);
        assert!(matches!(
            ask.confirm(q("Procedo?"), false),
            Err(AskError::Cancelled)
        ));
    }

    #[test]
    fn enter_on_a_question_with_a_default_means_the_default() {
        let mut ask = Scripted::new([Reply::text(""), Reply::number("")]);
        assert_eq!(
            ask.text(q("Come lo chiami?"), Some("cerimonia")).unwrap(),
            "cerimonia"
        );
        assert_eq!(ask.number_u32(q("Quanti indirizzi?"), Some(3)).unwrap(), 3);
    }

    #[test]
    fn an_unreadable_number_makes_the_question_repeat() {
        let mut ask = Scripted::new([Reply::number("due"), Reply::number(2u64)]);
        assert_eq!(ask.number_u64(q("Importo in sat:"), None).unwrap(), 2);
        assert_eq!(ask.prompts(), ["Importo in sat:", "Importo in sat:"]);
    }

    #[test]
    fn the_record_keeps_the_order_and_the_entries() {
        let mut ask = Scripted::new([Reply::Choice(0), Reply::Yes]);
        let _ = ask.select(q("A chi chiedo?"), &["pubblico", "il mio nodo"], None);
        let _ = ask.confirm(q("Procedo?"), true);

        let asked = ask.asked();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].kind, Kind::Select);
        assert_eq!(asked[0].options, ["pubblico", "il mio nodo"]);
        assert_eq!(asked[1].kind, Kind::Confirm);
        assert_eq!(ask.prompts(), ["A chi chiedo?", "Procedo?"]);
    }

    #[test]
    fn the_fake_seed_is_a_real_seed() {
        let mut ask = Scripted::new([Reply::seed(SEED)]);
        assert!(ask.seed(12).is_ok());
    }

    #[test]
    #[should_panic(expected = "il copione dà 12 parole, il flusso ne ha chieste 24")]
    fn a_seed_of_the_wrong_length_stops_the_test() {
        let mut ask = Scripted::new([Reply::seed(SEED)]);
        let _ = ask.seed(24);
    }

    #[test]
    fn cancelling_seed_entry_is_not_an_error() {
        let mut ask = Scripted::new([Reply::Cancelled]);
        assert!(matches!(ask.seed(12), Err(AskError::Cancelled)));
    }

    const TB: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    /// A wrong paste re-asks instead of ending the ceremony.
    #[test]
    fn a_wrong_address_makes_the_question_repeat() {
        let mut ask = Scripted::new([
            Reply::text("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"), // mainnet
            Reply::text("questo non è niente"),
            Reply::text(TB),
        ]);
        let parsed = ask
            .address(Question::new("A chi?"), Network::Signet)
            .unwrap();
        assert_eq!(parsed.address.to_string(), TB);
        assert_eq!(ask.remaining(), 0, "il copione deve essere stato consumato");
        assert_eq!(ask.prompts(), vec!["A chi?", "A chi?", "A chi?"]);
    }

    #[test]
    fn a_qr_uri_carries_address_and_amount() {
        let mut ask = Scripted::new([Reply::text(format!("bitcoin:{TB}?amount=0.001"))]);
        let parsed = ask
            .address(Question::new("A chi?"), Network::Signet)
            .unwrap();
        assert_eq!(parsed.address.to_string(), TB);
        assert_eq!(parsed.amount_sat, Some(100_000));
    }
}
