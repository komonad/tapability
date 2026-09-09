//! Non-GUI entry points: printing puzzles and benchmarking generation.
//!
//! Both are thin wrappers around [`tapa_core::report`], which the WebAssembly
//! build calls for its Print and Bench buttons.

use tapa_core::config::Settings;
use tapa_core::report;

pub fn print_puzzles(count: usize, settings: &Settings, seed: u64) {
    print!("{}", report::print_puzzles(count, settings, seed));
}

pub fn bench(count: usize, settings: &Settings, seed: u64) {
    print!("{}", report::bench(count, settings, seed));
}
