//! The Tapa engine, with no dependency on any windowing or platform API.
//!
//! Everything that is not drawing or input lives here, so the native Win32
//! game and the WebAssembly build in `wasm/` run exactly the same rules,
//! solver and generator:
//!
//! * [`model`] - the board, the clues and the rules (including live feedback)
//! * [`solver`] - arc consistency plus search, counts solutions up to a limit
//! * [`generator`] - tree-shaped puzzles whose clues prove a unique solution
//! * [`config`] - the `key = value` settings the UI edits
//! * [`rng`] - a small deterministic generator, so seeds reproduce puzzles
//! * [`clock`] - monotonic milliseconds that also work in the browser

pub mod clock;
pub mod config;
pub mod generator;
pub mod model;
pub mod report;
pub mod rng;
pub mod solver;
pub mod text;
