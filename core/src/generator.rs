//! Puzzle generation.
//!
//! Strategy:
//!   1. grow a random *valid* solution: black cells stay orthogonally connected,
//!      no 2x2 is fully black and no black cell is completely surrounded by
//!      black cells (such a cell would not be pinned down by any clue);
//!   2. take every white cell that has at least one black neighbour as a clue
//!      candidate 闂?at this point the clue set is maximal;
//!   3. prove the maximal clue set has a unique solution (otherwise the shape is
//!      unusable and we grow another one);
//!   4. remove clues one by one in random order, keeping every removal that
//!      still leaves a unique solution. The result is a puzzle that is
//!      *minimal*: dropping any single remaining clue makes it ambiguous.
//!
//! Every accepted puzzle is therefore guaranteed to have exactly one solution,
//! and that solution is the shape the generator started from.

use std::time::Duration;

use crate::clock::{now_ms, Timer};
use crate::config::Settings;
use crate::model::{
    clue_is_showable, clue_of, largest_blank_region, largest_clueless_region, validate, Clue,
    Grid, Puzzle, BLACK, ORTHO, UNKNOWN, WHITE,
};
use crate::rng::Rng;
use crate::solver::{SolveLimits, Solver};

pub struct GenConfig {
    pub w: usize,
    pub h: usize,
    pub max_attempts: usize,
    /// Fraction of the grid that should become black.
    pub black_lo: f64,
    pub black_hi: f64,
    /// Budget for the whole generation, as an absolute [`crate::clock::now_ms`]
    /// timestamp in milliseconds.
    pub deadline: Option<f64>,
    /// Nodes for the whole generation, shared adaptively between proofs.
    pub node_budget: u64,
    /// Time for the whole generation, shared adaptively between proofs.
    pub time_budget: Duration,
    /// Floor for a single uniqueness proof, so a cheap check is never starved.
    pub check_min_nodes: u64,
    /// How much a single proof may borrow over its fair share of the pool.
    pub check_slack: u64,
    /// Reject a solution whose blankest region (cells with no black neighbour,
    /// i.e. cells that produce no clue) is bigger than this many cells. This
    /// only rejects shapes; it never adds clues.
    pub max_clueless_region: usize,
}

/// A shared budget pool. Instead of a hard per-proof cap, each proof gets
/// `remaining / checks_left * slack`, so a proof that needs a little more than
/// its fair share can still finish while the pool has room.
struct Budget {
    nodes: u64,
    deadline: Option<f64>,
    slack: u64,
    min_nodes: u64,
}

impl Budget {
    fn new(cfg: &GenConfig) -> Budget {
        Budget {
            nodes: cfg.node_budget,
            deadline: cfg.deadline.or_else(|| {
                (!cfg.time_budget.is_zero())
                    .then(|| now_ms() + cfg.time_budget.as_secs_f64() * 1000.0)
            }),
            slack: cfg.check_slack.max(1),
            min_nodes: cfg.check_min_nodes.max(1),
        }
    }

    /// Limits for one proof, given how many proofs are still to come.
    fn share(&self, checks_left: usize) -> SolveLimits {
        let left = checks_left.max(1) as u64;
        let nodes = (self.nodes / left)
            .saturating_mul(self.slack)
            .max(self.min_nodes)
            .min(self.nodes.max(1));
        let deadline = self.deadline.map(|end| {
            let remaining = (end - now_ms()).max(0.0);
            now_ms() + remaining / left as f64 * self.slack as f64
        });
        SolveLimits::unique_check(deadline).with_node_budget(nodes)
    }

    fn exhausted(&self) -> bool {
        self.nodes == 0 || self.deadline.map_or(false, |end| now_ms() >= end)
    }

    fn spend(&mut self, nodes: u64) {
        self.nodes = self.nodes.saturating_sub(nodes);
    }
}

