//! The window's state: the transcript received from the ceremony, the open
//! question with the answer being typed, and the step line's last position.
//! Drawing is in `layout`, `draw` and `answers`.
//!
//! Seed words are masked by default and can be shown. Text fields are
//! zeroed once the seed is handed over (also on cancel and close). egui's
//! undo history keeps copies, and its glyph cache is outside our control.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use eframe::egui;
use plebline::out::{Kind, Line, Mono, Place, Status};
use zeroize::Zeroize;

use crate::bridge::{Answer, FromCeremony, Prompt, Shape};

mod answers;
mod draw;
mod layout;

/// The answer being written. Holds seed words and passphrase, so it zeroes
/// itself on drop.
struct Draft {
    text: String,
    words: Vec<String>,
    choice: usize,
    masked: bool,
    /// Why the last address read failed; kept until the next try.
    error: Option<String>,
    /// Images found on sticks; `None` until a search is made, while an empty
    /// list means none were found.
    found: Option<Vec<PathBuf>>,
    /// File the address was read from, shown because nobody checked that QR
    /// by eye.
    read_from: Option<PathBuf>,
    /// Slider position; not secret, so not zeroed.
    number: f64,
    /// Set once the user moves the slider; a `required` scale needs it.
    touched: bool,
}

impl Draft {
    /// No derived `Default`: `Drop` forbids `..Self::default()`, and a type
    /// that zeroes secrets should list every field anyway.
    fn empty() -> Self {
        Self {
            text: String::new(),
            words: Vec::new(),
            choice: 0,
            // Masked at first, since screenshots, screen sharing or recording
            // apps can capture a window. Showing the words takes one click.
            masked: true,
            error: None,
            found: None,
            read_from: None,
            number: 0.0,
            touched: false,
        }
    }

    fn for_shape(shape: &Shape) -> Self {
        let mut draft = Self::empty();
        match shape {
            Shape::Text(Some(default)) => draft.text = default.clone(),
            Shape::Number { default, scale, .. } => {
                draft.text = default.clone().unwrap_or_default();
                if let Some(scale) = scale {
                    draft.number = scale.start;
                }
            }
            Shape::Seed { count } => draft.words = vec![String::new(); *count],
            _ => {}
        }
        draft
    }
}

impl Drop for Draft {
    fn drop(&mut self) {
        self.text.zeroize();
        for word in &mut self.words {
            word.zeroize();
        }
    }
}

enum Outcome {
    Running,
    Finished,
    /// Stopped; the message is already in the transcript.
    Stopped,
}

pub struct App {
    transcript: Vec<Line>,
    question: Option<(Prompt, Draft)>,
    outcome: Outcome,
    from_ceremony: Receiver<FromCeremony>,
    to_ceremony: Sender<Answer>,
    /// Start of the section the box shows: the last title, unless the
    /// section before it was neither answered nor worked in.
    section: usize,
    /// A question came since `section` began.
    asked: bool,
    /// Width the box took last frame: a wide answer (the words) can widen it.
    box_drawn: f32,
    /// Height the question took last frame, kept clear under the section.
    prompt_h: f32,
    /// Header: the network's name, and whether it is a test network.
    header: (String, bool),
    /// Last position reported by the ceremony.
    place: Option<Place>,
}

impl App {
    pub fn new(
        from_ceremony: Receiver<FromCeremony>,
        to_ceremony: Sender<Answer>,
        network: &str,
        test_network: bool,
    ) -> Self {
        Self {
            transcript: Vec::new(),
            question: None,
            outcome: Outcome::Running,
            from_ceremony,
            to_ceremony,
            section: 0,
            asked: false,
            box_drawn: 0.0,
            prompt_h: 0.0,
            header: (network.to_string(), test_network),
            place: None,
        }
    }

    /// Index of the open status light that a `Done` or `Stopped` line
    /// replaces: the last one still open. With none open, the line is
    /// appended instead.
    fn light_to_settle(&self, kind: Kind) -> Option<usize> {
        if !matches!(kind, Kind::Status(Status::Done | Status::Stopped)) {
            return None;
        }
        self.transcript
            .iter()
            .rposition(|line| matches!(line.kind, Kind::Status(status) if status.open()))
    }

