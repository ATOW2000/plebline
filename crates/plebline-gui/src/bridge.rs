//! Bridge between the ceremony and the window. The ceremony runs on its own
//! thread and blocks on `recv()` at each question; the drawing thread never
//! blocks. To `run` this is just another [`Ask`].
//!
//! [`SeedWords`] crosses once, by move (not `Clone`, zeroed on drop).

use std::sync::mpsc::{Receiver, Sender};

use plebline::ask::{Ask, AskError, AskResult, Question};
use plebline::out::{Line, Place, Sink};
use plebline_core::address::Destination;
use plebline_core::bitcoin::Network;
use plebline_core::wallet::SeedWords;

/// An owned question: `plebline::ask::Question` borrows, and this must
/// cross threads.
#[derive(Clone)]
pub struct Prompt {
    pub text: String,
    pub help: Option<String>,
    /// Which input the window shows for the answer.
    pub shape: Shape,
}

#[derive(Clone)]
pub enum Shape {
    /// Yes/no; the value is the default (`false` for irreversible actions).
    Confirm(bool),
    /// Free text, with the starting value if there is one.
    Text(Option<String>),
    /// Text that is not seen while being typed.
    Password,
    /// Recipient address, with the network to validate it against while typing.
    Address(Network),
    /// A number; with `scale`, drawn as a slider.
    Number {
        default: Option<String>,
        kind: Num,
        invalid: Option<String>,
        scale: Option<Scale>,
    },
    /// An entry from a list. The answer is the index.
    Select(Vec<String>),
    /// The seed words.
    Seed { count: usize },
}

/// Bounds, for drawing the slider only: values outside are still accepted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub min: f64,
    pub max: f64,
    /// Where the handle sits when the question opens.
    pub start: f64,
    /// The start value is not an answer until moved (the batch fee).
    pub required: bool,
}

impl Scale {
    /// Useful values crowd at the low end (fees: 1–5 sat/vB matter, 150–154
    /// do not).
    pub fn logarithmic(self) -> bool {
        self.min > 0.0 && self.max / self.min >= 50.0
    }
}

/// Kind of number, so the window rejects bad input immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Num {
    U32,
    U64,
    Usize,
    F64,
}

impl Num {
    pub fn accepts(self, text: &str) -> bool {
        let text = text.trim();
        match self {
            Num::U32 => text.parse::<u32>().is_ok(),
            Num::U64 => text.parse::<u64>().is_ok(),
            Num::Usize => text.parse::<usize>().is_ok(),
            Num::F64 => text.parse::<f64>().is_ok(),
        }
    }
}

/// Messages from the ceremony thread to the window.
pub enum FromCeremony {
    /// A line to show.
    Line(Box<Line>),
    /// The step line, the network state, and whether the words are in memory.
    Place(Box<Place>),
    /// A question: the ceremony blocks until answered.
    Ask(Box<Prompt>),
    /// `None` on success; otherwise the message, already sent as lines via
    /// `report`.
    Done(Option<String>),
}

/// Answers from the window to the ceremony thread.
pub enum Answer {
    Yes(bool),
    Text(String),
    Index(usize),
    /// By move. `None` is a change of mind (see [`Ask::seed`]).
    Seed(Option<SeedWords>),
    /// Esc, or the window's close button.
    Cancelled,
}

/// Lives on the ceremony thread; the egui `Context` wakes the drawing
/// thread so the question appears at once.
pub struct Window {
    to_ui: Sender<FromCeremony>,
    from_ui: Receiver<Answer>,
    ctx: eframe::egui::Context,
}

impl Window {
    pub fn new(
        to_ui: Sender<FromCeremony>,
        from_ui: Receiver<Answer>,
        ctx: eframe::egui::Context,
    ) -> Self {
        Self {
            to_ui,
            from_ui,
            ctx,
        }
    }

