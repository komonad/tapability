//! Non-GUI entry points: printing puzzles and benchmarking generation.

use std::time::{Duration, Instant};

use crate::config::Settings;
use crate::generator::{generate_with, GenConfig};
use crate::model::{clue_text, validate, Puzzle, BLACK, WHITE};
use crate::rng::Rng;
use crate::solver::{SolveLimits, Solver};

/// Render a grid: clue cells show their numbers, everything else a dot.
fn render_puzzle(puzzle: &Puzzle) -> String {
    let mut out = String::new();
    for y in 0..puzzle.grid.h {
        for x in 0..puzzle.grid.w {
            let i = puzzle.grid.idx(x, y);
            let cell = match puzzle.clue_at(i) {
                Some(clue) => clue_text(clue),
                None => ".".to_string(),
            };
            out.push_str(&format!("{:>3}", cell));
        }
        out.push('\n');
    }
    out
}

fn render_cells(puzzle: &Puzzle, cells: &[u8]) -> String {
    let mut out = String::new();
    for y in 0..puzzle.grid.h {
        for x in 0..puzzle.grid.w {
            let i = puzzle.grid.idx(x, y);
            out.push_str(match cells[i] {
                BLACK => "  #",
                WHITE => "  .",
                _ => "  ?",
            });
        }
        out.push('\n');
    }
    out
}

pub fn print_puzzles(count: usize, settings: &Settings, seed: u64) {
    for k in 0..count {
        let this_seed = seed.wrapping_add(k as u64);
        let mut rng = Rng::new(this_seed);
        let mut cfg = GenConfig::from_settings(settings);
        cfg.deadline = Some(Instant::now() + Duration::from_secs(120));
        match generate_with(&mut rng, &cfg) {
            None => println!("seed {this_seed}: generation failed\n"),
            Some(result) => {
                let puzzle = result.puzzle;
                println!(
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
                print!("{}", render_puzzle(&puzzle));
                println!("solution:");
                print!("{}", render_cells(&puzzle, &puzzle.solution));

                let errors = validate(&puzzle.grid, &puzzle.solution, &puzzle.clues);
                let solver = Solver::new(puzzle.grid.clone(), &puzzle.clues);
                let outcome = solver.solve(&solver.fresh_state(), &SolveLimits::unique_check(None));
                println!(
                    "check: rules={} solutions={}{} nodes={}",
                    if errors.is_empty() { "ok" } else { "BROKEN" },
                    outcome.solutions.len(),
                    if outcome.aborted { " (aborted)" } else { "" },
                    outcome.nodes
                );
                println!();
            }
        }
    }
}

pub fn bench(count: usize, settings: &Settings, seed: u64) {
    let mut total = Duration::ZERO;
    let mut slowest = Duration::ZERO;
    let mut clue_lo = usize::MAX;
    let mut clue_hi = 0usize;
    let mut failures = 0usize;
    for k in 0..count {
        let this_seed = seed.wrapping_add(k as u64);
        let mut rng = Rng::new(this_seed);
        let mut cfg = GenConfig::from_settings(settings);
        cfg.deadline = Some(Instant::now() + Duration::from_secs(120));
        match generate_with(&mut rng, &cfg) {
            None => {
                failures += 1;
                println!("seed {this_seed}: FAILED");
            }
            Some(result) => {
                total += result.stats.elapsed;
                slowest = slowest.max(result.stats.elapsed);
                clue_lo = clue_lo.min(result.stats.clue_count);
                clue_hi = clue_hi.max(result.stats.clue_count);
                println!(
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
    println!(
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
}

