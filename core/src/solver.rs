//! Tapa solver: constraint propagation plus backtracking search.
//!
//! Propagation rules (run to a fixpoint before every branch):
//!   1. a clue cell is always empty;
//!   2. arc consistency on every clue: keep the neighbour masks that still fit
//!      the partially filled surroundings, then assign every neighbour that is
//!      black (or white) in *all* surviving masks;
//!   3. no 2x2 block may become black: three black cells force the fourth white;
//!   4. connectivity: the cells that could still become black are split into
//!      orthogonal components; the black cells must all end up in one of them,
//!      so as soon as one component holds a black cell every other component is
//!      forced white;
//!   5. a black cell that can never reach another black cell is a contradiction.
//!
//! The set of neighbour masks that survive for a clue is kept as a 256-bit
//! bitset, so "which masks still fit" is a handful of AND instructions instead
//! of a loop over up to 32 patterns. Buffers used by the component analysis are
//! allocated once per solve instead of once per propagation.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::clock::now_ms;
use crate::model::{clue_masks, Clue, Grid, BLACK, UNKNOWN, WHITE};

/// Cache of clue -> neighbour masks. Clues repeat a lot across solver instances.
fn mask_cache() -> &'static Mutex<HashMap<Clue, Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<Clue, Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_masks(clue: &[u8]) -> Vec<u8> {
    let mut cache = mask_cache().lock().unwrap();
    if let Some(m) = cache.get(clue) {
        return m.clone();
    }
    let m = clue_masks(clue);
    cache.insert(clue.to_vec(), m.clone());
    m
}

/// A set of neighbour masks: bit `m` of the 256-bit number means "mask m is
/// still possible".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MaskSet([u64; 4]);

impl MaskSet {
    const EMPTY: MaskSet = MaskSet([0; 4]);

    #[inline]
    fn insert(&mut self, m: u8) {
        self.0[(m >> 6) as usize] |= 1u64 << (m & 63);
    }

    #[inline]
    fn and(self, other: MaskSet) -> MaskSet {
        MaskSet([
            self.0[0] & other.0[0],
            self.0[1] & other.0[1],
            self.0[2] & other.0[2],
            self.0[3] & other.0[3],
        ])
    }

    #[inline]
    fn is_empty(self) -> bool {
        self.0[0] | self.0[1] | self.0[2] | self.0[3] == 0
    }

    #[inline]
    fn intersects(self, other: MaskSet) -> bool {
        (self.0[0] & other.0[0])
            | (self.0[1] & other.0[1])
            | (self.0[2] & other.0[2])
            | (self.0[3] & other.0[3])
            != 0
    }

    #[inline]
    fn count(self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }

    fn to_vec(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.count() as usize);
        for word in 0..4 {
            let mut bits = self.0[word];
            while bits != 0 {
                let bit = bits.trailing_zeros();
                out.push((word * 64 + bit as usize) as u8);
                bits &= bits - 1;
            }
        }
        out
    }
}

/// The mask sets of one clue cell: all feasible masks, and the subsets that
/// have a given neighbour black / white.
struct ClueSets {
    all: MaskSet,
    with_black: [MaskSet; 8],
    with_white: [MaskSet; 8],
}

/// Reusable buffers for the component analysis.
struct Scratch {
    comp: Vec<u32>,
    has_black: Vec<bool>,
    stack: Vec<usize>,
    /// Cells that just turned black and still need their 2x2 windows checked.
    new_blacks: Vec<usize>,
    /// Cells that turned black since the component analysis last ran.
    pending_blacks: Vec<usize>,
    comp_black_groups: usize,
    main_comp: u32,
    /// Set when a cell turned white, which can split components.
    comp_dirty: bool,
    /// Most constrained clue found by the last propagation pass.
    best: Option<(usize, MaskSet, u32)>,
}

impl Scratch {
    fn new(n: usize) -> Scratch {
        Scratch {
            comp: vec![u32::MAX; n],
            has_black: Vec::with_capacity(n),
            stack: Vec::with_capacity(n),
            new_blacks: Vec::new(),
            pending_blacks: Vec::new(),
            comp_black_groups: 0,
            main_comp: u32::MAX,
            comp_dirty: true,
            best: None,
        }
    }
}

