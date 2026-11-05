//! The parts of the layout that need no screen, so tests can call them:
//! which lines go to the box and which to the console, where the step
//! points sit, and the painting of the step line.

use eframe::egui::{pos2, Align2, FontId, Painter, Stroke};
use plebline::out::{Cancel, Kind, Link, Place};
use plebline_core::lang::t;

use crate::theme::{AMBER, AMBER_DIM, BACKGROUND, COLD, COLD_DIM, TEXT, TEXT_DIM, TEXT_STRONG};

/// Height of the step line, as a fraction of the window: the box above it
/// needs more room than the console below.
pub const LINE_AT: f32 = 0.70;
pub const HEADER_H: f32 = 64.0;
/// The box, the buttons beside it, and the gap between.
pub const BOX_W: f32 = 620.0;
pub const SIDE_W: f32 = 280.0;
pub const GAP: f32 = 24.0;
/// Space between the step line and the console, for two staggered rows of
/// labels.
pub const CONSOLE_GAP: f32 = 56.0;
/// Space between the box and the step line, crossed by a short vertical
/// stroke.
pub const CONNECTOR: f32 = 40.0;
/// Below this, the text to read and the question scroll together.
pub const MIN_READING: f32 = 96.0;

/// Lines that report progress go to the console. Errors go to the box too.
pub fn in_console(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Status(_) | Kind::Trace | Kind::Success | Kind::Failure | Kind::Step | Kind::Error
    )
}

/// Lines the user reads before answering go to the box.
pub fn in_box(kind: Kind) -> bool {
    kind == Kind::Error || !in_console(kind)
}

/// Where one line is shown: the box, the console, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub boxed: bool,
    pub console: bool,
}

/// Where each line goes. A ✓ or ✗ that follows a step or a status light in
/// the same section reports on that work and goes to the console; before
/// either, it is an item in a list and goes to the box.
pub fn routes(kinds: impl IntoIterator<Item = Kind>) -> Vec<Route> {
    let mut working = false;
    kinds
        .into_iter()
        .map(|kind| {
            match kind {
                Kind::Title => working = false,
                Kind::Step | Kind::Status(_) => working = true,
                _ => {}
            }
            if matches!(kind, Kind::Success | Kind::Failure) {
                Route {
                    boxed: !working,
                    console: working,
                }
            } else {
                Route {
                    boxed: in_box(kind),
                    console: in_console(kind),
                }
            }
        })
        .collect()
}

/// Label of the cancel button: it names where cancelling leads, as the
/// ceremony reports it.
pub fn cancel_label(cancel: Option<Cancel>) -> &'static str {
    match cancel {
        Some(Cancel::Back) => t("Indietro", "Back"),
        Some(Cancel::Home) => t("Torna al portafoglio", "Back to the wallet"),
        Some(Cancel::Stop) => t("Esci", "Quit"),
        None => t("Annulla", "Cancel"),
    }
}

pub fn point_x(i: usize, n: usize, x0: f32, x1: f32) -> f32 {
    if n < 2 {
        return (x0 + x1) / 2.0;
    }
    x0 + (x1 - x0) * i as f32 / (n - 1) as f32
}

/// Left edge of the box and its buttons: the box centred over the point,
/// the whole group inside `[min, max]`.
pub fn group_left(point_x: f32, box_w: f32, group_w: f32, min: f32, max: f32) -> f32 {
    (point_x - box_w / 2.0).min(max - group_w).max(min)
}