    /// Start of the block a retry replaces (status light, trace, error,
    /// remedy), only if nothing else came after it; otherwise the retry is
    /// appended.
    fn block_to_replace(&self, kind: Kind) -> Option<usize> {
        if kind != Kind::Status(Status::Retrying) {
            return None;
        }
        let start = self
            .transcript
            .iter()
            .rposition(|line| matches!(line.kind, Kind::Status(_)))?;
        // Only a finished status light; one still open is never erased.
        if !matches!(self.transcript[start].kind, Kind::Status(status) if !status.open()) {
            return None;
        }
        let belongs =
            |kind: Kind| matches!(kind, Kind::Trace | Kind::Error | Kind::Remedy | Kind::Blank);
        self.transcript[start + 1..]
            .iter()
            .all(|line| belongs(line.kind))
            .then_some(start)
    }

    fn drain(&mut self) {
        loop {
            match self.from_ceremony.try_recv() {
                Ok(FromCeremony::Line(line)) => {
                    // Artwork is for a terminal: the window has its own header.
                    if line.kind != Kind::Mono(Mono::Art) {
                        // A title opens a new section once the last one had a question or
                        // some work in it. Without either it is still unread, so it stays on top.
                        if line.kind == Kind::Title && self.section_done() {
                            self.section = self.transcript.len();
                            self.asked = false;
                        }
                        // A finished status replaces the working one; its trace stays under it.
                        if let Some(spot) = self.light_to_settle(line.kind) {
                            self.transcript[spot] = *line;
                        } else if let Some(spot) = self.block_to_replace(line.kind) {
                            // Same attempt again: drop the failed block.
                            self.transcript.truncate(spot);
                            self.transcript.push(*line);
                        } else {
                            self.transcript.push(*line);
                        }
                        self.section = self.section.min(self.transcript.len());
                    }
                }
                Ok(FromCeremony::Place(place)) => self.place = Some(*place),
                Ok(FromCeremony::Ask(prompt)) => {
                    self.asked = true;
                    let draft = Draft::for_shape(&prompt.shape);
                    self.question = Some((*prompt, draft));
                }
                Ok(FromCeremony::Done(error)) => {
                    self.question = None;
                    self.outcome = if error.is_some() {
                        Outcome::Stopped
                    } else {
                        Outcome::Finished
                    };
                }
                Err(TryRecvError::Empty) => break,
                // The ceremony thread ended without `Done`: stop waiting.
                Err(TryRecvError::Disconnected) => {
                    if matches!(self.outcome, Outcome::Running) {
                        self.outcome = Outcome::Stopped;
                    }
                    break;
                }
            }
        }
    }

    fn section_done(&self) -> bool {
        let start = self.section.min(self.transcript.len());
        self.asked
            || self.transcript[start..]
                .iter()
                .any(|line| matches!(line.kind, Kind::Step | Kind::Status(_)))
    }

    fn reply(&mut self, answer: Answer) {
        // Dropping the `Draft` zeroes it.
        self.question = None;
        let _ = self.to_ceremony.send(answer);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain();
        egui::CentralPanel::no_frame().show(ui, |ui| self.stage(ui));
    }

    /// Closing drops the channel; the ceremony stops at its next question,
    /// leaving files in place.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.question = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// A new window and the ceremony's end of the channel.
    fn window() -> (App, Sender<FromCeremony>) {
        let (ceremony, from_ceremony) = channel();
        let (to_ceremony, _answers) = channel();
        (
            App::new(from_ceremony, to_ceremony, "signet", true),
            ceremony,
        )
    }

    fn send(ceremony: &Sender<FromCeremony>, kind: Kind, text: &str) {
        let _ = ceremony.send(FromCeremony::Line(Box::new(Line::new(kind, text))));
    }

    fn ask(ceremony: &Sender<FromCeremony>) {
        let _ = ceremony.send(FromCeremony::Ask(Box::new(Prompt {
            text: "Andiamo?".into(),
            help: None,
            shape: Shape::Confirm(true),
        })));
    }