    /// A closed channel means the window is gone. That is reported as
    /// `Unavailable`, so the ceremony does not take it for Esc.
    fn exchange(&mut self, prompt: Prompt) -> AskResult<Answer> {
        let closed = || {
            AskError::Unavailable(
                plebline_core::lang::t("la finestra è stata chiusa", "the window was closed")
                    .to_string(),
            )
        };
        self.to_ui
            .send(FromCeremony::Ask(Box::new(prompt)))
            .map_err(|_| closed())?;
        self.ctx.request_repaint();
        self.from_ui.recv().map_err(|_| closed())
    }

    fn prompt(question: Question<'_>, shape: Shape) -> Prompt {
        Prompt {
            text: question.prompt.to_string(),
            help: question.help.map(str::to_string),
            shape,
        }
    }

    fn text_answer(&mut self, prompt: Prompt) -> AskResult<String> {
        match self.exchange(prompt)? {
            Answer::Text(text) => Ok(text),
            Answer::Cancelled => Err(AskError::Cancelled),
            _ => Err(AskError::Unavailable(answer_out_of_turn())),
        }
    }

    /// The window already checks the number. Parsing it again here turns a
    /// window bug into an error instead of a made-up value.
    fn number<T: std::str::FromStr>(&mut self, prompt: Prompt) -> AskResult<T> {
        let text = self.text_answer(prompt)?;
        text.trim().parse::<T>().map_err(|_| {
            AskError::Unavailable(
                plebline_core::lang::t(
                    "la finestra ha mandato una risposta che non è un numero",
                    "the window sent an answer that is not a number",
                )
                .to_string(),
            )
        })
    }
}

fn answer_out_of_turn() -> String {
    plebline_core::lang::t(
        "la finestra ha risposto a un'altra domanda",
        "the window answered a different question",
    )
    .to_string()
}

impl Ask for Window {
    fn confirm(&mut self, question: Question<'_>, default: bool) -> AskResult<bool> {
        match self.exchange(Self::prompt(question, Shape::Confirm(default)))? {
            Answer::Yes(value) => Ok(value),
            Answer::Cancelled => Err(AskError::Cancelled),
            _ => Err(AskError::Unavailable(answer_out_of_turn())),
        }
    }

    fn text(&mut self, question: Question<'_>, default: Option<&str>) -> AskResult<String> {
        let shape = Shape::Text(default.map(str::to_string));
        self.text_answer(Self::prompt(question, shape))
    }

    /// Validated again on this side, so a bug in the window cannot pass an
    /// unchecked address on to a payment.
    fn address(&mut self, question: Question<'_>, network: Network) -> AskResult<Destination> {
        let response = self.text_answer(Self::prompt(question, Shape::Address(network)))?;
        match plebline::destination::interpret(&response, network) {
            Ok(parsed) => Ok(parsed.destination),
            Err(reason) => Err(AskError::Unavailable(plebline_core::tf!(
                { "la finestra ha mandato un indirizzo che non è valido: {reason}" },
                { "the window sent an address that is not valid: {reason}" }
            ))),
        }
    }

    fn select(
        &mut self,
        question: Question<'_>,
        options: &[&str],
        // A window scrolls; no paging.
        _page_size: Option<usize>,
    ) -> AskResult<usize> {
        let shape = Shape::Select(options.iter().map(|o| o.to_string()).collect());
        match self.exchange(Self::prompt(question, shape))? {
            Answer::Index(index) if index < options.len() => Ok(index),
            // An out-of-range index never becomes the first entry.
            Answer::Index(_) => Err(AskError::Unavailable(answer_out_of_turn())),
            Answer::Cancelled => Err(AskError::Cancelled),
            _ => Err(AskError::Unavailable(answer_out_of_turn())),
        }
    }

    fn password(&mut self, question: Question<'_>) -> AskResult<String> {
        self.text_answer(Self::prompt(question, Shape::Password))
    }

