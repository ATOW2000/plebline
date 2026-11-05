//! Drawing one transcript line, plus the small painted pieces: status dots,
//! chips, buttons, the QR.

use eframe::egui;
use plebline::out::{Kind, Line, Mono, Status};

use crate::theme::{
    AMBER, BACKGROUND, BORDER, COLD, COLD_DIM, GREEN, NIGHT, RED, TEXT_STRONG, WARNING_BG,
    WEIGHT_TITLE,
};

pub(super) fn draw_line(ui: &mut egui::Ui, line: &Line) {
    match line.kind {
        Kind::Blank => ui.add_space(6.0),
        Kind::Title => {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(&line.text)
                    .size(16.0)
                    .strong()
                    .color(COLD),
            );
            ui.scope(|ui| {
                ui.visuals_mut().widgets.noninteractive.bg_stroke =
                    egui::Stroke::new(1.0, COLD_DIM);
                ui.separator();
            });
        }
        Kind::Step => {
            let (done, total) = line.step.unwrap_or((0, 0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{done}/{total}"))
                        .monospace()
                        .color(COLD)
                        .strong(),
                );
                ui.add(egui::Label::new(egui::RichText::new(&line.text).strong()).wrap());
            });
        }
        Kind::Note => {
            ui.label(egui::RichText::new(&line.text).weak());
        }
        Kind::Info => {
            ui.label(&line.text);
        }
        Kind::Item => {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(line.label.as_deref().unwrap_or_default())
                        .monospace()
                        .color(COLD)
                        .strong(),
                );
                // Selectable, so addresses can be copied.
                ui.add(
                    egui::Label::new(egui::RichText::new(&line.text).monospace())
                        .selectable(true)
                        .wrap(),
                );
            });
        }
        Kind::Success => marker(ui, COLD, &line.text),
        Kind::Warning => marker(ui, AMBER, &line.text),
        Kind::Failure => marker(ui, RED, &line.text),
        // Painted, because the monospace font has no `✓`.
        Kind::Status(status) => {
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                let colour = if status == Status::Done { GREEN } else { RED };
                // Pulses while open (asking for repaints), steady when settled. A retry
                // looks the same; its place and count tell it apart.
                let glow = if status.open() { pulse(ui) } else { 1.0 };
                led(ui, colour, glow);
                ui.add(egui::Label::new(egui::RichText::new(&line.text).strong()).wrap());
            });
        }
        Kind::Trace => {
            ui.horizontal_top(|ui| {
                ui.add_space(LED_INDENT);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&line.text)
                            .weak()
                            .monospace()
                            .size(12.0),
                    )
                    .wrap(),
                );
            });
        }
        Kind::Alert => {
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(WARNING_BG)
                .stroke(egui::Stroke::new(1.0, AMBER))
                .inner_margin(12.0)
                .corner_radius(0.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&line.text)
                            .color(AMBER)
                            .strong()
                            .size(15.0),
                    );
                });
            ui.add_space(10.0);
        }
        Kind::Mono(Mono::Qr) => qr_image(ui, &line.text),
        // Pre-formatted, shown as is, selectable.
        Kind::Mono(_) => {
            ui.add(egui::Label::new(egui::RichText::new(&line.text).monospace()).selectable(true));
        }
        Kind::Error => {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    plebline_core::lang::t("Errore:", "Error:"),
                    line.text
                ))
                .color(RED)
                .strong(),
            );
        }
        Kind::Remedy => {
            ui.label(egui::RichText::new(&line.text).color(AMBER));
            ui.add_space(6.0);
        }
    };
}

/// Indent that aligns trace lines under the status line's text.
const LED_INDENT: f32 = 18.0;

/// Dot radius, relative to body text.
const LED_RADIUS: f32 = 4.5;

/// Painted squares, black on white in any theme, because block glyphs leave
/// gaps between lines and a phone has to scan this.
fn qr_image(ui: &mut egui::Ui, text: &str) {
    let grid = plebline::qr::modules_from_ascii(text);
    let side = grid.len();
    if side == 0 {
        return;
    }
    let module = ((ui.available_width() - 8.0) / side as f32)
        .clamp(2.0, 6.0)
        .floor();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(module * side as f32, module * side as f32),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
    for (y, row) in grid.iter().enumerate() {
        for (x, light) in row.iter().enumerate() {
            if *light {
                let min = rect.min + egui::vec2(x as f32 * module, y as f32 * module);
                painter.rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(module, module)),
                    0.0,
                    egui::Color32::WHITE,
                );
            }
        }
    }
}