    #[test]
    fn a_title_after_an_answer_opens_a_new_section() {
        let (mut app, ceremony) = window();
        send(&ceremony, Kind::Note, "un passo qualsiasi");
        ask(&ceremony);
        app.drain();
        assert_eq!(
            app.section, 0,
            "senza titoli il riquadro mostra dall'inizio"
        );

        send(&ceremony, Kind::Title, "Cos'è PlebLine");
        send(&ceremony, Kind::Note, "la prima riga del discorso");
        send(
            &ceremony,
            Kind::Note,
            "e tutto il resto, che non ci sta nella finestra",
        );
        app.drain();

        assert_eq!(app.section, 1, "il riquadro parte dal titolo");
    }

    /// Two sections with no question between: the first was never read, so
    /// it stays on top and the second follows it.
    #[test]
    fn an_unread_section_stays_above_the_next() {
        let (mut app, ceremony) = window();
        send(&ceremony, Kind::Title, "Cos'è PlebLine");
        send(&ceremony, Kind::Note, "due parole");
        send(&ceremony, Kind::Title, "Il tuo portafoglio");
        app.drain();

        assert_eq!(app.section, 0);
    }

    /// A section with a step in it was watched while it ran, so the next
    /// title replaces it.
    #[test]
    fn a_section_with_work_in_it_is_replaced_by_the_next() {
        let (mut app, ceremony) = window();
        send(&ceremony, Kind::Title, "Guardo cosa hai");
        send(&ceremony, Kind::Step, "Leggo la catena");
        send(&ceremony, Kind::Title, "Il tuo portafoglio");
        app.drain();

        assert_eq!(app.section, 2);
    }

    /// The banner is skipped and does not shift indexes.
    #[test]
    fn the_opening_art_does_not_count_in_the_transcript() {
        let (mut app, ceremony) = window();
        send(
            &ceremony,
            Kind::Mono(Mono::Art),
            "il riquadro del terminale",
        );
        send(&ceremony, Kind::Title, "Cos'è PlebLine");
        app.drain();

        assert_eq!(app.transcript.len(), 1);
        assert_eq!(app.section, 0);
    }

    /// The trace stays under the finished status.
    #[test]
    fn a_finished_status_replaces_the_working_one() {
        let (mut app, ceremony) = window();
        send(
            &ceremony,
            Kind::Status(Status::Working),
            "Contatto la rete…",
        );
        send(&ceremony, Kind::Trace, "esco da Tor");
        send(&ceremony, Kind::Trace, "chiedo l'ultimo blocco");
        send(
            &ceremony,
            Kind::Status(Status::Done),
            "Contattato. L'ultimo blocco è il 321387.",
        );
        app.drain();

        assert_eq!(
            app.transcript.len(),
            3,
            "la spia non si somma, si sostituisce"
        );
        assert_eq!(app.transcript[0].kind, Kind::Status(Status::Done));
        assert_eq!(
            app.transcript[0].text,
            "Contattato. L'ultimo blocco è il 321387."
        );
        assert_eq!(
            app.transcript[2].kind,
            Kind::Trace,
            "la traccia resta dov'era, sotto la spia"
        );
    }

    #[test]
    fn a_failed_status_replaces_the_working_one_too() {
        let (mut app, ceremony) = window();
        send(
            &ceremony,
            Kind::Status(Status::Working),
            "Contatto la rete…",
        );
        send(&ceremony, Kind::Trace, "esco da Tor");
        send(&ceremony, Kind::Status(Status::Stopped), "Non risponde.");
        app.drain();

        assert_eq!(app.transcript.len(), 2);
        assert_eq!(app.transcript[0].kind, Kind::Status(Status::Stopped));
    }

    #[test]
    fn a_second_status_does_not_reopen_the_first() {
        let (mut app, ceremony) = window();
        send(&ceremony, Kind::Status(Status::Working), "primo lavoro");
        send(&ceremony, Kind::Status(Status::Done), "primo fatto");
        send(&ceremony, Kind::Status(Status::Working), "secondo lavoro");
        send(&ceremony, Kind::Status(Status::Done), "secondo fatto");
        app.drain();

        assert_eq!(app.transcript.len(), 2);
        assert_eq!(app.transcript[0].text, "primo fatto");
        assert_eq!(app.transcript[1].text, "secondo fatto");
    }

