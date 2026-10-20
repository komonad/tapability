//! Explainable deductions and SAT-verified progress from the player's marks.
use crate::{
    clock::now_ms,
    clue_cases::clue_case_step,
    model::{clue_satisfiable, clue_step, connection_bridges, Clue, Grid, BLACK, UNKNOWN, WHITE},
    reasoning::Reasoner,
    solver::PositionSolver,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Square,
    Clue,
    ClueCases,
    Bridge,
    Unreachable,
}
impl Rule {
    pub fn key(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Clue => "clue",
            Self::ClueCases => "clue_cases",
            Self::Bridge => "bridge",
            Self::Unreachable => "unreachable",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Step {
    pub rule: Rule,
    pub focus: Vec<usize>,
    pub marks: Vec<(usize, u8)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    Square,
    Clue,
    Connectivity,
}
impl ConflictKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Clue => "clue",
            Self::Connectivity => "connectivity",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub focus: Vec<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Rule,
    Contradiction,
    Sat,
}
impl Tier {
    pub fn key(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Contradiction => "contradiction",
            Self::Sat => "sat",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Hint {
    pub tier: Tier,
    pub marks: Vec<(usize, u8)>,
    pub focus: Vec<usize>,
    pub assumption: Option<(usize, u8)>,
    pub steps: Vec<Step>,
    pub conflict: Option<Conflict>,
}
#[derive(Debug)]
pub enum Outcome {
    Found(Hint),
    Inconsistent,
    Complete,
    NoForcedMove,
    Incomplete,
}

fn squares(grid: &Grid) -> impl Iterator<Item = [usize; 4]> + '_ {
    (0..grid.h.saturating_sub(1)).flat_map(move |y| {
        (0..grid.w.saturating_sub(1)).map(move |x| {
            [
                grid.idx(x, y),
                grid.idx(x + 1, y),
                grid.idx(x, y + 1),
                grid.idx(x + 1, y + 1),
            ]
        })
    })
}

/// Components of cells that may still become black, with one black witness each.
fn potential(grid: &Grid, marks: &[u8]) -> (Vec<usize>, Vec<usize>) {
    let mut seen = vec![false; grid.len()];
    let mut witnesses = Vec::new();
    let mut unoccupied = Vec::new();
    for root in 0..grid.len() {
        if seen[root] || marks[root] == WHITE {
            continue;
        }
        seen[root] = true;
        let mut cells = vec![root];
        let mut witness = None;
        let mut pos = 0;
        while pos < cells.len() {
            let cell = cells[pos];
            if marks[cell] == BLACK {
                witness = Some(cell);
            }
            for &nb in grid.ortho(cell).iter().flatten() {
                if !seen[nb] && marks[nb] != WHITE {
                    seen[nb] = true;
                    cells.push(nb);
                }
            }
            pos += 1;
        }
        if let Some(cell) = witness {
            witnesses.push(cell);
        } else {
            unoccupied.extend(cells);
        }
    }
    (witnesses, unoccupied)
}

fn conflict(grid: &Grid, marks: &[u8], clues: &[(usize, Clue)]) -> Option<Conflict> {
    for quad in squares(grid) {
        if quad.iter().all(|&i| marks[i] == BLACK) {
            return Some(Conflict {
                kind: ConflictKind::Square,
                focus: quad.to_vec(),
            });
        }
    }
    for (cell, clue) in clues {
        if marks[*cell] == BLACK || !clue_satisfiable(grid, marks, *cell, clue) {
            return Some(Conflict {
                kind: ConflictKind::Clue,
                focus: std::iter::once(*cell)
                    .chain(grid.neighbors(*cell).iter().flatten().copied())
                    .collect(),
            });
        }
    }
    let (black, _) = potential(grid, marks);
    (black.len() > 1).then_some(Conflict {
        kind: ConflictKind::Connectivity,
        focus: black,
    })
}

/// Each step uses one unchanged position and one named rule.
fn next_step(grid: &Grid, marks: &[u8], clues: &[(usize, Clue)]) -> Option<Step> {
    for quad in squares(grid) {
        if quad.iter().filter(|&&i| marks[i] == BLACK).count() == 3 {
            if let Some(&cell) = quad.iter().find(|&&i| marks[i] == UNKNOWN) {
                return Some(Step {
                    rule: Rule::Square,
                    focus: quad.to_vec(),
                    marks: vec![(cell, WHITE)],
                });
            }
        }
    }
    for (cell, clue) in clues {
        let found = clue_step(grid, marks, *cell, clue);
        if !found.is_empty() {
            return Some(Step {
                rule: Rule::Clue,
                focus: std::iter::once(*cell)
                    .chain(grid.neighbors(*cell).iter().flatten().copied())
                    .collect(),
                marks: found,
            });
        }
    }
    if let Some(&bridge) = connection_bridges(grid, marks).first() {
        let mut cut = marks.to_vec();
        cut[bridge] = WHITE;
        let (mut witnesses, _) = potential(grid, &cut);
        witnesses.push(bridge);
        return Some(Step {
            rule: Rule::Bridge,
            focus: witnesses,
            marks: vec![(bridge, BLACK)],
        });
    }
    let (black, unoccupied) = potential(grid, marks);
    if black.len() == 1 && !unoccupied.is_empty() {
        let mut focus: Vec<_> = marks
            .iter()
            .enumerate()
            .filter_map(|(i, &v)| (v == WHITE).then_some(i))
            .collect();
        focus.push(black[0]);
        return Some(Step {
            rule: Rule::Unreachable,
            focus,
            marks: unoccupied.into_iter().map(|i| (i, WHITE)).collect(),
        });
    }
    None
}

/// Replay a failed assumption using named rules, retaining the actual conflict.
fn explain(
    grid: &Grid,
    mut marks: Vec<u8>,
    clues: &[(usize, Clue)],
    deadline: f64,
) -> Option<(Vec<Step>, Conflict)> {
    let mut steps = Vec::new();
    for _ in 0..=grid.len() {
        if now_ms() >= deadline {
            return None;
        }
        if let Some(failure) = conflict(grid, &marks, clues) {
            return Some((steps, failure));
        }
        let step = next_step(grid, &marks, clues)?;
        for &(cell, value) in &step.marks {
            marks[cell] = value;
        }
        steps.push(step);
    }
    None
}

/// Every hint requires a satisfying completion of the current marks first.
/// SAT fallback proves the opposite value impossible; a witness alone is no hint.
pub fn find(grid: &Grid, marks: &[u8], clues: &[(usize, Clue)], deadline: f64) -> Outcome {
    assert_eq!(marks.len(), grid.len());
    let mut current = marks.to_vec();
    for (cell, _) in clues {
        if current[*cell] == BLACK {
            return Outcome::Inconsistent;
        }
        current[*cell] = WHITE;
    }
    if conflict(grid, &current, clues).is_some() {
        return Outcome::Inconsistent;
    }
    let mut exact = PositionSolver::new(grid, clues, &current);
    let support = exact.query(None, Some(deadline));
    let Some(witness) = support.solutions.first() else {
        return if support.aborted {
            Outcome::Incomplete
        } else {
            Outcome::Inconsistent
        };
    };
    if !current.contains(&UNKNOWN) {
        return Outcome::Complete;
    }
    if let Some(step) = next_step(grid, &current, clues) {
        return Outcome::Found(Hint {
            tier: Tier::Rule,
            marks: step.marks.clone(),
            focus: step.focus.clone(),
            assumption: None,
            steps: vec![step],
            conflict: None,
        });
    }
    if let Some(step) = clue_case_step(grid, &current, clues, Some(deadline)) {
        return Outcome::Found(Hint {
            tier: Tier::Rule,
            marks: step.marks.clone(),
            focus: step.focus.clone(),
            assumption: None,
            steps: vec![Step {
                rule: Rule::ClueCases,
                focus: step.focus,
                marks: step.marks,
            }],
            conflict: None,
        });
    }
    let mut candidates: Vec<_> = (0..grid.len()).filter(|&i| current[i] == UNKNOWN).collect();
    let mut weight = vec![0usize; grid.len()];
    for (cell, _) in clues {
        for &nb in grid.neighbors(*cell).iter().flatten() {
            weight[nb] += 1;
        }
    }
    candidates.sort_by_key(|&i| (std::cmp::Reverse(weight[i]), i));
    let reasoner = Reasoner::new(grid.clone(), clues);
    let reasoning_deadline = deadline.min(now_ms() + 250.0);
    for &cell in &candidates {
        if now_ms() >= reasoning_deadline {
            break;
        }
        let opposite = if witness[cell] == BLACK { WHITE } else { BLACK };
        let mut branch = current.clone();
        branch[cell] = opposite;
        if !reasoner.propagate(&mut branch) {
            let mut assumed = current.clone();
            assumed[cell] = opposite;
            if let Some((steps, failure)) =
                explain(grid, assumed, clues, deadline.min(now_ms() + 500.0))
            {
                let mut focus = failure.focus.clone();
                focus.push(cell);
                return Outcome::Found(Hint {
                    tier: Tier::Contradiction,
                    marks: vec![(cell, witness[cell])],
                    focus,
                    assumption: Some((cell, opposite)),
                    steps,
                    conflict: Some(failure),
                });
            }
            break;
        }
    }
    let mut interrupted = false;
    for &cell in &candidates {
        if now_ms() >= deadline {
            return Outcome::Incomplete;
        }
        let opposite = if witness[cell] == BLACK { WHITE } else { BLACK };
        let result = exact.query(Some((cell, opposite)), Some(deadline));
        if result.complete && result.solutions.is_empty() {
            return Outcome::Found(Hint {
                tier: Tier::Sat,
                marks: vec![(cell, witness[cell])],
                focus: vec![cell],
                assumption: Some((cell, opposite)),
                steps: Vec::new(),
                conflict: None,
            });
        }
        interrupted |= result.aborted;
    }
    if interrupted {
        Outcome::Incomplete
    } else {
        Outcome::NoForcedMove
    }
}

#[derive(Debug)]
pub struct ProofMetrics {
    pub assumption_depth: usize,
    pub rule_groups: usize,
    pub clue_count: usize,
    pub span_w: usize,
    pub span_h: usize,
}

impl Hint {
    /// Measures the displayed proof, not the minimum possible proof.
    /// SAT and unrecorded enumeration branches have no proof complexity estimate.
    pub fn metrics(&self, grid: &Grid) -> Option<ProofMetrics> {
        if self.tier == Tier::Sat || self.steps.iter().any(|step| step.rule == Rule::ClueCases) {
            return None;
        }
        let mut clues = std::collections::BTreeSet::new();
        let mut footprint = self.focus.clone();
        for step in &self.steps {
            footprint.extend(&step.focus);
            footprint.extend(step.marks.iter().map(|&(i, _)| i));
            if step.rule == Rule::Clue {
                clues.insert(step.focus[0]);
            }
        }
        if let Some(failure) = &self.conflict {
            if failure.kind == ConflictKind::Clue {
                clues.insert(failure.focus[0]);
            }
        }
        if self
            .steps
            .iter()
            .any(|step| matches!(step.rule, Rule::Bridge | Rule::Unreachable))
            || self
                .conflict
                .as_ref()
                .is_some_and(|failure| failure.kind == ConflictKind::Connectivity)
        {
            footprint.extend(0..grid.len());
        }
        let min_x = footprint.iter().map(|&i| grid.xy(i).0).min().unwrap_or(0);
        let max_x = footprint.iter().map(|&i| grid.xy(i).0).max().unwrap_or(0);
        let min_y = footprint.iter().map(|&i| grid.xy(i).1).min().unwrap_or(0);
        let max_y = footprint.iter().map(|&i| grid.xy(i).1).max().unwrap_or(0);
        Some(ProofMetrics {
            assumption_depth: usize::from(self.tier == Tier::Contradiction),
            rule_groups: self.steps.len(),
            clue_count: clues.len(),
            span_w: max_x - min_x + 1,
            span_h: max_y - min_y + 1,
        })
    }
    pub fn english(&self, grid: &Grid) -> String {
        fn marks(grid: &Grid, cells: &[(usize, u8)]) -> String {
            cells
                .iter()
                .map(|&(i, v)| {
                    let (x, y) = grid.xy(i);
                    format!(
                        "(row {}, column {}) {}",
                        y + 1,
                        x + 1,
                        if v == BLACK { "black" } else { "white" }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        let mut out = match self.tier {
            Tier::Rule => "Rule deduction\n".to_string(),
            Tier::Contradiction => "Single-assumption contradiction\n".to_string(),
            Tier::Sat => "Complete constraint verification\n".to_string(),
        };
        if let Some(assumption) = self.assumption {
            out.push_str(&format!("Assume {}.\n", marks(grid, &[assumption])));
        }
        for step in &self.steps {
            out.push_str(&format!(
                "{}: {}.\n",
                match step.rule {
                    Rule::Clue => {
                        let (x, y) = grid.xy(step.focus[0]);
                        format!(
                            "Feasible arrangements at clue (row {}, column {})",
                            y + 1,
                            x + 1
                        )
                    }
                    Rule::ClueCases => "Local clue distributions with one basic round per case".into(),
                    Rule::Square => "Three black cells in a 2x2".into(),
                    Rule::Bridge => "Required connection between black cells".into(),
                    Rule::Unreachable => "Cells unable to connect to the black region".into(),
                },
                marks(grid, &step.marks)
            ));
        }
        if let Some(conflict) = &self.conflict {
            out.push_str(&format!("Contradiction: {}.\n", conflict.kind.key()));
        }
        if self.tier == Tier::Sat {
            out.push_str(
                "The opposite value has no satisfying completion. No human-rule proof is shown.\n",
            );
        }
        out.push_str(&format!("Conclusion: {}.", marks(grid, &self.marks)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Settings,
        difficulty,
        generator::{generate_with, GenConfig},
        rng::Rng,
        solver::{SolveLimits, Solver},
    };
    #[test]
    fn a_direct_hint_is_one_rule_and_does_not_mutate_the_board() {
        let grid = Grid::new(4, 2);
        let clues = [(0, vec![3]), (2, vec![2])];
        let board = vec![
            WHITE, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN,
        ];
        let Outcome::Found(hint) = find(&grid, &board, &clues, now_ms() + 5000.0) else {
            panic!("missing hint")
        };
        assert_eq!(hint.tier, Tier::Rule);
        assert_eq!(hint.steps.len(), 1);
        let metrics = hint.metrics(&grid).unwrap();
        assert_eq!(metrics.assumption_depth, 0);
        assert_eq!(metrics.clue_count, 1);
        let mut conclusions = hint.marks.clone();
        conclusions.sort_unstable();
        assert_eq!(conclusions, vec![(1, BLACK), (4, BLACK), (5, BLACK)]);
        assert_eq!(
            board,
            vec![WHITE, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN]
        );
    }
    #[test]
    fn local_case_hints_keep_the_enumerated_clue_in_focus() {
        let grid = Grid::new(4, 3);
        let clues = [(0, vec![1]), (5, vec![1])];
        let board = vec![
            WHITE, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, WHITE,
            UNKNOWN, WHITE, WHITE, UNKNOWN, WHITE, UNKNOWN,
        ];
        let Outcome::Found(hint) = find(&grid, &board, &clues, now_ms() + 5000.0) else {
            panic!("missing hint")
        };
        assert_eq!(hint.tier, Tier::Rule);
        assert_eq!(hint.steps.len(), 1);
        assert_eq!(hint.steps[0].rule, Rule::ClueCases);
        assert_eq!(hint.focus[0], 0);
        assert_eq!(hint.marks, vec![(6, WHITE), (9, WHITE)]);
        assert!(hint.metrics(&grid).is_none());
    }

    #[test]
    fn ambiguous_positions_do_not_turn_a_model_into_a_hint() {
        assert!(matches!(
            find(&Grid::new(3, 1), &[UNKNOWN; 3], &[], now_ms() + 5000.0),
            Outcome::NoForcedMove
        ));
    }
    #[test]
    fn contradictions_and_expired_searches_never_propose_marks() {
        assert!(matches!(
            find(&Grid::new(2, 2), &[BLACK; 4], &[], now_ms() + 5000.0),
            Outcome::Inconsistent
        ));
        assert!(matches!(
            find(&Grid::new(3, 1), &[UNKNOWN; 3], &[], now_ms() - 1.0),
            Outcome::Incomplete
        ));
        assert!(matches!(
            find(
                &Grid::new(3, 1),
                &[WHITE, BLACK, WHITE],
                &[(0, vec![1]), (2, vec![1])],
                now_ms() + 5000.0
            ),
            Outcome::Complete
        ));
    }
    #[test]
    fn sat_progresses_after_the_single_assumption_assessor_stalls() {
        let cfg = GenConfig::from_settings(&Settings {
            size: 20,
            ..Settings::default()
        });
        let puzzle = generate_with(&mut Rng::new(10), &cfg).unwrap().puzzle;
        let rating = difficulty::assess(&puzzle.grid, &puzzle.clues, 4800, None);
        assert!(rating.consistent && !rating.exhausted && rating.remaining > 0);
        let Outcome::Found(hint) = find(
            &puzzle.grid,
            &rating.cells,
            &puzzle.clues,
            now_ms() + 5000.0,
        ) else {
            panic!("SAT hint missing")
        };
        assert_eq!(hint.tier, Tier::Sat);
        assert!(hint.metrics(&puzzle.grid).is_none());
        assert_eq!(hint.marks.len(), 1);
        let (cell, value) = hint.marks[0];
        assert_eq!(value, puzzle.solution[cell]);
        let mut opposite = rating.cells.clone();
        opposite[cell] = if value == BLACK { WHITE } else { BLACK };
        let proof = Solver::new(puzzle.grid.clone(), &puzzle.clues)
            .solve(&opposite, &SolveLimits::unique_check(None));
        assert!(proof.complete && proof.solutions.is_empty());
    }
    #[test]
    fn hints_and_explained_branches_are_sound_on_small_generated_puzzles() {
        let cfg = GenConfig::from_settings(&Settings {
            size: 6,
            ..Settings::default()
        });
        let mut assumptions = 0;
        for seed in 1..=8 {
            let puzzle = generate_with(&mut Rng::new(seed), &cfg).unwrap().puzzle;
            let mut marks = vec![UNKNOWN; 36];
            for (i, _) in &puzzle.clues {
                marks[*i] = WHITE;
            }
            for _ in 0..36 {
                let found = find(&puzzle.grid, &marks, &puzzle.clues, now_ms() + 5000.0);
                let Outcome::Found(hint) = found else {
                    assert!(matches!(found, Outcome::Complete));
                    break;
                };
                assert!(!hint.marks.is_empty());
                if hint.tier == Tier::Contradiction {
                    assumptions += 1;
                    let mut branch = marks.clone();
                    let (i, v) = hint.assumption.unwrap();
                    branch[i] = v;
                    for step in &hint.steps {
                        assert!(conflict(&puzzle.grid, &branch, &puzzle.clues).is_none());
                        let expected = next_step(&puzzle.grid, &branch, &puzzle.clues).unwrap();
                        assert_eq!(step.rule, expected.rule);
                        assert_eq!(step.marks, expected.marks);
                        for &(i, v) in &step.marks {
                            assert_eq!(branch[i], UNKNOWN);
                            branch[i] = v;
                        }
                    }
                    assert_eq!(
                        conflict(&puzzle.grid, &branch, &puzzle.clues).unwrap().kind,
                        hint.conflict.as_ref().unwrap().kind
                    );
                }
                for (i, v) in hint.marks {
                    assert_eq!(marks[i], UNKNOWN);
                    assert_eq!(v, puzzle.solution[i]);
                    marks[i] = v;
                }
            }
            assert_eq!(marks, puzzle.solution);
        }
        assert!(assumptions > 0);
    }
}
