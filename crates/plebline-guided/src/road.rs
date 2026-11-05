//! The step line a window draws (one point per step), plus the network state
//! and whether the words are in memory. Kept per thread like the screen
//! (`plebline::out`), for the same reason. Every change is sent whole.

use std::cell::RefCell;

use plebline::out::{self, Cancel, Link, Place, Stop};
use plebline_core::lang::t;

/// A point on the step line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Point {
    Start,
    Network,
    Words,
    Wallet,
    Addresses,
    Count,
    Parts,
    Unplug,
    Sign,
    Check,
    Export,
    Send,
}

impl Point {
    fn label(self) -> &'static str {
        match self {
            Point::Start => t("Inizio", "Start"),
            Point::Network => t("Rete", "Network"),
            Point::Words => t("Parole", "Words"),
            Point::Wallet => t("Portafoglio", "Wallet"),
            Point::Addresses => t("Indirizzi", "Addresses"),
            Point::Count => t("Quante", "How many"),
            Point::Parts => t("Parti", "Parts"),
            Point::Unplug => t("Stacca", "Unplug"),
            Point::Sign => t("Firma", "Sign"),
            Point::Check => t("Controllo", "Check"),
            Point::Export => t("Uscita", "Take away"),
            Point::Send => t("Invio", "Send"),
        }
    }
}

/// The points up to the wallet are the same every time.
const BASE: [Point; 4] = [Point::Start, Point::Network, Point::Words, Point::Wallet];

struct Road {
    points: Vec<Point>,
    here: usize,
    link: Link,
    /// What the link was before unplugging, to put back.
    connected: Link,
    words: bool,
    cancel: Cancel,
}

impl Road {
    fn fresh() -> Self {
        Road {
            points: BASE.to_vec(),
            here: 0,
            link: Link::Unknown,
            connected: Link::Unknown,
            words: false,
            // The first question has nothing before it.
            cancel: Cancel::Stop,
        }
    }

    fn send(&self) {
        out::place(&Place {
            road: self
                .points
                .iter()
                .map(|point| Stop {
                    label: point.label().to_string(),
                    home: *point == Point::Wallet,
                })
                .collect(),
            here: self.here,
            link: self.link,
            words: self.words,
            cancel: self.cancel,
        });
    }
}

thread_local! {
    static ROAD: RefCell<Road> = RefCell::new(Road::fresh());
}

fn change(edit: impl FnOnce(&mut Road)) {
    ROAD.with(|road| {
        let mut road = road.borrow_mut();
        edit(&mut road);
        road.send();
    });
}

/// A new ceremony: back to the start.
pub(crate) fn begin() {
    change(|road| *road = Road::fresh());
}

/// Moves to `point`: the next one ahead with that name, or else the first
/// (the wallet, coming back).
pub(crate) fn at(point: Point) {
    change(|road| {
        let ahead = road.points[road.here..]
            .iter()
            .position(|p| *p == point)
            .map(|i| i + road.here);
        let anywhere = || road.points.iter().position(|p| *p == point);
        if let Some(index) = ahead.or_else(anywhere) {
            road.here = index;
        }
    });
}

/// The operation picked at the wallet page: its points are appended after
/// the wallet's, and the current point is the wallet.
pub(crate) fn operation(tail: &[Point]) {
    change(|road| {
        road.points = BASE.to_vec();
        road.points.extend_from_slice(tail);
        road.here = BASE.len() - 1;
    });
}

/// Replaces the points after the current one (refills that need a split
/// first).
pub(crate) fn reroute(tail: &[Point]) {
    change(|road| {
        road.points.truncate(road.here + 1);
        road.points.extend_from_slice(tail);
    });
}

pub(crate) fn connected(link: Link) {
    change(|road| {
        road.link = link;
        road.connected = link;
    });
}

pub(crate) fn unplugged() {
    change(|road| road.link = Link::Unplugged);
}

pub(crate) fn plugged_back() {
    change(|road| road.link = road.connected);
}

pub(crate) fn words(in_memory: bool) {
    change(|road| road.words = in_memory);
}

/// Where cancelling leads from here. Set by the code that catches the
/// cancel.
pub(crate) fn cancel(to: Cancel) {
    change(|road| road.cancel = to);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::in_italian;
    use plebline::out::Recorder;

    fn here(place: &Place) -> &str {
        &place.road[place.here].label
    }

    #[test]
    fn coming_back_lands_on_the_wallet_and_going_on_finds_the_next_point() {
        in_italian();
        let notebook = Recorder::new();
        out::to(Box::new(notebook.clone()));
        begin();
        at(Point::Words);
        at(Point::Wallet);
        operation(&[Point::Words, Point::Count, Point::Sign]);
        at(Point::Words);
        let retyping = notebook.places().last().cloned().expect("mandato");
        at(Point::Wallet);
        let back = notebook.places().last().cloned().expect("mandato");
        out::take();

        assert_eq!(
            retyping.here, 4,
            "le parole ribattute, non quelle dell'inizio"
        );
        assert_eq!(here(&back), "Portafoglio");
        assert!(back.road[back.here].home);
    }

    #[test]
    fn plugging_back_restores_the_previous_link() {
        in_italian();
        let notebook = Recorder::new();
        out::to(Box::new(notebook.clone()));
        begin();
        connected(Link::OwnNode);
        unplugged();
        plugged_back();
        let links: Vec<Link> = notebook.places().iter().map(|p| p.link).collect();
        out::take();

        assert_eq!(
            links,
            vec![Link::Unknown, Link::OwnNode, Link::Unplugged, Link::OwnNode]
        );
    }

    /// From refills to a split: the points already passed stay, the ones
    /// after the current point change.
    #[test]
    fn rerouting_keeps_the_points_already_passed() {
        in_italian();
        let notebook = Recorder::new();
        out::to(Box::new(notebook.clone()));
        begin();
        at(Point::Wallet);
        operation(&[Point::Addresses, Point::Count, Point::Unplug, Point::Sign]);
        at(Point::Count);
        reroute(&[Point::Unplug, Point::Sign, Point::Send]);
        let place = notebook.places().last().cloned().expect("mandato");
        out::take();

        let labels: Vec<&str> = place.road.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Inizio",
                "Rete",
                "Parole",
                "Portafoglio",
                "Indirizzi",
                "Quante",
                "Stacca",
                "Firma",
                "Invio"
            ]
        );
        assert_eq!(here(&place), "Quante");
    }
}
