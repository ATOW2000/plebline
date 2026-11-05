//! `plebline-gui`: the guided ceremony in a window. This crate only sets up
//! who answers the questions and where the lines go, then calls
//! `plebline_guided::run`, the ceremony the tests drive with scripted
//! answers.
//!
//! The main thread draws and never blocks; the ceremony thread runs `run`
//! and holds the secrets. Only lines and answers cross (the seed once, by
//! move). A crate of its own because a window pulls in X11, Wayland and
//! OpenGL, which the libraries below do not need.

mod app;
mod bridge;
mod stage;
mod theme;

use std::sync::mpsc::channel;

use eframe::egui;
use plebline::chain::Esplora;
use plebline::out;
use plebline_core::lang::{self, Lang};

fn main() -> eframe::Result<()> {
    // First, while no seed exists yet: disable core dumps and, on Linux,
    // stop other processes from reading this one's memory. See `harden`.
    let hardened = plebline::harden::at_startup();

    lang::set(plebline_guided::lang_from_argv().unwrap_or(Lang::It));

    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{}", plebline_guided::version_line());
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 720.0])
            .with_min_inner_size([720.0, 520.0])
            .with_title(title()),
        ..Default::default()
    };

    eframe::run_native(
        "plebline-gui",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);

            // One channel per direction.
            let (to_ui, from_ceremony) = channel();
            let (to_ceremony, from_ui) = channel();

            let ctx = cc.egui_ctx.clone();
            let lines = to_ui.clone();
            std::thread::Builder::new()
                .name("ceremony".into())
                .spawn(move || {
                    // The sink is per thread; this sets the ceremony's.
                    out::to(Box::new(bridge::ToWindow::new(lines, ctx.clone())));
                    if let Some(shortfall) = hardened.shortfall() {
                        plebline::ui::warning(shortfall);
                    }

                    let mut ask = bridge::Window::new(to_ui.clone(), from_ui, ctx.clone());
                    let outcome = plebline_guided::run(&mut ask, &mut Esplora::new());

                    // Errors go through `report`, with the guided path's remedies.
                    let message = outcome.err().map(|error| {
                        plebline_guided::report(&error);
                        error.to_string()
                    });
                    let _ = to_ui.send(bridge::FromCeremony::Done(message));
                    ctx.request_repaint();
                })
                .expect("the ceremony thread did not start");

            Ok(Box::new(app::App::new(
                from_ceremony,
                to_ceremony,
                &network(),
                plebline_guided::TEST_NETWORK,
            )))
        }),
    )
}

fn title() -> String {
    if plebline_guided::TEST_NETWORK {
        // Set before the language is chosen, so no word in it is translated.
        "PlebLine (signet)".to_string()
    } else {
        "PlebLine".to_string()
    }
}

fn network() -> String {
    plebline_guided::NETWORK.to_string()
}