impl GenConfig {
    /// Default configuration for a w x h board.
    #[cfg(test)]
    pub fn for_size(w: usize, h: usize) -> GenConfig {
        let area = w * h;
        GenConfig {
            w,
            h,
            max_attempts: 200,
            black_lo: 0.40,
            black_hi: 0.50,
            deadline: None,
            node_budget: 150_000,
            time_budget: Duration::from_millis(2_000),
            check_min_nodes: 3_000,
            check_slack: 3,
            max_clueless_region: (area as f64 * 0.15).ceil() as usize,
        }
    }

    /// Build a configuration from user settings (board size, density, budgets).
    pub fn from_settings(settings: &Settings) -> GenConfig {
        let area = settings.size * settings.size;
        GenConfig {
            w: settings.size,
            h: settings.size,
            max_attempts: settings.max_attempts,
            black_lo: settings.density_lo,
            black_hi: settings.density_hi,
            deadline: None,
            node_budget: settings.node_budget,
            time_budget: Duration::from_millis(settings.time_budget_ms),
            check_min_nodes: settings.check_min_nodes,
            check_slack: settings.check_slack,
            max_clueless_region: (area as f64 * settings.max_clueless_fraction).ceil() as usize,
        }
    }
}

/// Statistics of one generation run, useful for the CLI.
pub struct GenStats {
    pub attempts: usize,
    pub clue_count: usize,
    pub black_count: usize,
    pub elapsed: Duration,
    pub removals: usize,
    /// Total search nodes spent proving uniqueness while minimising clues.
    pub nodes: u64,
    /// Time spent inside the solver (diagnostic).
    pub solve_time: Duration,
    /// How many uniqueness checks the minimisation ran (diagnostic).
    pub checks: usize,
    /// Most nodes any single check used (diagnostic).
    pub max_check_nodes: u64,
    /// Biggest clue-free patch of the finished board.
    pub blank_region: usize,
    /// Biggest patch of the solution that carries no clue at all.
    pub clueless_region: usize,
}

pub struct GenResult {
    pub puzzle: Puzzle,
    pub stats: GenStats,
}

pub fn generate_with(rng: &mut Rng, cfg: &GenConfig) -> Option<GenResult> {
    let started = Timer::start();
    let grid = Grid::new(cfg.w, cfg.h);
    let n = grid.len();
    let mut attempts = 0usize;
    // one pool for the whole generation, shared adaptively between proofs
    let mut budget = Budget::new(cfg);
    let mut spent_nodes = 0u64;
    let mut spent_time = Duration::ZERO;

    for _ in 0..cfg.max_attempts {
        if budget.exhausted() {
            return None;
        }
        attempts += 1;

        let fraction = cfg.black_lo + (cfg.black_hi - cfg.black_lo) * (rng.below(1000) as f64 / 1000.0);
        let target = (n as f64 * fraction) as usize;
        let Some(solution) = random_solution(rng, &grid, target) else {
            continue;
        };
        // no large patches that would carry no clue at all
        if largest_clueless_region(&grid, &solution) > cfg.max_clueless_region {
            continue;
        }

        // (2) maximal clue set: every white cell with at least one black
        // neighbour, except the ones whose clue would be 8
        let mut candidates: Vec<(usize, Clue)> = Vec::new();
        for i in 0..n {
            if solution[i] == WHITE {
                let clue = clue_of(&solution, &grid, i);
                if clue_is_showable(&clue) {
                    candidates.push((i, clue));
                }
            }
        }
        if candidates.is_empty() {
            continue;
        }
        rng.shuffle(&mut candidates);

        // (3) the maximal clue set must already pin the solution down. This is
        // only a shape filter, so it draws a small share of the pool: a shape
        // that cannot be pinned down quickly is dropped instead of burning time.
        let start = vec![UNKNOWN; n];
        let shape_limits = budget.share(10);
        let solver = Solver::new(grid.clone(), &candidates);
        let t0 = Timer::start();
        let outcome = solver.solve(&start, &shape_limits);
        spent_time += t0.elapsed();
        budget.spend(outcome.nodes);
        spent_nodes += outcome.nodes;
        if !outcome.is_unique() {
            continue;
        }

        // (4) strip clues from the maximal set down to a minimal one: a clue is
        // dropped whenever the puzzle still has exactly one solution, so no
        // single clue of the result can be removed any more.
        //
        // This is the dual of "start blank and add random clues until the
        // solution is unique", and it reaches the minimal set far more cheaply:
        // the intermediate puzzles are dense, so refuting them is quick, while
        // the adding direction spends its time on near-unique puzzles, which is
        // exactly where the search is expensive.
        let (clues, removals, nodes_used, solve_time, checks, max_check_nodes) =
            minimise(&grid, &candidates, &start, cfg, rng, &mut budget);
        spent_nodes += nodes_used;
        spent_time += solve_time;
        let clue_count = clues.len();
        let blank_region = blank_region(&grid, &solution, &clues);
        let black_count = solution.iter().filter(|&&c| c == BLACK).count();
        let clueless_region = largest_clueless_region(&grid, &solution);
        let puzzle = Puzzle::new(grid.clone(), clues, solution, 0);

        // Safety net: the stored solution must satisfy the rules. Uniqueness was
        // already proven by the minimisation loop - the clue set it returns is
        // always one whose uniqueness check succeeded.
        if !validate(&puzzle.grid, &puzzle.solution, &puzzle.clues).is_empty() {
            continue;
        }

        return Some(GenResult {
            puzzle,
            stats: GenStats {
                attempts,
                clue_count,
                black_count,
                elapsed: started.elapsed(),
                removals,
                nodes: spent_nodes,
                solve_time: spent_time,
                blank_region,
                clueless_region,
                checks,
                max_check_nodes,
            },
        });
    }
    None
}

