//! Text reports for the CLI (`--print`, `--bench`) and for the WebAssembly
//! build, so the browser's "Print" and "Bench" buttons show exactly what the
//! command line shows.

use std::fmt::Write as _;
use std::time::Duration;

use crate::clock::now_ms;
use crate::config::Settings;
use crate::generator::{generate_with, GenConfig};
use crate::model::validate;
use crate::rng::Rng;
use crate::solver::{SolveLimits, Solver};
use crate::text::{render_cells, render_puzzle};

/// Generate `count` puzzles and print each one with its solution plus a
/// verification pass.
pub fn print_puzzles(count: usize, settings: &Settings, seed: u64) -> String {
    let mut out = String::new();
    for k in 0..count {
        let this_seed = seed.wrapping_add(k as u64);
        let mut rng = Rng::new(this_seed);
        let mut cfg = GenConfig::from_settings(settings);
        cfg.deadline = Some(now_ms() + 120_000.0);
        match generate_with(&mut rng, &cfg) {
            None => {
                let _ = writeln!(out, "seed {this_seed}: generation failed\n");
            }
            Some(result) => {
                let puzzle = result.puzzle;
                let _ = writeln!(
                    out,
                    "Tapa {w}x{h}  seed={this_seed}  clues={}  black={}  blank={}  clueless={}  attempts={}  removed={}  {:.2}s",
                    result.stats.clue_count,
                    result.stats.black_count,
                    result.stats.blank_region,
                    result.stats.clueless_region,
                    result.stats.attempts,
                    result.stats.removals,
                    result.stats.elapsed.as_secs_f64(),
                    w = settings.size,
                    h = settings.size
                );
                out.push_str(&render_puzzle(&puzzle));
                out.push_str("solution:\n");
                out.push_str(&render_cells(&puzzle, &puzzle.solution));

                let errors = validate(&puzzle.grid, &puzzle.solution, &puzzle.clues);
                let solver = Solver::new(puzzle.grid.clone(), &puzzle.clues);
                let outcome = solver.solve(&solver.fresh_state(), &SolveLimits::unique_check(None));
                let _ = writeln!(
                    out,
                    "check: rules={} solutions={}{} nodes={}",
                    if errors.is_empty() { "ok" } else { "BROKEN" },
                    outcome.solutions.len(),
                    if outcome.aborted { " (aborted)" } else { "" },
                    outcome.nodes
                );
                out.push('\n');
            }
        }
    }
    out
}

/// Time `count` generations and summarise them.
pub fn bench(count: usize, settings: &Settings, seed: u64) -> String {
    let mut out = String::new();
    let mut total = Duration::ZERO;
    let mut slowest = Duration::ZERO;
    let mut clue_lo = usize::MAX;
    let mut clue_hi = 0usize;
    let mut failures = 0usize;
    for k in 0..count {
        let this_seed = seed.wrapping_add(k as u64);
        let mut rng = Rng::new(this_seed);
        let mut cfg = GenConfig::from_settings(settings);
        cfg.deadline = Some(now_ms() + 120_000.0);
        match generate_with(&mut rng, &cfg) {
            None => {
                failures += 1;
                let _ = writeln!(out, "seed {this_seed}: FAILED");
            }
            Some(result) => {
                total += result.stats.elapsed;
                slowest = slowest.max(result.stats.elapsed);
                clue_lo = clue_lo.min(result.stats.clue_count);
                clue_hi = clue_hi.max(result.stats.clue_count);
                let _ = writeln!(
                    out,
                    "seed {this_seed}: {:>7.0} ms  clues={:>3}  black={:>3}  blank={:>3} clueless={:>3}  attempts={}  nodes={} solve={:.0}ms checks={} maxcheck={}",
                    result.stats.elapsed.as_secs_f64() * 1000.0,
                    result.stats.clue_count,
                    result.stats.black_count,
                    result.stats.blank_region,
                    result.stats.clueless_region,
                    result.stats.attempts,
                    result.stats.nodes,
                    result.stats.solve_time.as_secs_f64() * 1000.0,
                    result.stats.checks,
                    result.stats.max_check_nodes
                );
            }
        }
    }
    let ok = count.saturating_sub(failures).max(1);
    let _ = writeln!(
        out,
        "\n{}x{}: {} puzzle(s), {} failed, avg {:.0} ms, slowest {:.0} ms, clues {}..{}",
        settings.size,
        settings.size,
        count,
        failures,
        total.as_secs_f64() * 1000.0 / ok as f64,
        slowest.as_secs_f64() * 1000.0,
        if clue_lo == usize::MAX { 0 } else { clue_lo },
        clue_hi
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_a_puzzle_with_a_check_line() {
        let settings = Settings {
            size: 8,
            seed: Some(1),
            max_attempts: 200,
            ..Settings::default()
        };
        let text = print_puzzles(1, &settings, 1);
        assert!(text.contains("check: rules=ok solutions=1"), "{text}");
        assert!(text.contains("solution:"), "{text}");
    }

    #[test]
    fn bench_reports_every_seed() {
        let settings = Settings {
            size: 8,
            ..Settings::default()
        };
        let text = bench(2, &settings, 100);
        assert_eq!(text.matches("seed ").count(), 2, "{text}");
        assert!(text.contains("clues "), "{text}");
    }
}
