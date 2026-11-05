//! Ceremony steps and connectors, shared by every binary. Money logic lives
//! in `plebline-core`; a fix here is written once.

pub mod ask;
pub mod chain;
pub mod destination;
pub mod harden;
pub mod inbox;
pub mod online;
pub mod out;
pub mod project;
pub mod qr;
pub mod replace;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod tor;
pub mod ui;
pub mod wizard;

use plebline_core::Error;

/// Error, cause chain and remedy; shared so every binary looks the same.
pub fn report(error: &Error) {
    report_with(error, error.remedy());
}

/// [`report`] with another remedy: the guided path has no options to point
/// at.
pub fn report_with(error: &Error, remedy: Option<&str>) {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        let cause = plebline_core::lang::t("causa", "cause");
        message.push_str(&format!("\n  {cause}: {inner}"));
        source = inner.source();
    }
    ui::error_with_remedy(&message, remedy);
}
