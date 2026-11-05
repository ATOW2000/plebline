//! Lays out each frame: the header, the step line with one point per step,
//! the box above the current point (text to read, then the question), the
//! answer buttons beside it, and the console below. No menu, no tabs.

use eframe::egui;
use plebline::out::Link;
use plebline_core::Error;

use super::draw::{badge, chip, draw_in_box, draw_line, secondary};
use super::{App, Outcome};
use crate::bridge::{Answer, Shape};
use crate::stage;
use crate::theme::{
    AMBER, AMBER_DIM, BACKGROUND, BORDER, COLD, COLD_DIM, GREEN, NIGHT, RED, SUNKEN, TEXT_STRONG,
    WEIGHT_BRAND,
};

impl App {
    /// Draws one frame of the whole window.
    pub(super) fn stage(&mut self, ui: &mut egui::Ui) {
        use egui::{pos2, vec2, Rect, Stroke};
        use stage::{group_left, paint_road, BOX_W, CONNECTOR, CONSOLE_GAP, GAP, HEADER_H, SIDE_W};

        let routes = stage::routes(self.transcript.iter().map(|line| line.kind));

        let full = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, BACKGROUND);

        let header =
            Rect::from_min_size(full.min, vec2(full.width(), HEADER_H)).shrink2(vec2(24.0, 0.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(header)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| self.header(ui),
        );

        let line_y = (full.top() + full.height() * stage::LINE_AT).round();
        let x0 = full.left() + full.width() * 0.05;
        let x1 = full.right() - full.width() * 0.05;
        let here_x = paint_road(&painter, self.place.as_ref(), line_y, x0, x1);

        let console = Rect::from_min_max(
            pos2(full.left() + full.width() * 0.03, line_y + CONSOLE_GAP),
            pos2(full.right() - full.width() * 0.03, full.bottom() - 16.0),
        );
        self.console(ui, console, &routes);

        let box_w = (BOX_W + GAP + SIDE_W).min(full.width() - 32.0) - GAP - SIDE_W;
        // As wide as it came out last frame, so the buttons beside never cover it.
        let drawn = self.box_drawn.max(box_w);
        let left = group_left(
            here_x,
            drawn,
            drawn + GAP + SIDE_W,
            full.left() + 16.0,
            full.right() - 16.0,
        );
        let top = full.top() + HEADER_H + 8.0;
        let bottom = line_y - CONNECTOR;
        let unplugged = self
            .place
            .as_ref()
            .is_some_and(|p| p.link == Link::Unplugged);
        painter.line_segment(
            [pos2(here_x, bottom), pos2(here_x, line_y - 12.0)],
            Stroke::new(2.0, if unplugged { COLD_DIM } else { AMBER_DIM }),
        );
        self.box_drawn = self.box_panel(ui, pos2(left, bottom), box_w, bottom - top, &routes);
        self.side_panel(ui, pos2(left + drawn + GAP, bottom), SIDE_W);
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        use egui::{pos2, Stroke};
        let t = plebline_core::lang::t;

        // The logo: a short amber stroke, a gap, then a longer blue one with
        // a glow.
        let (mark, _) = ui.allocate_exact_size(egui::vec2(48.0, 14.0), egui::Sense::hover());
        let y = mark.center().y;
        let painter = ui.painter();
        painter.line_segment(
            [pos2(mark.left(), y), pos2(mark.left() + 10.0, y)],
            Stroke::new(5.0, AMBER),
        );
        let cold = [pos2(mark.left() + 16.0, y), pos2(mark.right(), y)];
        painter.line_segment(cold, Stroke::new(10.0, COLD.gamma_multiply(0.25)));
        painter.line_segment(cold, Stroke::new(5.0, COLD));
        ui.label(
            egui::RichText::new("PlebLine")
                .size(20.0)
                .variation("wght", WEIGHT_BRAND)
                .color(TEXT_STRONG),
        );

        let (network, test_network) = &self.header;
        let colour = if *test_network { AMBER } else { RED };
        let tag = if *test_network {
            format!("{network} · {}", t("prova", "test"))
        } else {
            network.clone()
        };
        ui.add_space(4.0);
        chip(
            ui,
            &tag.to_uppercase(),
            colour,
            None,
            Stroke::new(1.0, colour.gamma_multiply(0.5)),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let Some(place) = &self.place else {
                return;
            };
            let (link, link_colour) = match place.link {
                Link::Tor => (t("Connesso a Tor", "Connected to Tor"), GREEN),
                Link::OwnNode => (t("Nodo tuo", "Your own node"), GREEN),
                Link::Unplugged => (t("Rete staccata", "Network unplugged"), RED),
                Link::Unknown => (
                    t("Rete da controllare", "Network not checked yet"),
                    COLD_DIM,
                ),
            };
            badge(ui, link_colour, link);
            ui.add_space(10.0);
            if place.words {
                badge(ui, AMBER, t("Parole in memoria", "Words in memory"));
            } else {
                badge(
                    ui,
                    COLD_DIM,
                    t("Nessuna parola in memoria", "No words in memory"),
                );
            }
        });
    }

    /// The box with the text to read and then the question. It is anchored
    /// by its bottom-left corner just above the step line and grows upward,
    /// up to `room`. Returns the width it took.
    fn box_panel(
        &mut self,
        ui: &mut egui::Ui,
        corner: egui::Pos2,
        width: f32,
        room: f32,
        routes: &[stage::Route],
    ) -> f32 {
        const MARGIN: egui::Vec2 = egui::vec2(24.0, 18.0);
        let section = self.section.min(self.transcript.len());
        let inner = (room - 2.0 * MARGIN.y).max(stage::MIN_READING);
        let reading = inner - self.prompt_h - 12.0;
        let shown = egui::Area::new(egui::Id::new("riquadro"))
            .fixed_pos(corner)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(NIGHT)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(MARGIN.x as i8, MARGIN.y as i8))
                    .show(ui, |ui| {
                        ui.set_width(width - 2.0 * MARGIN.x);
                        if reading >= stage::MIN_READING {
                            egui::ScrollArea::vertical()
                                .max_height(reading)
                                .auto_shrink([false, true])
                                .id_salt(("sezione", section))
                                .show(ui, |ui| self.reading(ui, section, routes));
                            self.prompt_h = ui.scope(|ui| self.prompt(ui)).response.rect.height();
                        } else {
                            // A tall question (the words, an address with images): one
                            // scroll for both, starting at the question.
                            egui::ScrollArea::vertical()
                                .max_height(inner)
                                .auto_shrink([false, true])
                                .stick_to_bottom(true)
                                .id_salt(("sezione e domanda", section))
                                .show(ui, |ui| {
                                    self.reading(ui, section, routes);
                                    self.prompt_h =
                                        ui.scope(|ui| self.prompt(ui)).response.rect.height();
                                });
                        }
                    });
            });
        shown.response.rect.width()
    }

    fn reading(&self, ui: &mut egui::Ui, section: usize, routes: &[stage::Route]) {
        for (line, route) in self.transcript[section..].iter().zip(&routes[section..]) {
            if route.boxed {
                draw_in_box(ui, line);
            }
        }
    }

    /// The answers that are buttons, beside the box, bottoms aligned.
    fn side_panel(&mut self, ui: &mut egui::Ui, corner: egui::Pos2, width: f32) {
        let Some((prompt, _)) = &self.question else {
            return;
        };
        let shape = prompt.shape.clone();
        egui::Area::new(egui::Id::new("pulsanti"))
            .fixed_pos(corner)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ui.ctx(), |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 10.0;
                match &shape {
                    Shape::Select(options) => self.choices(ui, options),
                    Shape::Confirm(default) => self.yes_no(ui, *default),
                    _ => {}
                }
                if self.question.is_some() {
                    self.cancel(ui);
                }
            });
    }

    /// Progress, as it happens, newest at the bottom. Read only.
    fn console(&mut self, ui: &mut egui::Ui, area: egui::Rect, routes: &[stage::Route]) {
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::Frame::new()
                .fill(SUNKEN)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .id_salt("console")
                        .show(ui, |ui| {
                            for (line, route) in self.transcript.iter().zip(routes) {
                                if route.console {
                                    draw_line(ui, line);
                                }
                            }
                        });
                });
        });
    }

    /// The end of the ceremony, or the question: text in the box, and the
    /// field when there is one. Choices and yes/no are buttons beside.
    fn prompt(&mut self, ui: &mut egui::Ui) {
        let t = plebline_core::lang::t;
        match &self.outcome {
            Outcome::Finished => {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(t(
                        "Finito. Puoi chiudere la finestra.",
                        "Done. You can close the window.",
                    ))
                    .color(COLD)
                    .strong(),
                );
                return;
            }
            Outcome::Stopped => {
                ui.add_space(10.0);
                let remedy = Error::Cancelled.remedy().unwrap_or_default();
                ui.label(
                    egui::RichText::new(format!("{} {remedy}", t("Fermato.", "Stopped.")))
                        .color(AMBER),
                );
                return;
            }
            Outcome::Running => {}
        }

        let Some((prompt, _)) = &self.question else {
            return;
        };
        let prompt = prompt.clone();

        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(&prompt.text)
                .strong()
                .size(16.0)
                .color(TEXT_STRONG),
        );
        if let Some(help) = &prompt.help {
            ui.label(egui::RichText::new(help).weak().small());
        }
        ui.add_space(6.0);

        match &prompt.shape {
            Shape::Confirm(_) | Shape::Select(_) => {}
            Shape::Text(_) => self.text_prompt(ui, false),
            Shape::Password => self.text_prompt(ui, true),
            Shape::Address(network) => self.address(ui, *network),
            Shape::Number {
                kind,
                invalid,
                scale,
                ..
            } => self.number(ui, *kind, invalid.clone(), *scale),
            Shape::Seed { count } => self.seed(ui, *count),
        }
    }

    /// A cancel button on every question (Esc does the same), so nobody is
    /// stuck with a seed in memory. The ceremony decides where cancelling
    /// leads.
    fn cancel(&mut self, ui: &mut egui::Ui) {
        let t = plebline_core::lang::t;
        let label = stage::cancel_label(self.place.as_ref().map(|p| p.cancel));
        ui.add_space(4.0);
        let pressed = ui
            .add(secondary(label).min_size(egui::vec2(ui.available_width(), 32.0)))
            .on_hover_text(t(
                "quello che è già scritto resta dov'è",
                "what is already written stays where it is",
            ))
            .clicked();
        if pressed || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.reply(Answer::Cancelled);
        }
    }
}
