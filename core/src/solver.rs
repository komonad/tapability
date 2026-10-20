//! Complete Tapa solving with incremental CDCL SAT and connectivity cuts.
use crate::{
    clock::now_ms,
    model::{feasible_masks, validate, Clue, Grid, BLACK, UNKNOWN, WHITE},
};
use batsat::{lbool, Callbacks, Lit, SolverInterface};

pub struct SolveLimits {
    pub max_solutions: usize,
    pub deadline: Option<f64>,
    /// SAT conflict and model-check allowance; `u64::MAX` is unlimited.
    pub max_nodes: u64,
}
impl SolveLimits {
    pub fn unique_check(deadline: Option<f64>) -> Self {
        Self {
            max_solutions: 2,
            deadline,
            max_nodes: u64::MAX,
        }
    }
    pub fn with_node_budget(mut self, max_nodes: u64) -> Self {
        self.max_nodes = max_nodes;
        self
    }
}
pub struct SolveOutcome {
    pub solutions: Vec<Vec<u8>>,
    /// SAT conflicts plus model checks, independent of human difficulty.
    pub nodes: u64,
    pub aborted: bool,
    /// True when UNSAT proves there are no further models.
    pub complete: bool,
}
impl SolveOutcome {
    pub fn is_unique(&self) -> bool {
        self.complete && !self.aborted && self.solutions.len() == 1
    }
}

#[derive(Default)]
struct Stop {
    deadline: Option<f64>,
    conflicts: u64,
    allowance: u64,
}
impl Callbacks for Stop {
    fn on_new_clause(&mut self, _: &[Lit], _: batsat::ClauseKind) {
        self.conflicts = self.conflicts.saturating_add(1);
    }
    fn stop(&self) -> bool {
        self.conflicts >= self.allowance || self.deadline.is_some_and(|end| now_ms() >= end)
    }
}

struct Session {
    grid: Grid,
    sat: batsat::Solver<Stop>,
    black: Vec<Lit>,
    enabled: Vec<Lit>,
}
impl Session {
    fn new(grid: Grid, clues: &[(usize, Clue)]) -> Self {
        let mut sat = batsat::Solver::new(Default::default(), Stop::default());
        let black: Vec<_> = (0..grid.len())
            .map(|_| Lit::new(sat.new_var_default(), true))
            .collect();
        let mut this = Self {
            grid,
            sat,
            black,
            enabled: Vec::new(),
        };
        for y in 0..this.grid.h.saturating_sub(1) {
            for x in 0..this.grid.w.saturating_sub(1) {
                this.add(
                    [
                        this.grid.idx(x, y),
                        this.grid.idx(x + 1, y),
                        this.grid.idx(x, y + 1),
                        this.grid.idx(x + 1, y + 1),
                    ]
                    .map(|i| !this.black[i])
                    .to_vec(),
                );
            }
        }
        for (cell, clue) in clues {
            let active = Lit::new(this.sat.new_var_default(), true);
            this.enabled.push(active);
            this.add(vec![!active, !this.black[*cell]]);
            let mut choices = vec![!active];
            for mask in feasible_masks(&this.grid, *cell, clue) {
                let pattern = Lit::new(this.sat.new_var_default(), true);
                choices.push(pattern);
                this.add(vec![!pattern, active]);
                let neighbors = *this.grid.neighbors(*cell);
                for (bit, neighbor) in neighbors.into_iter().enumerate() {
                    if let Some(i) = neighbor {
                        this.add(vec![
                            !pattern,
                            this.black[i].apply_sign(mask & (1 << bit) != 0),
                        ]);
                    }
                }
            }
            this.add(choices);
        }
        this
    }
    fn add(&mut self, mut clause: Vec<Lit>) {
        self.sat.add_clause_reuse(&mut clause);
    }
    fn exclude(&mut self, cells: &[u8]) {
        self.add(
            cells
                .iter()
                .enumerate()
                .map(|(i, &v)| self.black[i].apply_sign(v != BLACK))
                .collect(),
        );
    }
    /// A disconnected component needs a black boundary whenever black cells
    /// survive both inside and outside it. Each cut is valid for every clue set.
    fn connect(&mut self, cells: &[u8]) -> bool {
        let mut seen = vec![false; self.grid.len()];
        let mut groups = Vec::new();
        for root in 0..cells.len() {
            if cells[root] != BLACK || seen[root] {
                continue;
            }
            seen[root] = true;
            let mut component = vec![root];
            let mut pos = 0;
            while pos < component.len() {
                for &nb in self.grid.ortho(component[pos]).iter().flatten() {
                    if cells[nb] == BLACK && !seen[nb] {
                        seen[nb] = true;
                        component.push(nb);
                    }
                }
                pos += 1;
            }
            groups.push(component);
        }
        if groups.len() <= 1 {
            return true;
        }
        for (k, group) in groups.iter().enumerate() {
            let outside = groups[usize::from(k == 0)][0];
            let mut boundary = Vec::new();
            for &i in group {
                boundary.extend(
                    self.grid
                        .ortho(i)
                        .iter()
                        .flatten()
                        .copied()
                        .filter(|&nb| cells[nb] != BLACK),
                );
            }
            boundary.sort_unstable();
            boundary.dedup();
            let mut cut = vec![!self.black[group[0]], !self.black[outside]];
            cut.extend(boundary.iter().map(|&i| self.black[i]));
            self.add(cut);
        }
        false
    }
    fn enumerate(&mut self, assumptions: &[Lit], limits: &SolveLimits) -> SolveOutcome {
        let initial_conflicts = self.sat.num_conflicts();
        let mut checks = 0u64;
        let mut solutions = Vec::new();
        let mut aborted = false;
        let mut complete = false;
        self.sat.cb_mut().deadline = limits.deadline;
        while solutions.len() < limits.max_solutions {
            let used = (self.sat.num_conflicts() - initial_conflicts).saturating_add(checks);
            if used >= limits.max_nodes || limits.deadline.is_some_and(|end| now_ms() >= end) {
                aborted = true;
                break;
            }
            self.sat.cb_mut().conflicts = 0;
            self.sat.cb_mut().allowance = limits.max_nodes.saturating_sub(used);
            let result = self.sat.solve_limited(assumptions);
            checks += 1;
            if result == lbool::FALSE {
                complete = true;
                break;
            }
            if result != lbool::TRUE {
                aborted = true;
                break;
            }
            let cells: Vec<_> = self
                .black
                .iter()
                .map(|&lit| {
                    if self.sat.value_lit(lit) == lbool::TRUE {
                        BLACK
                    } else {
                        WHITE
                    }
                })
                .collect();
            if !self.connect(&cells) {
                continue;
            }
            solutions.push(cells.clone());
            if solutions.len() < limits.max_solutions {
                self.exclude(&cells);
            }
        }
        SolveOutcome {
            solutions,
            nodes: (self.sat.num_conflicts() - initial_conflicts).saturating_add(checks),
            aborted,
            complete,
        }
    }
}

