//! The Tapa engine, with no dependency on any windowing or platform API.
//!
//! Everything that is not drawing or input lives here, so the native Win32
//! game and the WebAssembly build in `wasm/` run exactly the same rules,
//! solver and generator:
//!
//! * [`model`] - the board, the clues and the rules (including live feedback)
//! * [`solver`] - incremental CDCL SAT with complete solution and uniqueness checks
//! * [`reasoning`] - deterministic deductions without complete search
//! * [`generator`] - puzzle generation with uniqueness proofs and bounded logical assessment
//! * [`difficulty`] - logical progress and one-cell contradiction assessment
//! * [`hints`] - named-rule proofs and SAT-verified hint conclusions
//! * [`history`] - grouped edits and deduction-selection restoration
//! * [`config`] - the `key = value` settings the UI edits
//! * [`rng`] - a small deterministic generator, so seeds reproduce puzzles
//! * [`clock`] - monotonic milliseconds that also work in the browser

pub mod clock;
pub mod config;
pub mod difficulty;
pub mod generator;
pub mod hints;
pub mod history;
pub mod model;
pub mod reasoning;
pub mod report;
pub mod rng;
pub mod solver;
pub mod text;