/// How many of the orthogonal neighbours of `cell` are already black.
fn black_neighbors(cells: &[u8], grid: &Grid, cell: usize) -> u32 {
    let (x, y) = grid.xy(cell);
    let mut count = 0;
    for &(dx, dy) in ORTHO.iter() {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx < 0 || ny < 0 || nx as usize >= grid.w || ny as usize >= grid.h {
            continue;
        }
        if cells[grid.idx(nx as usize, ny as usize)] == BLACK {
            count += 1;
        }
    }
    count
}

/// Biggest clue-free patch of the board for a given clue set.
fn blank_region(grid: &Grid, solution: &[u8], clues: &[(usize, Clue)]) -> usize {
    let mut is_clue = vec![false; grid.len()];
    for (i, _) in clues.iter() {
        is_clue[*i] = true;
    }
    largest_blank_region(grid, solution, &is_clue)
}

/// One greedy pass over the clue candidates in random order: a clue is dropped
/// whenever the puzzle still has exactly one solution. Because every kept clue
/// was tested against a superset of the final clue set, the result is
/// *inclusion-minimal* - no single clue of it can be removed any more.
///
/// Each proof draws an adaptive share of the shared budget. A proof that runs
/// out is not judged for good: the clue stays in and is retried once at the end
/// with whatever budget is left, so a near miss is not treated like a hopeless
/// case.
///
/// Returns the kept clues, how many were removed, the nodes used and the time
/// spent inside the solver.
fn minimise(
    grid: &Grid,
    candidates: &[(usize, Clue)],
    start: &[u8],
    cfg: &GenConfig,
    rng: &mut Rng,
    budget: &mut Budget,
) -> (Vec<(usize, Clue)>, usize, u64, Duration, usize, u64) {
    let mut keep = vec![true; candidates.len()];
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    rng.shuffle(&mut order);

    let mut removals = 0usize;
    let mut nodes = 0u64;
    let mut solve_time = Duration::ZERO;
    let mut checks = 0usize;
    let mut max_check_nodes = 0u64;
    let mut retry: Vec<usize> = Vec::new();

    // `rounds` runs the queue once; the second round only retries the clues
    // whose proof ran out of budget the first time.
    for round in 0..2 {
        let queue: Vec<usize> = if round == 0 {
            order.clone()
        } else {
            std::mem::take(&mut retry)
        };
        let mut left = queue.len();
        for &i in &queue {
            if budget.exhausted() {
                return finish(candidates, keep, removals, nodes, solve_time, checks, max_check_nodes);
            }
            left = left.saturating_sub(1);
            keep[i] = false;
            let trial: Vec<(usize, Clue)> = candidates
                .iter()
                .enumerate()
                .filter(|(k, _)| keep[*k])
                .map(|(_, c)| c.clone())
                .collect();
            let solver = Solver::new(grid.clone(), &trial);
            let t0 = Timer::start();
            let outcome = solver.solve(start, &budget.share(left + 1));
            solve_time += t0.elapsed();
            nodes += outcome.nodes;
            budget.spend(outcome.nodes);
            checks += 1;
            max_check_nodes = max_check_nodes.max(outcome.nodes);
            if outcome.is_unique() {
                removals += 1;
            } else {
                keep[i] = true;
                if outcome.aborted && round == 0 {
                    retry.push(i);
                }
            }
        }
        if retry.is_empty() {
            break;
        }
    }
    let _ = cfg;
    finish(candidates, keep, removals, nodes, solve_time, checks, max_check_nodes)
}