pub struct Solver {
    grid: Grid,
    clues: Vec<(usize, Clue)>,
}
impl Solver {
    pub fn new(grid: Grid, clues: &[(usize, Clue)]) -> Self {
        Self {
            grid,
            clues: clues.to_vec(),
        }
    }
    pub fn fresh_state(&self) -> Vec<u8> {
        vec![UNKNOWN; self.grid.len()]
    }
    pub fn solve(&self, initial: &[u8], limits: &SolveLimits) -> SolveOutcome {
        assert_eq!(initial.len(), self.grid.len());
        let mut session = Session::new(self.grid.clone(), &self.clues);
        let mut assumptions = session.enabled.clone();
        assumptions.extend(initial.iter().enumerate().filter_map(|(i, &v)| match v {
            BLACK => Some(session.black[i]),
            WHITE => Some(!session.black[i]),
            _ => None,
        }));
        session.enumerate(&assumptions, limits)
    }
}

/// Incremental satisfiability queries under a fixed player position.
/// No model is blocked, so a later query cannot inherit an artificial exclusion.
pub(crate) struct PositionSolver {
    session: Session,
    assumptions: Vec<Lit>,
}
impl PositionSolver {
    pub(crate) fn new(grid: &Grid, clues: &[(usize, Clue)], marks: &[u8]) -> Self {
        assert_eq!(marks.len(), grid.len());
        let session = Session::new(grid.clone(), clues);
        let mut assumptions = session.enabled.clone();
        assumptions.extend(
            marks
                .iter()
                .enumerate()
                .filter_map(|(i, &value)| match value {
                    BLACK => Some(session.black[i]),
                    WHITE => Some(!session.black[i]),
                    _ => None,
                }),
        );
        Self {
            session,
            assumptions,
        }
    }
    pub(crate) fn query(
        &mut self,
        extra: Option<(usize, u8)>,
        deadline: Option<f64>,
    ) -> SolveOutcome {
        if let Some((cell, value)) = extra {
            self.assumptions
                .push(self.session.black[cell].apply_sign(value == BLACK));
        }
        let result = self.session.enumerate(
            &self.assumptions,
            &SolveLimits {
                max_solutions: 1,
                deadline,
                max_nodes: u64::MAX,
            },
        );
        if extra.is_some() {
            self.assumptions.pop();
        }
        result
    }
}