/// A rounded label with a coloured dot; the header uses two, for the
/// network and for the words.
pub(super) fn badge(ui: &mut egui::Ui, colour: egui::Color32, text: &str) {
    chip(
        ui,
        text,
        TEXT_STRONG,
        Some(colour),
        egui::Stroke::new(1.0, BORDER),
    );
}

/// Text in a rounded outline, sized to the text and painted by hand: a
/// `Frame` in a centred row grows to the row's height, and one holding a
/// nested layout takes the row's whole width.
pub(super) fn chip(
    ui: &mut egui::Ui,
    text: &str,
    colour: egui::Color32,
    light: Option<egui::Color32>,
    edge: egui::Stroke,
) {
    let font = egui::FontId::monospace(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, colour);
    let pad = egui::vec2(10.0, 5.0);
    let light_w = if light.is_some() { 16.0 } else { 0.0 };
    let size = egui::vec2(light_w + galley.size().x, galley.size().y) + pad * 2.0;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    let radius = if light.is_some() {
        rect.height() / 2.0
    } else {
        4.0
    };
    let fill = if light.is_some() {
        NIGHT
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect(rect, radius, fill, edge, egui::StrokeKind::Inside);
    let mut x = rect.left() + pad.x;
    if let Some(light) = light {
        painter.circle_filled(
            egui::pos2(x + LED_RADIUS, rect.center().y),
            LED_RADIUS,
            light,
        );
        x += light_w;
    }
    let top = rect.center().y - galley.size().y / 2.0;
    painter.galley(egui::pos2(x, top), galley, colour);
}

/// Button for the usual next step, filled blue.
pub(super) fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(text)
            .variation("wght", WEIGHT_TITLE)
            .color(BACKGROUND),
    )
    .fill(COLD)
    .stroke(egui::Stroke::new(1.0, TEXT_STRONG.gamma_multiply(0.6)))
    .corner_radius(6)
}

pub(super) fn secondary(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).variation("wght", WEIGHT_TITLE))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, COLD_DIM))
        .corner_radius(6)
}

/// Like [`draw_line`], except that a title is drawn larger, as the step's
/// heading.
pub(super) fn draw_in_box(ui: &mut egui::Ui, line: &Line) {
    if line.kind == Kind::Title {
        ui.label(
            egui::RichText::new(&line.text)
                .size(28.0)
                .variation("wght", WEIGHT_TITLE)
                .color(TEXT_STRONG),
        );
        ui.add_space(4.0);
        return;
    }
    draw_line(ui, line);
}

/// A coloured dot marker (the font lacks ✓/✗).
fn marker(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    ui.horizontal_top(|ui| {
        led(ui, color, 1.0);
        ui.add(egui::Label::new(text).wrap());
    });
}

/// Paints the dot at `glow` brightness, taking the indent width.
fn led(ui: &mut egui::Ui, color: egui::Color32, glow: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(LED_INDENT, ui.text_style_height(&egui::TextStyle::Body)),
        egui::Sense::hover(),
    );
    let lit = color.gamma_multiply(glow);
    let centre = egui::pos2(rect.left() + LED_RADIUS + 2.0, rect.center().y);
    ui.painter().circle_filled(centre, LED_RADIUS, lit);
}

/// Slow sine pulse; requests the next frame itself, or it would only
/// animate on mouse movement.
fn pulse(ui: &egui::Ui) -> f32 {
    ui.ctx().request_repaint();
    let phase = ui.input(|i| i.time) * std::f64::consts::PI;
    0.55 + 0.45 * (phase.sin() as f32).abs()
}

/// Arrow-key navigation; keeps the index within the list.
pub(super) fn arrow_keys(ui: &egui::Ui, choice: &mut usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let last = count - 1;
    if *choice > last {
        *choice = last;
    }
    ui.input(|i| {
        if i.key_pressed(egui::Key::ArrowDown) && *choice < last {
            *choice += 1;
        }
        if i.key_pressed(egui::Key::ArrowUp) && *choice > 0 {
            *choice -= 1;
        }
    });
    *choice
}
