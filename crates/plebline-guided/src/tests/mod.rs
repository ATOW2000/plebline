//! End-to-end tests: the whole guided path, driven by scripted answers and
//! a fake chain.

use super::*;
use crate::fixtures::*;
use plebline::out::Link;

use plebline::ask::{Kind, Reply, Scripted};

/// Runs the scripted ceremony and expects it to finish without an error.
#[track_caller]
fn run_to_the_end(ceremony: &Project, ask: &mut dyn Ask, chain: &mut dyn Chain) {
    run_in(ceremony, ask, chain).expect("la cerimonia arriva in fondo");
}

mod opening;
mod wallet_page;
mod way_back;
mod what_stays;
mod whole;