/// Paints the step line with its points and their names: amber and flat
/// while the network is on, blue with a glow once it is unplugged, as in
/// the logo. Returns the x of the current point.
pub fn paint_road(painter: &Painter, place: Option<&Place>, y: f32, x0: f32, x1: f32) -> f32 {
    let unplugged = place.is_some_and(|p| p.link == Link::Unplugged);
    let (lit, dim) = if unplugged {
        (COLD, COLD_DIM)
    } else {
        (AMBER, AMBER_DIM)
    };
    let reach = (x1 - x0) * 0.025;
    let ends = [pos2(x0 - reach, y), pos2(x1 + reach, y)];
    if unplugged {
        painter.line_segment(ends, Stroke::new(18.0, COLD.gamma_multiply(0.10)));
        painter.line_segment(ends, Stroke::new(9.0, COLD.gamma_multiply(0.25)));
    }
    painter.line_segment(ends, Stroke::new(4.0, lit));

    let Some(place) = place else {
        return (x0 + x1) / 2.0;
    };
    let n = place.road.len();
    let spacing = if n > 1 {
        (x1 - x0) / (n - 1) as f32
    } else {
        f32::MAX
    };
    // Close points: names alternate between two rows.
    let stagger = spacing < 96.0;
    let at_home = place.road.get(place.here).is_some_and(|stop| stop.home);
    let mut here_x = (x0 + x1) / 2.0;

    for (i, stop) in place.road.iter().enumerate() {
        let x = point_x(i, n, x0, x1);
        let centre = pos2(x, y);
        let done = i < place.here;
        let current = i == place.here;
        // At the wallet, the usual next step is highlighted.
        let suggested = at_home && i == place.here + 1;
        let radius = if current {
            11.0
        } else if stop.home || suggested {
            9.0
        } else {
            7.0
        };

        if current {
            here_x = x;
            painter.circle_filled(centre, radius + 7.0, lit.gamma_multiply(0.20));
        } else if suggested {
            painter.circle_filled(centre, radius + 6.0, COLD.gamma_multiply(0.30));
        }
        let fill = if done || current { lit } else { BACKGROUND };
        let edge = if done || current {
            lit
        } else if suggested {
            COLD
        } else {
            dim
        };
        painter.circle(centre, radius, fill, Stroke::new(2.0, edge));

        let row = if stagger && i % 2 == 1 { 14.0 } else { 0.0 };
        let colour = if current {
            TEXT_STRONG
        } else if suggested {
            COLD
        } else if done {
            TEXT
        } else {
            TEXT_DIM
        };
        painter.text(
            pos2(x, y + 18.0 + row),
            Align2::CENTER_TOP,
            stop.label.to_uppercase(),
            FontId::monospace(11.0),
            colour,
        );
    }
    here_x
}

#[cfg(test)]
mod tests {
    use super::*;
    use plebline::out::Status;

    #[test]
    fn every_line_has_a_place_and_errors_have_two() {
        let kinds = [
            Kind::Blank,
            Kind::Title,
            Kind::Step,
            Kind::Note,
            Kind::Info,
            Kind::Item,
            Kind::Success,
            Kind::Warning,
            Kind::Failure,
            Kind::Status(Status::Working),
            Kind::Trace,
            Kind::Alert,
            Kind::Mono(plebline::out::Mono::Qr),
            Kind::Error,
            Kind::Remedy,
        ];
        for kind in kinds {
            assert!(
                in_box(kind) || in_console(kind),
                "{kind:?} non va da nessuna parte"
            );
        }
        assert!(in_box(Kind::Error) && in_console(Kind::Error));
        assert!(
            in_box(Kind::Warning),
            "un avviso si legge prima di rispondere"
        );
        assert!(in_console(Kind::Trace) && !in_box(Kind::Trace));
    }

    /// The ✓ items under "What it does" are read in the box; a ✓ after a
    /// step reports progress.
    #[test]
    fn a_tick_goes_to_the_box_unless_it_reports_on_a_step() {
        let routes = routes([
            Kind::Title,
            Kind::Success,
            Kind::Failure,
            Kind::Title,
            Kind::Step,
            Kind::Success,
            Kind::Title,
            Kind::Note,
            Kind::Status(Status::Done),
            Kind::Failure,
        ]);
        let boxed: Vec<bool> = routes.iter().map(|r| r.boxed).collect();
        assert_eq!(
            boxed,
            [true, true, true, true, false, false, true, true, false, false]
        );
        assert!(routes.iter().all(|r| r.boxed || r.console));
        assert!(routes[5].console && routes[9].console);
    }

    #[test]
    fn the_cancel_button_names_where_it_leads() {
        plebline_core::lang::set_for_this_thread(plebline_core::lang::Lang::It);
        let names = [Cancel::Back, Cancel::Home, Cancel::Stop].map(|c| cancel_label(Some(c)));
        assert_eq!(names, ["Indietro", "Torna al portafoglio", "Esci"]);
        assert_eq!(cancel_label(None), "Annulla");
    }

    #[test]
    fn the_points_are_spread_evenly_between_the_ends() {
        assert_eq!(point_x(0, 5, 100.0, 500.0), 100.0);
        assert_eq!(point_x(4, 5, 100.0, 500.0), 500.0);
        assert_eq!(point_x(2, 5, 100.0, 500.0), 300.0);
        assert_eq!(point_x(0, 1, 100.0, 500.0), 300.0);
    }

    #[test]
    fn the_box_follows_the_point_inside_the_window() {
        let (box_w, group_w) = (600.0, 860.0);
        assert_eq!(group_left(700.0, box_w, group_w, 16.0, 1400.0), 400.0);
        assert_eq!(group_left(50.0, box_w, group_w, 16.0, 1400.0), 16.0);
        assert_eq!(group_left(1380.0, box_w, group_w, 16.0, 1400.0), 540.0);
    }
}
