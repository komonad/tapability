//! Deterministic Tapa deductions for human-oriented assessment.
use crate::model::{clue_masks, connection_bridges, Clue, Grid, BLACK, UNKNOWN, WHITE};

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
        }
    }
}

pub struct Reasoner {
    grid: Grid,
    clue_cells: Vec<usize>,
    sets: Vec<ClueSets>,
}

impl Reasoner {
    pub fn new(grid: Grid, clues: &[(usize, Clue)]) -> Reasoner {
        let mut clue_cells = Vec::with_capacity(clues.len());
        let mut sets = Vec::with_capacity(clues.len());
        for (idx, clue) in clues {
            clue_cells.push(*idx);
            let neighbors = grid.neighbors(*idx);
            // A cell outside the grid is never black, so drop every mask that
            // would require one.
            let feasible: Vec<u8> = clue_masks(clue)
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
        }
        Reasoner {
            grid,
            clue_cells,
            sets,
        }
    }

    /// Initial state: everything unknown.
    pub fn fresh_state(&self) -> Vec<u8> {
        vec![UNKNOWN; self.grid.len()]
    }

    /// Bring `state` to a fixpoint. Returns false on contradiction.
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
            if black_added || white_added || scratch.comp_dirty {
                if !self.propagate_connectivity(state, scratch, &mut changed) {
                    return false;
                }
            }

            if !changed {
                let bridges = connection_bridges(&self.grid, state);
                if bridges.is_empty() {
                    return true;
                }
                for cell in bridges {
                    state[cell] = BLACK;
                    scratch.new_blacks.push(cell);
                    scratch.pending_blacks.push(cell);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn propagation_agrees_with_all_sat_completions() {
        use crate::{
            model::{clue_of, validate},
            rng::Rng,
            solver::{SolveLimits, Solver},
        };
        let grid = Grid::new(3, 3);
        let mut rng = Rng::new(431);
        for mask in 0..512 {
            let solution: Vec<_> = (0..9)
                .map(|i| if mask & (1 << i) != 0 { BLACK } else { WHITE })
                .collect();
            if !validate(&grid, &solution, &[]).is_empty() {
                continue;
            }
            let clues: Vec<_> = (0..9)
                .filter(|&i| solution[i] == WHITE && rng.below(2) == 0)
                .map(|i| (i, clue_of(&solution, &grid, i)))
                .collect();
            let mut marks: Vec<_> = solution
                .iter()
                .map(|&v| if rng.below(4) == 0 { v } else { UNKNOWN })
                .collect();
            let exact = Solver::new(grid.clone(), &clues).solve(
                &marks,
                &SolveLimits {
                    max_solutions: 512,
                    deadline: None,
                    max_nodes: u64::MAX,
                },
            );
            assert!(!exact.aborted && !exact.solutions.is_empty());
            assert!(Reasoner::new(grid.clone(), &clues).propagate(&mut marks));
            for completion in exact.solutions {
                assert!(marks
                    .iter()
                    .zip(completion)
                    .all(|(&m, v)| m == UNKNOWN || m == v));
            }
        }
    }
    #[test]
    fn detects_local_contradictions_and_forces_black_square_escape() {
        let solver = Reasoner::new(Grid::new(3, 3), &[(4, vec![1])]);
        let mut state = solver.fresh_state();
        state[0] = BLACK;
        state[2] = BLACK;
        assert!(!solver.propagate(&mut state));
        let solver = Reasoner::new(Grid::new(3, 3), &[]);
        let mut state = solver.fresh_state();
        state[0] = BLACK;
        state[1] = BLACK;
        state[3] = BLACK;
        assert!(solver.propagate(&mut state));
        assert_eq!(state[4], WHITE);
    }
}
