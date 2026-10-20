//! Compare puzzles using the same deterministic deductions and bounded solver.
//! Input lines: width<TAB>height<TAB>label<TAB>cell=digits;cell=digits.
//! A digit 0 represents a clue with no black neighbours.
use std::{
    env, fs,
    io::{self, Write},
    time::Instant,
};
use tapa_core::{
    clock::now_ms,
    config::Settings,
    difficulty,
    generator::{generate_with, GenConfig},
    model::{clue_step, deduction_closure, validate, Clue, Grid, BLACK, UNKNOWN, WHITE},
    rng::Rng,
    solver::{SolveLimits, Solver},
};

fn assess(label: &str, grid: &Grid, clues: &[(usize, Clue)], generation_ms: f64) {
    let mut marks = vec![UNKNOWN; grid.len()];
    for (cell, _) in clues {
        marks[*cell] = WHITE;
    }
    let mut initial = marks.clone();
    for (cell, clue) in clues {
        for (i, value) in clue_step(grid, &marks, *cell, clue) {
            initial[i] = value;
        }
    }
    let first = initial.iter().filter(|&&v| v != UNKNOWN).count() - clues.len();
    for (cell, value) in deduction_closure(grid, &marks, clues) {
        marks[cell] = value;
    }
    let remaining = marks.iter().filter(|&&v| v == UNKNOWN).count();
    let solver = Solver::new(grid.clone(), clues);
    let result = solver.solve(
        &solver.fresh_state(),
        &SolveLimits::unique_check(Some(now_ms() + 5000.0)).with_node_budget(1_000_000),
    );
    let (density, cycles) = if let Some(solution) = result.solutions.first() {
        assert!(
            validate(grid, solution, clues).is_empty(),
            "{label}: {:?}",
            validate(grid, solution, clues)
        );
        let blacks = solution.iter().filter(|&&v| v == BLACK).count();
        let edges = (0..grid.len())
            .filter(|&i| solution[i] == BLACK)
            .map(|i| {
                grid.ortho(i)
                    .iter()
                    .flatten()
                    .filter(|&&j| j > i && solution[j] == BLACK)
                    .count()
            })
            .sum::<usize>();
        (blacks as f64 / grid.len() as f64, edges + 1 - blacks)
    } else {
        (f64::NAN, 0)
    };
    let logical = difficulty::assess(grid, clues, grid.len() * 12, Some(now_ms() + 3000.0));
    let singles = clues.iter().filter(|(_, clue)| clue.len() == 1).count();
    println!("{label},{},{},{},{first},{remaining},{},{},{density:.4},{cycles},{singles},{generation_ms:.2},{},{},{},{}",
        grid.w, grid.h, clues.len(), result.nodes, result.is_unique(), logical.remaining, logical.contradiction_steps, logical.exhausted, difficulty::meets_quality_target(grid, clues.len(), &logical));
    io::stdout().flush().expect("flush assessment output");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    println!("label,width,height,clues,first_forced,remaining,sat_work,unique,black_fraction,cycles,single_clues,generation_ms,logical_remaining,contradiction_steps,rating_exhausted,quality_passed");
    if let Some(path) = args.get(1).filter(|p| p.as_str() != "-") {
        for line in fs::read_to_string(path)?
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 4 {
                return Err("expected four tab-separated fields".into());
            }
            let grid = Grid::new(fields[0].parse()?, fields[1].parse()?);
            let mut clues = Vec::new();
            for encoded in fields[3].split(';') {
                let (cell, digits) = encoded.split_once('=').ok_or("expected cell=digits")?;
                let index = cell.parse::<usize>()?;
                if index >= grid.len() {
                    return Err("clue cell outside grid".into());
                }
                let clue = if digits == "0" {
                    Vec::new()
                } else {
                    digits
                        .chars()
                        .map(|c| c.to_digit(10).map(|v| v as u8).ok_or("invalid clue digit"))
                        .collect::<Result<Vec<_>, _>>()?
                };
                clues.push((index, clue));
            }
            assess(fields[2], &grid, &clues, 0.0);
        }
    }
    let count: u64 = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(12);
    let sizes = args
        .get(3)
        .map(|s| {
            s.split(',')
                .map(str::parse)
                .collect::<Result<Vec<usize>, _>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![20, 25]);
    for size in sizes {
        let settings = Settings {
            size,
            ..Settings::default()
        };
        for seed in 1..=count {
            let start = Instant::now();
            if let Some(result) =
                generate_with(&mut Rng::new(seed), &GenConfig::from_settings(&settings))
            {
                assess(
                    &format!("generated{size}-{seed}"),
                    &result.puzzle.grid,
                    &result.puzzle.clues,
                    start.elapsed().as_secs_f64() * 1000.0,
                );
            } else {
                eprintln!(
                    "generation failed: size={size}, seed={seed}, elapsed={:.0}ms",
                    start.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
    }
    Ok(())
}