pub struct SolveLimits {
    pub max_solutions: usize,
    /// Hard wall clock budget as an absolute [`crate::clock::now_ms`] timestamp
    /// in milliseconds; `None` means "no limit".
    pub deadline: Option<f64>,
    /// Hard node budget; `u64::MAX` means "no limit".
    pub max_nodes: u64,
}

impl SolveLimits {
    pub fn unique_check(deadline: Option<f64>) -> SolveLimits {
        SolveLimits {
            max_solutions: 2,
            deadline,
            max_nodes: u64::MAX,
        }
    }

    pub fn with_node_budget(mut self, max_nodes: u64) -> SolveLimits {
        self.max_nodes = max_nodes;
        self
    }
}

/// Result of a search.
pub struct SolveOutcome {
    pub solutions: Vec<Vec<u8>>,
    pub nodes: u64,
    pub aborted: bool,
}

impl SolveOutcome {
    /// True only when exactly one solution exists and the search proved it.
    pub fn is_unique(&self) -> bool {
        !self.aborted && self.solutions.len() == 1
    }
}

pub struct Solver {
    grid: Grid,
    clue_cells: Vec<usize>,
    sets: Vec<ClueSets>,
    /// How many clues constrain each cell (used to pick branch cells).
    weight: Vec<u16>,
}

impl Solver {
    pub fn new(grid: Grid, clues: &[(usize, Clue)]) -> Solver {
        let mut clue_cells = Vec::with_capacity(clues.len());
        let mut sets = Vec::with_capacity(clues.len());
        let mut weight = vec![0u16; grid.len()];
        for (idx, clue) in clues {
            clue_cells.push(*idx);
            let neighbors = grid.neighbors(*idx);
            // A cell outside the grid is never black, so drop every mask that
            // would require one.
            let feasible: Vec<u8> = cached_masks(clue)
                .into_iter()
                .filter(|m| (0..8).all(|k| neighbors[k].is_some() || m & (1 << k) == 0))
                .collect();

            let mut all = MaskSet::EMPTY;
            let mut with_black = [MaskSet::EMPTY; 8];
            let mut with_white = [MaskSet::EMPTY; 8];
            for &m in &feasible {
                all.insert(m);
                for k in 0..8 {
                    if m & (1 << k) != 0 {
                        with_black[k].insert(m);
                    } else {
                        with_white[k].insert(m);
                    }
                }
            }
            sets.push(ClueSets {
                all,
                with_black,
                with_white,
            });

            weight[*idx] = weight[*idx].saturating_add(255); // never branch here
            for nb in neighbors.iter().flatten() {
                weight[*nb] = weight[*nb].saturating_add(1);
            }
        }
        Solver {
            grid,
            clue_cells,
            sets,
            weight,
        }
    }

    /// Initial state: everything unknown.
    pub fn fresh_state(&self) -> Vec<u8> {
        vec![UNKNOWN; self.grid.len()]
    }

    /// Search for up to `limits.max_solutions` solutions.
    pub fn solve(&self, initial: &[u8], limits: &SolveLimits) -> SolveOutcome {
        let mut ctx = Ctx {
            nodes: 0,
            max_nodes: limits.max_nodes,
            deadline: limits.deadline,
            aborted: false,
            scratch: Scratch::new(self.grid.len()),
        };
        let mut out = Vec::new();
        let mut state = initial.to_vec();
        self.search(&mut state, limits.max_solutions, &mut ctx, &mut out);
        SolveOutcome {
            solutions: out,
            nodes: ctx.nodes,
            aborted: ctx.aborted,
        }
    }