fn finish(
    candidates: &[(usize, Clue)],
    keep: Vec<bool>,
    removals: usize,
    nodes: u64,
    solve_time: Duration,
    checks: usize,
    max_check_nodes: u64,
) -> (Vec<(usize, Clue)>, usize, u64, Duration, usize, u64) {
    let clues: Vec<(usize, Clue)> = candidates
        .iter()
        .enumerate()
        .filter(|(k, _)| keep[*k])
        .map(|(_, c)| c.clone())
        .collect();
    (clues, removals, nodes, solve_time, checks, max_check_nodes)
}

/// Grow a random solution whose black cells form a **tree**.
///
/// Each new black cell must touch the existing region on exactly one side, so
/// the black cells are an acyclic connected set (a tree in the grid graph).
/// That single rule buys every structural constraint Tapa needs for free:
/// a 2x2 block, a black cell surrounded by black cells and a white cell ringed
/// by black cells (a clue of 8) all contain a cycle, so none of them can occur.
///
/// The growth is aimed at whatever white area is currently furthest from the
/// tree, so the tree reaches into every corner instead of leaving one big
/// empty patch behind.
fn random_solution(rng: &mut Rng, grid: &Grid, target: usize) -> Option<Vec<u8>> {
    let n = grid.len();
    let mut cells = vec![WHITE; n];
    let mut blacks: Vec<usize> = Vec::new();
    let mut distance = vec![u32::MAX; n];
    let mut queue: Vec<usize> = Vec::with_capacity(n);

    let start = rng.below(n);
    cells[start] = BLACK;
    blacks.push(start);

    while blacks.len() < target {
        // distance from the black region to every white cell
        for d in distance.iter_mut() {
            *d = u32::MAX;
        }
        queue.clear();
        for &b in blacks.iter() {
            distance[b] = 0;
            queue.push(b);
        }
        let mut head = 0usize;
        while head < queue.len() {
            let c = queue[head];
            head += 1;
            let next = distance[c] + 1;
            for nb in grid.ortho(c).iter().flatten() {
                if distance[*nb] == u32::MAX {
                    distance[*nb] = next;
                    queue.push(*nb);
                }
            }
        }

        // the white cell that is furthest away decides where to grow next
        let mut goal = None;
        let mut best_distance = 0u32;
        for i in 0..n {
            if cells[i] == WHITE && distance[i] != u32::MAX && distance[i] > best_distance {
                best_distance = distance[i];
                goal = Some(i);
            }
        }
        let Some(goal) = goal else { break };

        // among the legal tree extensions, take the one closest to that goal
        let mut choice = None;
        let mut best_gap = i32::MAX;
        for &b in blacks.iter() {
            for nb in grid.ortho(b).iter().flatten() {
                if cells[*nb] != WHITE || black_neighbors(&cells, grid, *nb) != 1 {
                    continue;
                }
                let gap = chebyshev(grid, *nb, goal);
                if gap < best_gap || (gap == best_gap && rng.below(3) == 0) {
                    best_gap = gap;
                    choice = Some(*nb);
                }
            }
        }
        let Some(cell) = choice else { break };
        cells[cell] = BLACK;
        blacks.push(cell);
    }

    if blacks.len() * 4 < target * 3 || blacks.len() < 6 {
        return None;
    }
    Some(cells)
}

