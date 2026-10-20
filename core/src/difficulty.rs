//! Bounded logical assessment: basic rules followed by one-cell contradiction tests.
use crate::{
    clock::now_ms,
    model::{one_step_deductions, Clue, Grid, BLACK, UNKNOWN, WHITE},
    solver::Solver,
};

#[derive(Debug)]
pub struct Assessment {
    pub basic_remaining: usize,
    pub remaining: usize,
    pub contradiction_steps: usize,
    pub probes: usize,
    pub exhausted: bool,
    pub consistent: bool,
}

impl Assessment {
    pub fn solved(&self) -> bool {
        self.consistent && !self.exhausted && self.remaining == 0
    }
}

/// Probe one assignment at a time using propagation, without recursive search.
/// Resource exhaustion leaves a partial assessment and never proves solvability.
pub fn assess(
    grid: &Grid,
    clues: &[(usize, Clue)],
    max_probes: usize,
    deadline: Option<f64>,
) -> Assessment {
    let solver = Solver::new(grid.clone(), clues);
    let mut state = solver.fresh_state();
    for (cell, _) in clues {
        state[*cell] = WHITE;
    }
    for (cell, value) in one_step_deductions(grid, &state, clues) {
        state[cell] = value;
    }
    let basic_remaining = state.iter().filter(|&&v| v == UNKNOWN).count();
    let mut steps = 0;
    let mut probes = 0;
    let mut exhausted = false;
    let mut consistent = true;
    'deduce: loop {
        if !solver.propagate(&mut state) {
            consistent = false;
            break;
        }
        let mut progress = false;
        for cell in 0..grid.len() {
            if state[cell] != UNKNOWN {
                continue;
            }
            if probes + 2 > max_probes || deadline.is_some_and(|end| now_ms() >= end) {
                exhausted = true;
                break 'deduce;
            }
            let mut black = state.clone();
            black[cell] = BLACK;
            let black_ok = solver.propagate(&mut black);
            let mut white = state.clone();
            white[cell] = WHITE;
            let white_ok = solver.propagate(&mut white);
            probes += 2;
            if !black_ok && !white_ok {
                consistent = false;
                break 'deduce;
            }
            if black_ok != white_ok {
                state = if black_ok { black } else { white };
                for (cell, value) in one_step_deductions(grid, &state, clues) {
                    state[cell] = value;
                }
                steps += 1;
                progress = true;
                break;
            }
        }
        if !progress {
            break;
        }
    }
    Assessment {
        basic_remaining,
        remaining: state.iter().filter(|&&v| v == UNKNOWN).count(),
        contradiction_steps: steps,
        probes,
        exhausted,
        consistent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_and_inconsistent_boards_are_distinct() {
        let grid = Grid::new(2, 1);
        let rating = assess(&grid, &[(0, vec![1])], 0, None);
        assert!(rating.solved());
        let rating = assess(&Grid::new(1, 1), &[(0, vec![1])], 10, None);
        assert!(!rating.consistent);
        assert!(!rating.solved());
    }

    #[test]
    fn probe_limits_do_not_certify_unresolved_boards() {
        let grid = Grid::new(3, 3);
        let rating = assess(&grid, &[(4, vec![1])], 1, None);
        assert!(rating.exhausted);
        assert!(!rating.solved());
        assert!(rating.remaining > 0);
        assert_eq!(rating.probes, 0);
        let rating = assess(&grid, &[(4, vec![1])], 100, None);
        assert!(!rating.exhausted);
        assert!(!rating.solved());
        assert_eq!(rating.contradiction_steps, 0);
    }
}