    fn search(
        &self,
        state: &mut Vec<u8>,
        max_solutions: usize,
        ctx: &mut Ctx,
        out: &mut Vec<Vec<u8>>,
    ) {
        if out.len() >= max_solutions || ctx.aborted {
            return;
        }
        if ctx.nodes >= ctx.max_nodes {
            ctx.aborted = true;
            return;
        }
        if ctx.nodes & 0x3FF == 0 {
            if let Some(deadline) = ctx.deadline {
                if now_ms() >= deadline {
                    ctx.aborted = true;
                    return;
                }
            }
        }
        ctx.nodes += 1;

        if !self.propagate_with(state, &mut ctx.scratch) {
            return;
        }

        let best = ctx.scratch.best.map(|(k, alive, _)| (k, alive));
        match self.pick_branch(state, best) {
            Branch::Done => out.push(state.clone()),
            Branch::Cell(cell) => {
                for value in [BLACK, WHITE] {
                    let mut next = state.clone();
                    next[cell] = value;
                    if value == BLACK {
                        ctx.scratch.new_blacks.push(cell);
                    }
                    self.search(&mut next, max_solutions, ctx, out);
                    if out.len() >= max_solutions || ctx.aborted {
                        return;
                    }
                }
            }
            Branch::Clue { cell, masks } => {
                let neighbors = *self.grid.neighbors(cell);
                for m in masks {
                    let mut next = state.clone();
                    for (b, nb) in neighbors.iter().enumerate() {
                        if let Some(nb) = nb {
                            let value = if m & (1 << b) != 0 { BLACK } else { WHITE };
                            next[*nb] = value;
                            if value == BLACK {
                                ctx.scratch.new_blacks.push(*nb);
                            }
                        }
                    }
                    self.search(&mut next, max_solutions, ctx, out);
                    if out.len() >= max_solutions || ctx.aborted {
                        return;
                    }
                }
            }
        }
    }

    /// Which piece of the puzzle to split on next.
    ///
    /// The strongest choice is the clue with the fewest surviving neighbour
    /// patterns: branching on those patterns assigns up to eight cells at once.
    /// That clue is found while propagating, so no second scan is needed. Only
    /// when every clue is fully determined do we fall back to a single cell
    /// (which can then only be a cell no clue constrains).
    fn pick_branch(&self, state: &[u8], best: Option<(usize, MaskSet)>) -> Branch {
        if let Some((k, alive)) = best {
            return Branch::Clue {
                cell: self.clue_cells[k],
                masks: alive.to_vec(),
            };
        }
        match self.pick_cell(state) {
            Some(cell) => Branch::Cell(cell),
            None => Branch::Done,
        }
    }

    /// The surviving neighbour masks of clue `k` for the given state.
    #[cfg(test)]
    #[inline]
    fn alive_for(&self, k: usize, state: &[u8]) -> MaskSet {
        let sets = &self.sets[k];
        let neighbors = self.grid.neighbors(self.clue_cells[k]);
        let mut alive = sets.all;
        for (b, nb) in neighbors.iter().enumerate() {
            let Some(nb) = nb else { continue };
            match state[*nb] {
                BLACK => alive = alive.and(sets.with_black[b]),
                WHITE => alive = alive.and(sets.with_white[b]),
                _ => {}
            }
        }
        alive
    }

