//! Bounded logical assessment: basic rules followed by one-cell contradiction tests.
use crate::{
    clock::now_ms,
    model::{clue_step, deduction_closure, Clue, Grid, BLACK, UNKNOWN, WHITE},
    reasoning::Reasoner,
};

#[derive(Debug)]
pub struct Assessment {
    pub basic_remaining: usize,
    pub first_forced: usize,
    pub cells: Vec<u8>,
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
    let solver = Reasoner::new(grid.clone(), clues);
    let mut state = solver.fresh_state();
    for (cell, _) in clues {
        state[*cell] = WHITE;
    }
    let mut first = state.clone();
    for (cell, clue) in clues {
        for (index, value) in clue_step(grid, &state, *cell, clue) {
            first[index] = value;
        }
    }
    let first_forced = first
        .iter()
        .filter(|&&v| v != UNKNOWN)
        .count()
        .saturating_sub(clues.len());
    for (cell, value) in deduction_closure(grid, &state, clues) {
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
                for (cell, value) in deduction_closure(grid, &state, clues) {
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
        first_forced,
        remaining: state.iter().filter(|&&v| v == UNKNOWN).count(),
        contradiction_steps: steps,
        probes,
        exhausted,
        consistent,
        cells: state,
    }
}

/// Keep an absolute workload floor once boards exceed the reference area.
pub(crate) fn minimum_basic_remaining(grid: &Grid, clue_count: usize) -> usize {
    let playable = grid.len().saturating_sub(clue_count);
    (playable * grid.len().min(625)).div_ceil(grid.len().max(1) * 2)
}

/// Size-normalized structural targets for puzzles with sustained logical work.
/// Completion by the assessment rule set is recorded independently.
pub fn meets_quality_target(grid: &Grid, clue_count: usize, assessment: &Assessment) -> bool {
    if grid.len() < 225 {
        return true;
    }
    let playable = grid.len().saturating_sub(clue_count);
    assessment.basic_remaining >= minimum_basic_remaining(grid, clue_count)
        && assessment.basic_remaining * 100 <= playable * 80
        && assessment.first_forced * 100 >= playable * 3
        && clue_count * 100 >= grid.len() * 12
        && clue_count * 100 <= grid.len() * 23
}

/// Distance to the target workload breaks ties between qualified candidates.
pub fn quality_score(
    grid: &Grid,
    clue_count: usize,
    assessment: &Assessment,
) -> (usize, usize, usize) {
    let playable = grid.len().saturating_sub(clue_count).max(1);
    let bottleneck = assessment.basic_remaining * 1000 / playable;
    let steps = assessment.contradiction_steps * 1000 / grid.len().max(1);
    (
        1000usize.saturating_sub(bottleneck.abs_diff(600)),
        1000usize.saturating_sub(steps.abs_diff(77)),
        grid.len().saturating_sub(clue_count),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_targets_do_not_require_a_complete_logical_path() {
        let grid = Grid::new(20, 20);
        let mut assessment = Assessment {
            basic_remaining: 200,
            first_forced: 48,
            remaining: 0,
            contradiction_steps: 24,
            probes: 800,
            exhausted: false,
            consistent: true,
            cells: Vec::new(),
        };
        assert!(meets_quality_target(&grid, 68, &assessment));
        assessment.remaining = 1;
        assert!(meets_quality_target(&grid, 68, &assessment));
        assessment.remaining = 0;
        assessment.exhausted = true;
        assert!(meets_quality_target(&grid, 68, &assessment));
        assessment.exhausted = false;
        assessment.basic_remaining = 20;
        assert!(!meets_quality_target(&grid, 68, &assessment));
        assessment.basic_remaining = 200;
        assessment.contradiction_steps = 0;
        assert!(meets_quality_target(&grid, 68, &assessment));
        assessment.contradiction_steps = 24;
        assessment.first_forced = 0;
        assert!(!meets_quality_target(&grid, 68, &assessment));
    }

    #[test]
    fn large_board_workload_does_not_require_proportional_stalls() {
        let grid = Grid::new(60, 60);
        let assessment = Assessment {
            basic_remaining: 405,
            first_forced: 600,
            cells: Vec::new(),
            remaining: 0,
            contradiction_steps: 84,
            probes: 800,
            exhausted: false,
            consistent: true,
        };
        assert!(meets_quality_target(&grid, 596, &assessment));
        assert_eq!(minimum_basic_remaining(&Grid::new(25, 25), 110), 258);
    }

    #[test]
    fn workload_score_prefers_the_target_over_a_larger_bottleneck() {
        let grid = Grid::new(20, 20);
        let mut assessment = Assessment {
            basic_remaining: 200,
            first_forced: 48,
            remaining: 0,
            contradiction_steps: 30,
            probes: 800,
            exhausted: false,
            consistent: true,
            cells: Vec::new(),
        };
        let target = quality_score(&grid, 68, &assessment);
        assessment.basic_remaining = 260;
        assert!(target > quality_score(&grid, 68, &assessment));
    }

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
