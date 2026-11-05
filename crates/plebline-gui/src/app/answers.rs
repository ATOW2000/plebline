//! One way to answer per kind of question: buttons, text, an address, a
//! number, a list, the words.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use eframe::egui;
use plebline::out::{Kind, Line};
use plebline_core::bitcoin::Network;
use plebline_core::wallet::SeedWords;
use zeroize::Zeroize;

use super::draw::{arrow_keys, primary, secondary};
use super::{App, Draft};
use crate::bridge::{Answer, Num, Scale};
use crate::theme::{AMBER, COLD, RED};

impl App {
    /// The default answer comes first, filled; it is "no" for anything
    /// irreversible.
    pub(super) fn yes_no(&mut self, ui: &mut egui::Ui, default: bool) {
        let si = plebline_core::lang::t("Sì", "Yes");
        let no = "No";
        let (first, second) = if default { (si, no) } else { (no, si) };
        let size = egui::vec2(ui.available_width(), 44.0);
        if ui.add(primary(first).min_size(size)).clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            self.reply(Answer::Yes(default));
            return;
        }
        if ui.add(secondary(second).min_size(size)).clicked() {
            self.reply(Answer::Yes(!default));
        }
    }

    pub(super) fn text_prompt(&mut self, ui: &mut egui::Ui, secret: bool) {
        let Some((_, draft)) = &mut self.question else {
            return;
        };
        let field = egui::TextEdit::singleline(&mut draft.text)
            .password(secret)
            .desired_width(f32::INFINITY);
        let response = ui.add(field);
        response.request_focus();
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let pressed = ui
            .add(primary(plebline_core::lang::t("Avanti", "Next")))
            .clicked();
        if enter || pressed {
            let text = std::mem::take(&mut draft.text);
            self.reply(Answer::Text(text));
        }
    }

    /// The recipient address. Its own screen: on Tails (no camera) it comes
    /// from a stick or the clipboard, so paste, image search and (where it
    /// works) drag and drop are all visible. Validated while typing.
    pub(super) fn address(&mut self, ui: &mut egui::Ui, network: Network) {
        let t = plebline_core::lang::t;

        let dropped: Option<PathBuf> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));

        let Some((_, draft)) = &mut self.question else {
            return;
        };

        if let Some(file) = dropped {
            read_from_file(draft, &file, network);
        }

        let response = ui.add(
            egui::TextEdit::singleline(&mut draft.text)
                .hint_text(t("bc1…  oppure  bitcoin:bc1…", "bc1…  or  bitcoin:bc1…"))
                .desired_width(f32::INFINITY),
        );
        response.request_focus();
        if response.changed() {
            draft.error = None;
            draft.read_from = None;
            // A pasted file path is read as a QR image.
            read_if_path(draft, network);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add(secondary(t("Incolla", "Paste"))).clicked() {
                // Asks the window for a paste, the same path as Ctrl+V, because a
                // clipboard library on Wayland/GNOME (as on Tails) needs a protocol GNOME
                // does not implement. The field is cleared; the text arrives next frame.
                draft.text.clear();
                draft.error = None;
                draft.read_from = None;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            if ui
                .add(secondary(t(
                    "Cerca un QR nella chiavetta",
                    "Look for a QR on the USB stick",
                )))
                .clicked()
            {
                draft.found = Some(plebline::destination::images_on_sticks());
                draft.error = None;
            }
            if can_drop_files() {
                ui.label(
                    egui::RichText::new(t(
                        "…oppure trascina qui l'immagine di un QR",
                        "…or drag a QR image in here",
                    ))
                    .weak()
                    .small(),
                );
            }
        });

        // Images found on the sticks, listed after a search and hidden once
        // one is picked.
        if let Some(found) = draft.found.clone() {
            ui.add_space(4.0);
            if found.is_empty() {
                ui.label(
                    egui::RichText::new(t(
                        "nessuna immagine trovata nelle chiavette montate",
                        "no images found on the mounted USB sticks",
                    ))
                    .color(AMBER)
                    .small(),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for file in &found {
                            if ui.button(file.display().to_string()).clicked() {
                                read_from_file(draft, file, network);
                            }
                        }
                    });
            }
        }

        ui.add_space(4.0);
        if let Some(file) = draft.read_from.clone() {
            ui.label(
                egui::RichText::new(plebline_core::tf!(
                    { "letto dal QR in {}", file.display() },
                    { "read from the QR in {}", file.display() }
                ))
                .color(COLD)
                .small(),
            );
        }

        // Parsed by the core on every frame.
        let parsed = plebline_core::address::parse(&draft.text, network);
        match &parsed {
            Ok(destination) => {
                ui.label(
                    egui::RichText::new(format!(
                        "✓ {}",
                        plebline::destination::in_groups(&destination.address.to_string())
                    ))
                    .color(COLD)
                    .monospace(),
                );
                if let Some(sat) = destination.amount_sat {
                    ui.label(
                        egui::RichText::new(plebline_core::tf!(
                            {
                                "il codice chiede anche {sat} sat: te lo richiederò fra un attimo, \
                               e potrai cambiarlo"
                            },
                            {
                                "the code also asks for {sat} sat: I will ask you next, \
                               and you can change it"
                            }
                        ))
                        .weak()
                        .small(),
                    );
                }
            }
            Err(reason) if !draft.text.trim().is_empty() && draft.error.is_none() => {
                ui.label(egui::RichText::new(reason.to_string()).color(AMBER).small());
            }
            Err(_) => {}
        }
        if let Some(error) = draft.error.clone() {
            ui.label(egui::RichText::new(error).color(RED).small());
        }

        let valid = parsed.is_ok();
        ui.add_space(4.0);
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let pressed = ui
            .add_enabled(valid, primary(t("Avanti", "Next")))
            .clicked();
        if valid && (enter || pressed) {
            let text = std::mem::take(&mut draft.text);
            self.reply(Answer::Text(text));
        }
    }

    pub(super) fn number(
        &mut self,
        ui: &mut egui::Ui,
        kind: Num,
        invalid: Option<String>,
        scale: Option<Scale>,
    ) {
        // Known bounds: a slider.
        if let Some(scale) = scale {
            self.slider(ui, kind, scale);
            return;
        }
        let Some((_, draft)) = &mut self.question else {
            return;
        };
        let response = ui.add(egui::TextEdit::singleline(&mut draft.text).desired_width(160.0));
        response.request_focus();
        let valid = kind.accepts(&draft.text);
        if !valid && !draft.text.is_empty() {
            let message = invalid.unwrap_or_else(|| {
                plebline_core::lang::t("Non è un numero.", "Not a number.").to_string()
            });
            ui.label(egui::RichText::new(message).color(AMBER).small());
        }
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let pressed = ui
            .add_enabled(valid, primary(plebline_core::lang::t("Avanti", "Next")))
            .clicked();
        // Invalid numbers do not go through.
        if valid && (enter || pressed) {
            let text = std::mem::take(&mut draft.text);
            self.reply(Answer::Text(text));
        }
    }

    /// Slider with its own box for typing. Clamping is off because the scale
    /// only shows magnitude; values outside it are accepted.
    fn slider(&mut self, ui: &mut egui::Ui, kind: Num, scale: Scale) {
        let Some((_, draft)) = &mut self.question else {
            return;
        };
        let integer = kind != Num::F64;

        let mut widget = egui::Slider::new(&mut draft.number, scale.min..=scale.max)
            .clamping(egui::SliderClamping::Never)
            .logarithmic(scale.logarithmic());
        widget = if integer {
            widget.integer()
        } else {
            widget.fixed_decimals(2)
        };
        if ui.add(widget).changed() {
            draft.touched = true;
        }

        // A required scale must be moved (the batch fee). See `Ask::range_f64`.
        let ready = draft.touched || !scale.required;
        if !ready {
            ui.label(
                egui::RichText::new(plebline_core::lang::t(
                    "Sposta il cursore: questo numero non lo posso scegliere io.",
                    "Move the slider: this number is not mine to pick.",
                ))
                .color(AMBER)
                .small(),
            );
        }

        let pressed = ui
            .add_enabled(ready, primary(plebline_core::lang::t("Avanti", "Next")))
            .clicked();
        if ready && pressed {
            // The ceremony reads a string.
            let value = draft.number;
            let written = if integer {
                format!("{}", value.round().max(0.0) as u64)
            } else {
                format!("{value:.2}")
            };
            self.reply(Answer::Text(written));
        }
    }

    /// One button per choice. The first is the usual next step, filled; the
    /// arrows move a highlight and Enter takes it.
    pub(super) fn choices(&mut self, ui: &mut egui::Ui, options: &[String]) {
        let Some((_, draft)) = &mut self.question else {
            return;
        };
        let highlighted = arrow_keys(ui, &mut draft.choice, options.len());
        let mut chosen = None;
        let size = egui::vec2(ui.available_width(), 44.0);

        egui::ScrollArea::vertical()
            .max_height(360.0)
            .id_salt("scelte")
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                for (index, option) in options.iter().enumerate() {
                    let button = if index == 0 {
                        primary(option)
                    } else {
                        secondary(option)
                    };
                    let mut response = ui.add(button.min_size(size).wrap());
                    if index == highlighted && options.len() > 1 {
                        response = response.highlight();
                    }
                    if response.clicked() {
                        chosen = Some(index);
                    }
                }
            });

        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            chosen = Some(highlighted);
        }
        if let Some(index) = chosen {
            self.reply(Answer::Index(index));
        }
    }

    pub(super) fn seed(&mut self, ui: &mut egui::Ui, count: usize) {
        let Some((_, draft)) = &mut self.question else {
            return;
        };
        if draft.words.len() != count {
            draft.words.resize(count, String::new());
        }
        let masked = draft.masked;

        ui.horizontal(|ui| {
            let label = if masked {
                plebline_core::lang::t("Mostra le parole", "Show the words")
            } else {
                plebline_core::lang::t("Nascondi le parole", "Hide the words")
            };
            if ui.add(secondary(label)).clicked() {
                draft.masked = !masked;
            }
            ui.label(
                egui::RichText::new(plebline_core::lang::t(
                    "Restano solo in memoria: non finiscono su disco.",
                    "They stay in memory only: they never reach the disk.",
                ))
                .weak()
                .small(),
            );
        });

        if !masked {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(plebline_core::lang::t(
                    "Le parole sono in chiaro sullo schermo. Assicurati che nessuno stia \
                     guardando, e che nulla stia registrando.",
                    "The words are in the clear on screen. Make sure nobody is watching, and \
                     that nothing is recording.",
                ))
                .color(ui.visuals().warn_fg_color)
                .small(),
            );
        }
        ui.add_space(6.0);

        // Four columns (24 words in six rows), as wide as the box allows.
        // `min_col_width` keeps fields equal; otherwise the last column takes
        // all the space.
        const SPACING: f32 = 16.0;
        let column = ((ui.available_width() - 3.0 * SPACING) / 4.0).floor();
        let field = (column - 36.0).clamp(64.0, 130.0);
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .id_salt("seed")
            .show(ui, |ui| {
                egui::Grid::new("parole")
                    .num_columns(4)
                    .min_col_width(column)
                    .spacing([SPACING, 6.0])
                    .show(ui, |ui| {
                        for index in 0..count {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:>2}", index + 1))
                                        .monospace()
                                        .weak(),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut draft.words[index])
                                        .password(masked)
                                        .desired_width(field)
                                        .font(egui::TextStyle::Monospace),
                                );
                            });
                            if index % 4 == 3 {
                                ui.end_row();
                            }
                        }
                    });
            });

        ui.add_space(6.0);
        let complete = draft.words.iter().all(|w| !w.trim().is_empty());
        let mut submit = false;
        ui.horizontal(|ui| {
            submit = ui
                .add_enabled(
                    complete,
                    primary(plebline_core::lang::t("Apri il wallet", "Open the wallet")),
                )
                .clicked();
            if !complete {
                ui.label(
                    egui::RichText::new(plebline_core::lang::t(
                        "mancano delle parole",
                        "some words are missing",
                    ))
                    .weak()
                    .small(),
                );
            }
        });

        if !submit {
            return;
        }

        // Joined, handed to `SeedWords`, and the copy zeroed right away.
        let mut joined = draft
            .words
            .iter()
            .map(|w| w.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let outcome = SeedWords::from_interactive_input(&joined);
        joined.zeroize();

        match outcome {
            Ok(words) => self.reply(Answer::Seed(Some(words))),
            Err(error) => {
                // Wrong word count: keep what was typed and show the error.
                self.transcript
                    .push(Line::new(Kind::Failure, error.to_string()));
            }
        }
    }
}