    #[test]
    fn a_retry_replaces_the_block_the_failed_attempt_left() {
        let (mut app, ceremony) = window();
        send(&ceremony, Kind::Note, "una nota che resta dov'è");
        send(
            &ceremony,
            Kind::Status(Status::Working),
            "Contatto la rete…",
        );
        send(&ceremony, Kind::Trace, "busso al proxy");
        send(&ceremony, Kind::Status(Status::Stopped), "Non risponde.");
        send(&ceremony, Kind::Error, "Tor non risponde su 9050");
        send(&ceremony, Kind::Remedy, "accendilo");
        app.drain();
        assert_eq!(app.transcript.len(), 5, "prima del ritentativo");

        send(
            &ceremony,
            Kind::Status(Status::Retrying),
            "Contatto la rete… (2° tentativo)",
        );
        send(&ceremony, Kind::Trace, "busso al proxy");
        app.drain();

        assert_eq!(
            app.transcript.len(),
            3,
            "il blocco fallito se ne va con la sua traccia, il suo errore e il suo rimedio"
        );
        assert_eq!(
            app.transcript[0].kind,
            Kind::Note,
            "quello che veniva prima del blocco non si tocca"
        );
        assert_eq!(app.transcript[1].kind, Kind::Status(Status::Retrying));
        assert!(app.transcript[1].text.contains("2°"));
    }

    #[test]
    fn retrying_over_and_over_does_not_pile_up() {
        let (mut app, ceremony) = window();
        send(
            &ceremony,
            Kind::Status(Status::Working),
            "Contatto la rete…",
        );
        send(&ceremony, Kind::Status(Status::Stopped), "Non risponde.");
        send(&ceremony, Kind::Error, "Tor non risponde");
        app.drain();

        for attempt in 2..=10 {
            send(
                &ceremony,
                Kind::Status(Status::Retrying),
                &format!("Contatto la rete… ({attempt}° tentativo)"),
            );
            send(&ceremony, Kind::Status(Status::Stopped), "Non risponde.");
            send(&ceremony, Kind::Error, "Tor non risponde");
            app.drain();
        }

        assert_eq!(app.transcript.len(), 2, "una spia e un errore, non trenta");
        assert!(app.transcript[0].text.contains("Non risponde"));
    }

    #[test]
    fn a_retry_leaves_alone_what_is_not_its_block() {
        let (mut app, ceremony) = window();
        send(
            &ceremony,
            Kind::Status(Status::Working),
            "Contatto la rete…",
        );
        send(&ceremony, Kind::Status(Status::Stopped), "Non risponde.");
        send(&ceremony, Kind::Title, "Un altro passo, in mezzo");
        app.drain();

        assert_eq!(app.transcript.len(), 2);

        send(
            &ceremony,
            Kind::Status(Status::Retrying),
            "Contatto la rete… (2)",
        );
        app.drain();

        assert_eq!(app.transcript.len(), 3, "niente viene tolto");
        assert_eq!(app.transcript[1].kind, Kind::Title);
        assert_eq!(app.transcript[2].kind, Kind::Status(Status::Retrying));
    }

    #[test]
    fn a_retry_does_not_erase_a_status_still_working() {
        let (mut app, ceremony) = window();
        send(
            &ceremony,
            Kind::Status(Status::Working),
            "Contatto la rete…",
        );
        send(&ceremony, Kind::Trace, "busso al proxy");
        app.drain();

        send(
            &ceremony,
            Kind::Status(Status::Retrying),
            "Contatto la rete… (2)",
        );
        app.drain();

        assert_eq!(app.transcript.len(), 3);
    }

    #[test]
    fn a_result_with_no_status_open_is_kept() {
        let (mut app, ceremony) = window();
        send(&ceremony, Kind::Note, "un passo qualsiasi");
        send(
            &ceremony,
            Kind::Status(Status::Done),
            "fatto, chissà quando",
        );
        app.drain();

        assert_eq!(app.transcript.len(), 2);
        assert_eq!(app.transcript[1].kind, Kind::Status(Status::Done));
    }
}