/// Reuses the constraint database and learned clauses throughout clue removal.
/// The known solution is validated and blocked once; UNSAT proves uniqueness.
pub(crate) struct UniquenessChecker {
    session: Session,
    solution: Vec<u8>,
}
impl UniquenessChecker {
    pub(crate) fn new(grid: &Grid, clues: &[(usize, Clue)], solution: &[u8]) -> Option<Self> {
        if solution.len() != grid.len() || !validate(grid, solution, clues).is_empty() {
            return None;
        }
        let mut session = Session::new(grid.clone(), clues);
        session.exclude(solution);
        Some(Self {
            session,
            solution: solution.to_vec(),
        })
    }
    pub(crate) fn check(&mut self, keep: &[bool], limits: &SolveLimits) -> SolveOutcome {
        assert_eq!(keep.len(), self.session.enabled.len());
        let assumptions: Vec<_> = self
            .session
            .enabled
            .iter()
            .zip(keep)
            .map(|(&lit, &enabled)| lit.apply_sign(enabled))
            .collect();
        let single = SolveLimits {
            max_solutions: 1,
            deadline: limits.deadline,
            max_nodes: limits.max_nodes,
        };
        let mut result = self.session.enumerate(&assumptions, &single);
        result.solutions.insert(0, self.solution.clone());
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::clue_of;

    /// Build the full clue set of a solution and check the solver recovers it.
    fn round_trip(w: usize, h: usize, blacks: &[usize]) {
        let grid = Grid::new(w, h);
        let mut cells = vec![WHITE; grid.len()];
        for &b in blacks {
            cells[b] = BLACK;
        }
        let mut clues = Vec::new();
        for i in 0..grid.len() {
            if cells[i] == WHITE {
                let clue = clue_of(&cells, &grid, i);
                if !clue.is_empty() {
                    clues.push((i, clue));
                }
            }
        }
        let solver = Solver::new(grid, &clues);
        let outcome = solver.solve(&solver.fresh_state(), &SolveLimits::unique_check(None));
        assert!(!outcome.aborted);
        assert_eq!(outcome.solutions.len(), 1, "puzzle is not unique");
        assert_eq!(
            outcome.solutions[0], cells,
            "solver returned a different grid"
        );
    }

    #[test]
    fn single_black_cell() {
        round_trip(3, 3, &[4]);
    }

    #[test]
    fn two_black_cells() {
        round_trip(3, 3, &[0, 1]);
    }

    #[test]
    fn snake_on_5x5() {
        round_trip(5, 5, &[0, 1, 2, 7, 12, 17, 22, 23, 24]);
    }

    #[test]
    fn search_checks_connectivity_after_a_white_branch() {
        let solver = Solver::new(Grid::new(3, 1), &[]);
        let result = solver.solve(&[BLACK, UNKNOWN, BLACK], &SolveLimits::unique_check(None));
        assert!(result.is_unique());
        assert_eq!(result.solutions[0], vec![BLACK, BLACK, BLACK]);
        let result = solver.solve(&[BLACK, WHITE, BLACK], &SolveLimits::unique_check(None));
        assert!(result.solutions.is_empty());
    }

    #[test]
    fn search_solutions_match_exhaustive_boards() {
        use crate::{model::validate, rng::Rng};
        let grid = Grid::new(3, 3);
        let boards: Vec<Vec<u8>> = (0..512)
            .map(|mask| {
                (0..9)
                    .map(|cell| {
                        if mask & (1 << cell) != 0 {
                            BLACK
                        } else {
                            WHITE
                        }
                    })
                    .collect::<Vec<u8>>()
            })
            .filter(|board| validate(&grid, board, &[]).is_empty())
            .collect();
        let mut rng = Rng::new(17);
        for _ in 0..32 {
            let solution = &boards[rng.below(boards.len())];
            let clues: Vec<_> = (0..9)
                .filter(|&i| solution[i] == WHITE && rng.below(2) == 0)
                .map(|i| (i, clue_of(solution, &grid, i)))
                .collect();
            let mut marks = vec![UNKNOWN; 9];
            for _ in 0..2 {
                let cell = rng.below(9);
                marks[cell] = solution[cell];
            }
            let mut expected: Vec<_> = boards
                .iter()
                .filter(|board| {
                    marks
                        .iter()
                        .zip(board.iter())
                        .all(|(&mark, &value)| mark == UNKNOWN || mark == value)
                        && validate(&grid, board, &clues).is_empty()
                })
                .cloned()
                .collect();
            let solver = Solver::new(grid.clone(), &clues);
            let mut actual = solver.solve(
                &marks,
                &SolveLimits {
                    max_solutions: 512,
                    deadline: None,
                    max_nodes: u64::MAX,
                },
            );
            assert!(!actual.aborted);
            actual.solutions.sort();
            expected.sort();
            assert_eq!(actual.solutions, expected);
        }
    }

    #[test]
    fn search_checks_initial_black_squares() {
        let solver = Solver::new(Grid::new(2, 2), &[]);
        let result = solver.solve(&[BLACK; 4], &SolveLimits::unique_check(None));
        assert!(result.solutions.is_empty());
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    #[test]
    fn repeated_ambiguous_queries_keep_the_alternative_models() {
        let grid = Grid::new(3, 1);
        let clues = [(0, vec![1]), (2, vec![1])];
        let mut checker = UniquenessChecker::new(&grid, &clues, &[WHITE, BLACK, WHITE]).unwrap();
        let limits = SolveLimits::unique_check(None);
        assert!(checker.check(&[true, true], &limits).is_unique());
        for _ in 0..3 {
            assert!(!checker.check(&[false, true], &limits).is_unique());
        }
        assert!(checker.check(&[true, true], &limits).is_unique());
        assert!(!checker.check(&[false, false], &limits).is_unique());
    }
    #[test]
    fn incremental_removals_match_independent_exhaustive_validation() {
        use crate::{model::clue_of, rng::Rng};
        let grid = Grid::new(3, 3);
        let boards: Vec<Vec<u8>> = (0..512)
            .map(|m| {
                (0..9)
                    .map(|i| if m & (1 << i) != 0 { BLACK } else { WHITE })
                    .collect()
            })
            .filter(|b: &Vec<u8>| validate(&grid, b, &[]).is_empty())
            .collect();
        let mut rng = Rng::new(38);
        for _ in 0..30 {
            let solution = &boards[rng.below(boards.len())];
            let clues: Vec<_> = (0..9)
                .filter(|&i| solution[i] == WHITE)
                .map(|i| (i, clue_of(solution, &grid, i)))
                .collect();
            let mut checker = UniquenessChecker::new(&grid, &clues, solution).unwrap();
            for _ in 0..20 {
                let keep: Vec<_> = clues.iter().map(|_| rng.below(2) == 0).collect();
                let active: Vec<_> = clues
                    .iter()
                    .zip(&keep)
                    .filter(|(_, &k)| k)
                    .map(|(c, _)| c.clone())
                    .collect();
                let expected = boards
                    .iter()
                    .filter(|b| validate(&grid, b, &active).is_empty())
                    .count();
                let result = checker.check(&keep, &SolveLimits::unique_check(None));
                assert!(!result.aborted);
                assert_eq!(result.is_unique(), expected == 1);
                assert!(result
                    .solutions
                    .iter()
                    .all(|b| validate(&grid, b, &active).is_empty()));
            }
        }
    }
    #[test]
    fn invalid_known_solutions_cannot_certify_uniqueness() {
        assert!(UniquenessChecker::new(&Grid::new(3, 1), &[], &[BLACK, WHITE, BLACK]).is_none());
    }
    #[test]
    fn finding_one_model_does_not_prove_uniqueness() {
        let solver = Solver::new(Grid::new(3, 1), &[]);
        let result = solver.solve(
            &solver.fresh_state(),
            &SolveLimits {
                max_solutions: 1,
                deadline: None,
                max_nodes: u64::MAX,
            },
        );
        assert_eq!(result.solutions.len(), 1);
        assert!(!result.complete && !result.is_unique());
    }
    #[test]
    fn limits_never_certify_uniqueness() {
        let solver = Solver::new(Grid::new(3, 1), &[(0, vec![1]), (2, vec![1])]);
        for limits in [
            SolveLimits::unique_check(None).with_node_budget(0),
            SolveLimits::unique_check(Some(now_ms() - 1.0)),
        ] {
            let result = solver.solve(&solver.fresh_state(), &limits);
            assert!(result.aborted);
            assert!(!result.is_unique());
        }
    }
}