    fn number_u32(&mut self, question: Question<'_>, default: Option<u32>) -> AskResult<u32> {
        let shape = Shape::Number {
            default: default.map(|v| v.to_string()),
            kind: Num::U32,
            invalid: question.invalid.map(str::to_string),
            scale: None,
        };
        self.number(Self::prompt(question, shape))
    }

    fn number_u64(&mut self, question: Question<'_>, default: Option<u64>) -> AskResult<u64> {
        let shape = Shape::Number {
            default: default.map(|v| v.to_string()),
            kind: Num::U64,
            invalid: question.invalid.map(str::to_string),
            scale: None,
        };
        self.number(Self::prompt(question, shape))
    }

    fn number_usize(&mut self, question: Question<'_>, default: Option<usize>) -> AskResult<usize> {
        let shape = Shape::Number {
            default: default.map(|v| v.to_string()),
            kind: Num::Usize,
            invalid: question.invalid.map(str::to_string),
            scale: None,
        };
        self.number(Self::prompt(question, shape))
    }

    fn number_f64(&mut self, question: Question<'_>, default: Option<f64>) -> AskResult<f64> {
        let shape = Shape::Number {
            default: default.map(|v| v.to_string()),
            kind: Num::F64,
            invalid: question.invalid.map(str::to_string),
            scale: None,
        };
        self.number(Self::prompt(question, shape))
    }

    fn range_usize(
        &mut self,
        question: Question<'_>,
        range: std::ops::RangeInclusive<usize>,
        default: Option<usize>,
    ) -> AskResult<usize> {
        let scale = Scale {
            min: *range.start() as f64,
            max: *range.end() as f64,
            start: default.unwrap_or(*range.start()) as f64,
            required: default.is_none(),
        };
        let shape = Shape::Number {
            default: default.map(|v| v.to_string()),
            kind: Num::Usize,
            invalid: question.invalid.map(str::to_string),
            scale: Some(scale),
        };
        self.number(Self::prompt(question, shape))
    }

    fn range_f64(
        &mut self,
        question: Question<'_>,
        range: std::ops::RangeInclusive<f64>,
        default: Option<f64>,
    ) -> AskResult<f64> {
        let scale = Scale {
            min: *range.start(),
            max: *range.end(),
            start: default.unwrap_or(*range.start()),
            required: default.is_none(),
        };
        let shape = Shape::Number {
            default: default.map(|v| v.to_string()),
            kind: Num::F64,
            invalid: question.invalid.map(str::to_string),
            scale: Some(scale),
        };
        self.number(Self::prompt(question, shape))
    }

    fn seed(&mut self, count: usize) -> AskResult<SeedWords> {
        let prompt = Prompt {
            text: plebline_core::lang::t("Le tue parole", "Your words").to_string(),
            help: None,
            shape: Shape::Seed { count },
        };
        match self.exchange(prompt)? {
            Answer::Seed(Some(words)) => Ok(words),
            Answer::Seed(None) | Answer::Cancelled => Err(AskError::Cancelled),
            _ => Err(AskError::Unavailable(answer_out_of_turn())),
        }
    }
}

/// The window's sink. A closed channel is ignored: the ceremony notices at
/// its next question.
pub struct ToWindow {
    tx: Sender<FromCeremony>,
    ctx: eframe::egui::Context,
}

impl ToWindow {
    pub fn new(tx: Sender<FromCeremony>, ctx: eframe::egui::Context) -> Self {
        Self { tx, ctx }
    }
}

impl Sink for ToWindow {
    fn line(&mut self, line: Line) {
        if self.tx.send(FromCeremony::Line(Box::new(line))).is_ok() {
            self.ctx.request_repaint();
        }
    }