/// Chebyshev distance between two cells.
fn chebyshev(grid: &Grid, a: usize, b: usize) -> i32 {
    let (ax, ay) = grid.xy(a);
    let (bx, by) = grid.xy(b);
    (ax as i32 - bx as i32).abs().max((ay as i32 - by as i32).abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::Solver;

    fn check_generated(w: usize, h: usize, seed: u64, strict_minimal: bool) {
        let mut rng = Rng::new(seed);
        let mut cfg = GenConfig::for_size(w, h);
        cfg.deadline = Some(now_ms() + 120_000.0);
        if strict_minimal {
            // a budget abort keeps a clue, so unbounded runs are needed to
            // assert strict minimality
            cfg.node_budget = u64::MAX;
            cfg.time_budget = Duration::ZERO;
            cfg.check_min_nodes = u64::MAX;
        }
        let result = generate_with(&mut rng, &cfg).expect("generation failed");
        let puzzle = result.puzzle;

        // the stored solution obeys every rule
        assert!(validate(&puzzle.grid, &puzzle.solution, &puzzle.clues).is_empty());
        assert!(puzzle.clues.len() >= 4);

        // exactly one solution
        let solver = Solver::new(puzzle.grid.clone(), &puzzle.clues);
        let outcome = solver.solve(&solver.fresh_state(), &SolveLimits::unique_check(None));
        assert!(!outcome.aborted);
        assert_eq!(outcome.solutions.len(), 1, "puzzle is ambiguous");
        assert_eq!(outcome.solutions[0], puzzle.solution);

        // no clue of 8 is ever shown
        for (_, clue) in puzzle.clues.iter() {
            assert_ne!(clue.as_slice(), [8u8], "clue 8 must not appear");
            assert!(clue_is_showable(clue));
        }

        if !strict_minimal {
            return;
        }

        // minimality: dropping any single clue must make the puzzle ambiguous
        for k in 0..puzzle.clues.len() {
            let mut fewer = puzzle.clues.clone();
            fewer.remove(k);
            let solver = Solver::new(puzzle.grid.clone(), &fewer);
            let outcome = solver.solve(&solver.fresh_state(), &SolveLimits::unique_check(None));
            assert!(!outcome.aborted);
            assert!(
                !outcome.is_unique(),
                "clue {k} is redundant, the set is not minimal"
            );
        }
    }

    #[test]
    fn one_step_deductions_match_the_solution() {
        use crate::model::{clue_deductions, UNKNOWN, WHITE};
        let mut rng = Rng::new(2024);
        let cfg = GenConfig::for_size(20, 20);
        let result = generate_with(&mut rng, &cfg).expect("generation failed");
        let puzzle = result.puzzle;

        let mut marks = vec![UNKNOWN; puzzle.grid.len()];
        for (i, _) in puzzle.clues.iter() {
            marks[*i] = WHITE;
        }
        let deduced = clue_deductions(&puzzle.grid, &marks, &puzzle.clues);
        println!("one step from a fresh board filled {} cells", deduced.len());
        for (cell, value) in &deduced {
            assert_eq!(
                *value, puzzle.solution[*cell],
                "one step deduced a wrong value for cell {cell}"
            );
        }

        // applying them and stepping again must stay sound and stop eventually
        for (cell, value) in &deduced {
            marks[*cell] = *value;
        }
        let more = clue_deductions(&puzzle.grid, &marks, &puzzle.clues);
        for (cell, value) in &more {
            assert_eq!(*value, puzzle.solution[*cell]);
        }
        println!("second step filled {} more", more.len());
    }

    #[test]
    fn generates_unique_small() {
        check_generated(6, 6, 1, true);
    }

    #[test]
    fn generates_unique_medium() {
        check_generated(10, 10, 42, true);
    }

    #[test]
    fn generates_unique_20x20() {
        check_generated(20, 20, 2024, false);
    }
}