    /// Most constrained unknown cell, with a nudge towards cells that already
    /// touch black cells (those decide connectivity).
    fn pick_cell(&self, state: &[u8]) -> Option<usize> {
        let mut best: Option<(u32, usize)> = None;
        for (i, &v) in state.iter().enumerate() {
            if v != UNKNOWN {
                continue;
            }
            let mut score = self.weight[i] as u32 * 4;
            for nb in self.grid.neighbors(i).iter().flatten() {
                if state[*nb] == BLACK {
                    score += 3;
                }
            }
            if best.map_or(true, |(s, _)| score > s) {
                best = Some((score, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Bring `state` to a fixpoint. Returns false on contradiction.
    /// (The search calls `propagate_with` directly to reuse its buffers.)
    #[cfg(test)]
    pub fn propagate(&self, state: &mut [u8]) -> bool {
        let mut scratch = Scratch::new(state.len());
        for (i, &v) in state.iter().enumerate() {
            if v == BLACK {
                scratch.new_blacks.push(i);
            }
        }
        self.propagate_with(state, &mut scratch)
    }

    /// Check the four 2x2 windows containing `cell`: a full black block is a
    /// contradiction, three black cells force the fourth white.
    fn check_2x2(
        &self,
        state: &mut [u8],
        cell: usize,
        changed: &mut bool,
        scratch: &mut Scratch,
    ) -> bool {
        if self.grid.w < 2 || self.grid.h < 2 {
            return true;
        }
        let (x, y) = self.grid.xy(cell);
        for dy in -1..=0i32 {
            for dx in -1..=0i32 {
                let x0 = x as i32 + dx;
                let y0 = y as i32 + dy;
                if x0 < 0 || y0 < 0 {
                    continue;
                }
                let (x0, y0) = (x0 as usize, y0 as usize);
                if x0 + 1 >= self.grid.w || y0 + 1 >= self.grid.h {
                    continue;
                }
                let quad = [
                    self.grid.idx(x0, y0),
                    self.grid.idx(x0 + 1, y0),
                    self.grid.idx(x0, y0 + 1),
                    self.grid.idx(x0 + 1, y0 + 1),
                ];
                let blacks = quad.iter().filter(|&&i| state[i] == BLACK).count();
                if blacks == 4 {
                    return false;
                }
                if blacks == 3 {
                    for &i in quad.iter() {
                        if state[i] == UNKNOWN {
                            state[i] = WHITE;
                            *changed = true;
                            scratch.comp_dirty = true;
                        }
                    }
                }
            }
        }
        true
    }

    fn propagate_with(&self, state: &mut [u8], scratch: &mut Scratch) -> bool {
        // the caller may hand us a state that differs from the previous call
        scratch.comp_dirty = true;
        loop {
            let mut changed = false;
            let mut black_added = false;
            let mut white_added = false;
            scratch.best = None;

            // (3) no 2x2 black block: only cells that just turned black can
            // have created one (the caller queues the cells it assigned)
            while let Some(cell) = scratch.new_blacks.pop() {
                black_added = true;
                if !self.check_2x2(state, cell, &mut changed, scratch) {
                    return false;
                }
                white_added = scratch.comp_dirty;
            }

            // (1) clue cells are empty
            for &c in &self.clue_cells {
                match state[c] {
                    UNKNOWN => {
                        state[c] = WHITE;
                        changed = true;
                        white_added = true;
                        scratch.comp_dirty = true;
                    }
                    BLACK => return false,
                    _ => {}
                }
            }

            // (2) arc consistency on the clues
            for (i, &cell) in self.clue_cells.iter().enumerate() {
                let sets = &self.sets[i];
                let neighbors = self.grid.neighbors(cell);
                let mut alive = sets.all;
                for (b, nb) in neighbors.iter().enumerate() {
                    let Some(nb) = nb else { continue };
                    match state[*nb] {
                        BLACK => alive = alive.and(sets.with_black[b]),
                        WHITE => alive = alive.and(sets.with_white[b]),
                        _ => {}
                    }
                }
                if alive.is_empty() {
                    return false;
                }
                let count = alive.count();
                if count >= 2 && scratch.best.map_or(true, |(_, _, c)| count < c) {
                    scratch.best = Some((i, alive, count));
                }
                for (b, nb) in neighbors.iter().enumerate() {
                    let Some(nb) = nb else { continue };
                    if state[*nb] != UNKNOWN {
                        continue;
                    }
                    if !alive.intersects(sets.with_black[b]) {
                        state[*nb] = WHITE;
                        changed = true;
                        white_added = true;
                        scratch.comp_dirty = true;
                    } else if !alive.intersects(sets.with_white[b]) {
                        state[*nb] = BLACK;
                        changed = true;
                        black_added = true;
                        scratch.new_blacks.push(*nb);
                        scratch.pending_blacks.push(*nb);
                    }
                }
            }

            // (4) connectivity of the potential black cells. The component
            // structure only changes when a cell becomes white; black cells
            // only change which component holds black.
            if black_added || white_added {
                if !self.propagate_connectivity(state, scratch, &mut changed) {
                    return false;
                }
            }

            if !changed {
                return true;
            }
        }
    }

    /// Update the component analysis and force the components that cannot hold
    /// black cells to be white.
    fn propagate_connectivity(
        &self,
        state: &mut [u8],
        scratch: &mut Scratch,
        changed: &mut bool,
    ) -> bool {
        let n = state.len();
        if scratch.comp_dirty {
            for c in scratch.comp.iter_mut() {
                *c = u32::MAX;
            }
            scratch.has_black.clear();
            scratch.comp_black_groups = 0;
            scratch.main_comp = u32::MAX;
            scratch.pending_blacks.clear();

            let mut stack = std::mem::take(&mut scratch.stack);
            for start in 0..n {
                if state[start] == WHITE || scratch.comp[start] != u32::MAX {
                    continue;
                }
                let cid = scratch.has_black.len() as u32;
                let mut has_black = false;
                scratch.comp[start] = cid;
                stack.push(start);
                while let Some(c) = stack.pop() {
                    if state[c] == BLACK {
                        has_black = true;
                    }
                    for nb in self.grid.ortho(c).iter().flatten() {
                        if scratch.comp[*nb] == u32::MAX && state[*nb] != WHITE {
                            scratch.comp[*nb] = cid;
                            stack.push(*nb);
                        }
                    }
                }
                if has_black {
                    scratch.comp_black_groups += 1;
                    scratch.main_comp = cid;
                }
                scratch.has_black.push(has_black);
            }
            scratch.stack = stack;
            scratch.comp_dirty = false;
        } else if !scratch.pending_blacks.is_empty() {
            for k in 0..scratch.pending_blacks.len() {
                let cell = scratch.pending_blacks[k];
                let cid = scratch.comp[cell];
                if cid != u32::MAX && !scratch.has_black[cid as usize] {
                    scratch.has_black[cid as usize] = true;
                    scratch.comp_black_groups += 1;
                    scratch.main_comp = cid;
                }
            }
            scratch.pending_blacks.clear();
        }

        if scratch.comp_black_groups > 1 {
            return false;
        }
        if scratch.comp_black_groups == 1 {
            let keep = scratch.main_comp;
            for i in 0..n {
                if state[i] == UNKNOWN && scratch.comp[i] != keep {
                    state[i] = WHITE;
                    *changed = true;
                    scratch.comp_dirty = true;
                }
            }
        }
        true
    }
}

struct Ctx {
    nodes: u64,
    max_nodes: u64,
    deadline: Option<f64>,
    aborted: bool,
    scratch: Scratch,
}

enum Branch {
    Done,
    Cell(usize),
    Clue { cell: usize, masks: Vec<u8> },
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
        assert_eq!(outcome.solutions[0], cells, "solver returned a different grid");
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
    fn rejects_illegal_states() {
        let grid = Grid::new(3, 3);
        // clue "1" at the centre, but two black neighbours -> contradiction
        let clues = vec![(4usize, vec![1u8])];
        let solver = Solver::new(grid, &clues);
        let mut state = solver.fresh_state();
        state[0] = BLACK;
        state[2] = BLACK;
        assert!(!solver.propagate(&mut state));
    }

    #[test]
    fn forced_2x2() {
        let grid = Grid::new(3, 3);
        let solver = Solver::new(grid, &[]);
        let mut state = solver.fresh_state();
        state[0] = BLACK;
        state[1] = BLACK;
        state[3] = BLACK;
        assert!(solver.propagate(&mut state));
        assert_eq!(state[4], WHITE);
    }

    #[test]
    fn mask_set_arithmetic() {
        let mut set = MaskSet::EMPTY;
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
        for m in [0u8, 5, 63, 64, 200, 255] {
            set.insert(m);
        }
        assert_eq!(set.count(), 6);
        assert!(!set.is_empty());
        assert!(set.intersects(set));
        assert_eq!(set.and(MaskSet::EMPTY), MaskSet::EMPTY);
        assert_eq!(set.and(set), set);
        let mut list = set.to_vec();
        list.sort_unstable();
        assert_eq!(list, vec![0, 5, 63, 64, 200, 255]);
    }

    #[test]
    fn arc_consistency_matches_naive_filter() {
        let grid = Grid::new(5, 5);
        let clues = vec![(12usize, vec![2u8, 1]), (7usize, vec![1u8])];
        let solver = Solver::new(grid.clone(), &clues);
        let mut state = solver.fresh_state();
        state[11] = BLACK;
        state[13] = WHITE;
        state[6] = WHITE;
        for (k, (cell, _)) in clues.iter().enumerate() {
            let bitset = solver.alive_for(k, &state);
            let mut naive: Vec<u8> = solver.sets[k]
                .all
                .to_vec()
                .into_iter()
                .filter(|&m| {
                    grid.neighbors(*cell).iter().enumerate().all(|(b, nb)| match nb {
                        None => true,
                        Some(nb) => match state[*nb] {
                            BLACK => m & (1 << b) != 0,
                            WHITE => m & (1 << b) == 0,
                            _ => true,
                        },
                    })
                })
                .collect();
            naive.sort_unstable();
            let mut got = bitset.to_vec();
            got.sort_unstable();
            assert_eq!(got, naive);
        }
    }
}