/// Reads a QR image into the field via `destination::interpret`: the same
/// check as typed text.
fn read_from_file(draft: &mut Draft, file: &Path, network: Network) {
    match plebline::destination::interpret(&file.to_string_lossy(), network) {
        Ok(parsed) => {
            draft.text = parsed.destination.address.to_string();
            draft.read_from = parsed.from_file;
            draft.error = None;
            draft.found = None;
        }
        Err(reason) => draft.error = Some(reason),
    }
}

/// `winit` delivers dropped files only on X11; on Wayland (Tails) nothing
/// arrives, not even an error.
fn can_drop_files() -> bool {
    drop_files_with(std::env::var_os("WAYLAND_DISPLAY").as_deref())
}

/// A non-empty `WAYLAND_DISPLAY` means winit uses Wayland.
fn drop_files_with(wayland_display: Option<&OsStr>) -> bool {
    match wayland_display {
        Some(value) => value.is_empty(),
        None => true,
    }
}

fn read_if_path(draft: &mut Draft, network: Network) {
    let path = PathBuf::from(draft.text.trim());
    if path.is_file() {
        read_from_file(draft, &path, network);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_wayland_there_is_no_dragging() {
        assert!(!drop_files_with(Some(OsStr::new("wayland-0"))));
    }

    #[test]
    fn without_wayland_dragging_works() {
        assert!(drop_files_with(None));
    }

    #[test]
    fn an_empty_variable_counts_as_absent() {
        assert!(drop_files_with(Some(OsStr::new(""))));
    }
}