    fn place(&mut self, place: &Place) {
        if self
            .tx
            .send(FromCeremony::Place(Box::new(place.clone())))
            .is_ok()
        {
            self.ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn an_integer_accepts_neither_comma_nor_sign() {
        assert!(Num::U64.accepts(" 2100 "));
        assert!(!Num::U64.accepts("2,1"));
        assert!(!Num::U64.accepts("-1"));
        assert!(Num::F64.accepts("2.5"));
        assert!(!Num::F64.accepts("due"));
    }

    /// A fake window answering `answer`; also returns what was asked.
    fn answering_with(answer: Answer) -> (Window, std::sync::mpsc::Receiver<FromCeremony>) {
        let (to_ui, from_ceremony) = channel();
        let (to_ceremony, from_ui) = channel();
        to_ceremony.send(answer).expect("la risposta parte");
        (
            Window::new(to_ui, from_ui, eframe::egui::Context::default()),
            from_ceremony,
        )
    }

    #[test]
    fn the_question_reaches_the_window_with_text_and_help() {
        let (mut window, mail) = answering_with(Answer::Text("ciao".into()));
        let question = Question::new("Come ti chiami?").help("serve solo a me");
        assert_eq!(window.text(question, None).unwrap(), "ciao");

        let FromCeremony::Ask(prompt) = mail.recv().unwrap() else {
            panic!("la finestra non ha ricevuto una domanda");
        };
        assert_eq!(prompt.text, "Come ti chiami?");
        assert_eq!(prompt.help.as_deref(), Some("serve solo a me"));
    }

    fn scale_of(mail: &std::sync::mpsc::Receiver<FromCeremony>) -> Scale {
        let FromCeremony::Ask(prompt) = mail.recv().unwrap() else {
            panic!("la finestra non ha ricevuto una domanda");
        };
        let Shape::Number { scale, .. } = prompt.shape else {
            panic!("non era una domanda numerica");
        };
        scale.expect("la domanda doveva portarsi dietro la scala")
    }

    #[test]
    fn a_bounded_count_arrives_as_a_scale_already_set() {
        let (mut window, mail) = answering_with(Answer::Text("5".into()));
        assert_eq!(
            window
                .range_usize(Question::new("quante ricariche?"), 1..=12, Some(4))
                .unwrap(),
            5
        );

        let scale = scale_of(&mail);
        assert_eq!((scale.min, scale.max, scale.start), (1.0, 12.0, 4.0));
        // With a start value, Next works without moving the slider.
        assert!(!scale.required);
        assert!(
            !scale.logarithmic(),
            "dodici ricariche stanno su una lineare"
        );
    }

    #[test]
    fn a_fee_with_no_default_arrives_required_and_logarithmic() {
        let (mut window, mail) = answering_with(Answer::Text("7.50".into()));
        assert_eq!(
            window
                .range_f64(Question::new("fee?"), 1.0..=500.0, None)
                .unwrap(),
            7.5
        );

        let scale = scale_of(&mail);
        // No default: the handle must be moved (batch fee).
        assert!(scale.required);
        assert!(
            scale.logarithmic(),
            "fra 1 e 5 sat/vB c'è la differenza che conta"
        );
    }

    const TB: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    #[test]
    fn the_address_reaches_the_window_with_the_network_to_check() {
        let (mut window, mail) = answering_with(Answer::Text(TB.into()));
        let parsed = window
            .address(Question::new("A chi?"), Network::Signet)
            .unwrap();
        assert_eq!(parsed.address.to_string(), TB);

        let FromCeremony::Ask(prompt) = mail.recv().unwrap() else {
            panic!("la finestra non ha ricevuto una domanda");
        };
        // The network travels with the question.
        assert!(matches!(prompt.shape, Shape::Address(Network::Signet)));
    }

    #[test]
    fn the_qr_also_carries_the_amount_and_it_stays_a_suggestion() {
        let (mut window, _mail) =
            answering_with(Answer::Text(format!("bitcoin:{TB}?amount=0.001")));
        let parsed = window
            .address(Question::new("A chi?"), Network::Signet)
            .unwrap();
        assert_eq!(parsed.amount_sat, Some(100_000));
    }

    #[test]
    fn an_invalid_address_from_the_window_is_a_failure() {
        let (mut window, _mail) = answering_with(Answer::Text("non un indirizzo".into()));
        let outcome = window.address(Question::new("A chi?"), Network::Signet);
        assert!(matches!(outcome, Err(AskError::Unavailable(_))));
    }

    #[test]
    fn an_address_from_another_network_does_not_pass() {
        let mainnet = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        let (mut window, _mail) = answering_with(Answer::Text(mainnet.into()));
        let outcome = window.address(Question::new("A chi?"), Network::Signet);
        assert!(matches!(outcome, Err(AskError::Unavailable(_))));
    }

    #[test]
    fn the_choice_comes_back_as_an_index() {
        let (mut window, _mail) = answering_with(Answer::Index(2));
        let chosen = window
            .select(Question::new("quale?"), &["a", "b", "c"], None)
            .unwrap();
        assert_eq!(chosen, 2);
    }

    #[test]
    fn an_index_outside_the_list_is_a_failure_not_the_first_entry() {
        let (mut window, _mail) = answering_with(Answer::Index(9));
        let outcome = window.select(Question::new("quale?"), &["a", "b"], None);
        assert!(matches!(outcome, Err(AskError::Unavailable(_))));
    }

    #[test]
    fn an_answer_that_is_not_a_number_does_not_become_zero() {
        let (mut window, _mail) = answering_with(Answer::Text("parecchio".into()));
        let outcome = window.number_u64(Question::new("quanti sat di fee?"), Some(10));
        assert!(matches!(outcome, Err(AskError::Unavailable(_))));
    }

    #[test]
    fn an_answer_of_another_kind_does_not_pass() {
        let (mut window, _mail) = answering_with(Answer::Yes(true));
        let outcome = window.text(Question::new("come ti chiami?"), None);
        assert!(matches!(outcome, Err(AskError::Unavailable(_))));
    }

    #[test]
    fn cancelling_is_cancelling_and_not_a_failure() {
        let (mut window, _mail) = answering_with(Answer::Cancelled);
        assert!(matches!(
            window.confirm(Question::new("sicuro?"), false),
            Err(AskError::Cancelled)
        ));
    }

    /// `seed` cancels like every other question.
    #[test]
    fn cancelling_seed_entry_is_not_an_error() {
        let (mut window, _mail) = answering_with(Answer::Cancelled);
        assert!(matches!(window.seed(24), Err(AskError::Cancelled)));
    }

    #[test]
    fn a_closed_window_is_a_failure_not_a_cancellation() {
        let (to_ui, _mail) = channel();
        let (to_ceremony, from_ui) = channel();
        drop(to_ceremony);
        let mut window = Window::new(to_ui, from_ui, eframe::egui::Context::default());
        assert!(matches!(
            window.confirm(Question::new("sicuro?"), false),
            Err(AskError::Unavailable(_))
        ));
    }

    #[test]
    fn lines_arrive_and_a_closed_channel_does_not_panic() {
        let (tx, mail) = channel();
        let mut sink = ToWindow::new(tx, eframe::egui::Context::default());
        sink.line(Line::new(plebline::out::Kind::Success, "fatto"));
        let FromCeremony::Line(line) = mail.recv().unwrap() else {
            panic!("non è arrivata una riga");
        };
        assert_eq!(line.text, "fatto");

        drop(mail);
        sink.line(Line::new(plebline::out::Kind::Info, "e questa si perde"));
    }

    /// The step line and the link state arrive unchanged.
    #[test]
    fn the_place_arrives_whole() {
        let (tx, mail) = channel();
        let mut sink = ToWindow::new(tx, eframe::egui::Context::default());
        let place = Place {
            road: vec![plebline::out::Stop {
                label: "Firma".into(),
                home: false,
            }],
            here: 0,
            link: plebline::out::Link::Unplugged,
            words: true,
            cancel: plebline::out::Cancel::Home,
        };
        sink.place(&place);
        let FromCeremony::Place(arrived) = mail.recv().unwrap() else {
            panic!("non è arrivato il punto");
        };
        assert_eq!(*arrived, place);
    }
}
