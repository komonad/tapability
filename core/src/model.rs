//! Core Tapa model: grid geometry, cell states, clue encoding and validation.
//!
//! Tapa rules implemented here:
//!   * a clue cell is never black;
//!   * the black cells around a clue form runs; the run lengths must match the
//!     clue numbers in order, and the clue may be cut open at any position - the
//!     numbers are a cycle, so `1 2` and `2 1` are the same clue;
//!   * all black cells are orthogonally connected;
//!   * no 2x2 block of cells is completely black.

use std::{collections::HashMap, sync::OnceLock};

pub const UNKNOWN: u8 = 0;
pub const BLACK: u8 = 1;
pub const WHITE: u8 = 2;

/// Orthogonal neighbour deltas (up, right, down, left).
pub const ORTHO: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// The eight neighbours of a cell, in one fixed cyclic order that starts at the
/// up-left neighbour. The deltas are `(dx, dy)` and `dy` grows downwards, so on
/// screen this cycle runs counter-clockwise; only the *cycle* matters, though.
/// Tapa clue numbers are read on it, cut open at any position, and reversing a
/// legal clue's runs always yields one of its rotations - so "clockwise from
/// anywhere" and this order describe exactly the same puzzles.
pub const NEIGHBOR_DELTAS: [(i32, i32); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
];

/// Rectangular grid with precomputed neighbour tables.
#[derive(Clone, Debug)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    neighbors: Vec<[Option<usize>; 8]>,
    ortho: Vec<[Option<usize>; 4]>,
}

impl Grid {
    pub fn new(w: usize, h: usize) -> Grid {
        let mut neighbors = Vec::with_capacity(w * h);
        let mut ortho = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut arr: [Option<usize>; 8] = [None; 8];
                for (k, &(dx, dy)) in NEIGHBOR_DELTAS.iter().enumerate() {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        arr[k] = Some((ny as usize) * w + nx as usize);
                    }
                }
                neighbors.push(arr);
                let mut four: [Option<usize>; 4] = [None; 4];
                for (k, &(dx, dy)) in ORTHO.iter().enumerate() {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        four[k] = Some((ny as usize) * w + nx as usize);
                    }
                }
                ortho.push(four);
            }
        }
        Grid {
            w,
            h,
            neighbors,
            ortho,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.w * self.h
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    #[inline]
    pub fn xy(&self, i: usize) -> (usize, usize) {
        (i % self.w, i / self.w)
    }

    #[inline]
    pub fn neighbors(&self, i: usize) -> &[Option<usize>; 8] {
        &self.neighbors[i]
    }

    /// The four orthogonal neighbours, precomputed (used by the hot loops).
    #[inline]
    pub fn ortho(&self, i: usize) -> &[Option<usize>; 4] {
        &self.ortho[i]
    }
}

/// A clue: the lengths of the black runs around a cell. The numbers have no
/// fixed starting point - they are a cycle, so `1 2` and `2 1` are the same
/// clue. Only single digits occur (a run is at most 8).
pub type Clue = Vec<u8>;

/// Extract the run lengths of the black cells of an 8-bit neighbour mask on the
/// neighbour cycle. Where the cycle is cut open is irrelevant: any position
/// directly after a white neighbour yields the same cyclic sequence.
pub fn runs_clockwise(mask: u8) -> Vec<u8> {
    if mask == 0 {
        return Vec::new();
    }
    if mask == 0xFF {
        return vec![8];
    }
    let start = (0..8).find(|&i| mask & (1 << i) == 0).unwrap();
    let mut runs = Vec::new();
    let mut count = 0u8;
    for k in 1..=8 {
        let i = (start + k) % 8;
        if mask & (1 << i) != 0 {
            count += 1;
        } else if count > 0 {
            runs.push(count);
            count = 0;
        }
    }
    runs
}

/// Does `runs` equal `clue` up to a cyclic rotation?
fn matches_rotation(runs: &[u8], clue: &[u8]) -> bool {
    if runs.len() != clue.len() {
        return false;
    }
    if clue.is_empty() {
        return true;
    }
    (0..clue.len()).any(|s| (0..clue.len()).all(|k| clue[(s + k) % clue.len()] == runs[k]))
}

/// All neighbour masks that satisfy a clue.
pub fn clue_masks(clue: &[u8]) -> Vec<u8> {
    static PATTERNS: OnceLock<HashMap<Clue, Vec<u8>>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        let mut patterns = HashMap::<Clue, Vec<u8>>::new();
        for mask in 0u16..256 {
            let runs = runs_clockwise(mask as u8);
            for offset in 0..runs.len().max(1) {
                let key: Clue = runs
                    .iter()
                    .cycle()
                    .skip(offset)
                    .take(runs.len())
                    .copied()
                    .collect();
                let entry = patterns.entry(key).or_default();
                if entry.last() != Some(&(mask as u8)) {
                    entry.push(mask as u8);
                }
            }
        }
        patterns
    });
    patterns.get(clue).cloned().unwrap_or_default()
}

/// Is `clue` a legal clue at all?
///
/// Runs are read on an 8-cell cycle, so the black runs plus one separator cell
/// each must fit: `sum + len <= 8`. This is why e.g. `1 2 3` can never occur.
pub fn clue_is_valid(clue: &[u8]) -> bool {
    if clue.is_empty() || clue.iter().any(|&n| !(1..=8).contains(&n)) {
        return false;
    }
    if clue.len() == 1 {
        return true; // a single run needs no separator
    }
    clue.iter().map(|&n| n as usize).sum::<usize>() + clue.len() <= 8
}

/// Clues the generator is willing to place: legal, but never `8` (which would
/// mean "all eight neighbours are black"). A `0` cannot occur, because a clue
/// is only placed on a cell that already touches black.
pub fn clue_is_showable(clue: &[u8]) -> bool {
    clue_is_valid(clue) && clue != [8]
}

/// Largest orthogonally connected group of cells that show nothing: not a clue
/// and not a wall. This is what a player perceives as an empty patch of board.
pub fn largest_blank_region(grid: &Grid, cells: &[u8], is_clue: &[bool]) -> usize {
    largest_region(grid, |i| cells[i] != BLACK && !is_clue[i])
}

/// Largest orthogonally connected group of white cells that have no black
/// neighbour at all. Such cells produce no clue, so big groups of them are the
/// blank patches the generator tries to avoid.
pub fn largest_clueless_region(grid: &Grid, cells: &[u8]) -> usize {
    largest_region(grid, |i| {
        cells[i] == WHITE
            && grid
                .neighbors(i)
                .iter()
                .flatten()
                .all(|&nb| cells[nb] != BLACK)
    })
}

/// Largest orthogonally connected group of cells accepted by `keep`.
fn largest_region<F: Fn(usize) -> bool>(grid: &Grid, keep: F) -> usize {
    let mut seen = vec![false; grid.len()];
    let mut largest = 0usize;
    let mut stack = Vec::new();
    for start in 0..grid.len() {
        if seen[start] || !keep(start) {
            continue;
        }
        let mut size = 0usize;
        seen[start] = true;
        stack.push(start);
        while let Some(c) = stack.pop() {
            size += 1;
            let (x, y) = grid.xy(c);
            for &(dx, dy) in ORTHO.iter() {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx as usize >= grid.w || ny as usize >= grid.h {
                    continue;
                }
                let nb = grid.idx(nx as usize, ny as usize);
                if !seen[nb] && keep(nb) {
                    seen[nb] = true;
                    stack.push(nb);
                }
            }
        }
        largest = largest.max(size);
    }
    largest
}

/// The 8-bit mask of the neighbours of `idx` that hold `BLACK` right now.
/// Cells outside the grid are never black; `UNKNOWN` counts as "not black yet".
fn black_mask(grid: &Grid, cells: &[u8], idx: usize) -> u8 {
    let mut mask = 0u8;
    for (k, nb) in grid.neighbors(idx).iter().enumerate() {
        if let Some(nb) = nb {
            if cells[*nb] == BLACK {
                mask |= 1 << k;
            }
        }
    }
    mask
}

/// The clue produced by a fully filled grid at cell `idx`.
pub fn clue_of(cells: &[u8], grid: &Grid, idx: usize) -> Clue {
    runs_clockwise(black_mask(grid, cells, idx))
}

/// A generated puzzle: clues plus the unique solution they describe.
#[derive(Clone)]
pub struct Puzzle {
    pub grid: Grid,
    /// Clues as (cell index, numbers), sorted by cell index. The numbers
    /// themselves are stored smallest first, so no view can suggest an order.
    pub clues: Vec<(usize, Clue)>,
    /// cell index -> index into `clues`
    clue_index: Vec<Option<u16>>,
    pub solution: Vec<u8>,
    pub seed: u64,
}

impl Puzzle {
    pub fn new(grid: Grid, mut clues: Vec<(usize, Clue)>, solution: Vec<u8>, seed: u64) -> Puzzle {
        // Which number of a clue is written first says nothing: the numbers are
        // a cycle, read from any starting point. They are stored sorted, so the
        // window, the web page and the printed report all phrase a clue the
        // same way and none of them hints at a starting point. Sorting a legal
        // clue loses nothing: `1 1 2` and `1 2 1` are the same clue, and no
        // legal clue can be turned into a different one by sorting it (three
        // runs always contain a repeat, because the runs need their separators
        // and so sum to at most five).
        for (_, clue) in clues.iter_mut() {
            clue.sort_unstable();
        }
        clues.sort_by_key(|(i, _)| *i);
        let mut clue_index = vec![None; grid.len()];
        for (k, (i, _)) in clues.iter().enumerate() {
            clue_index[*i] = Some(k as u16);
        }
        Puzzle {
            grid,
            clues,
            clue_index,
            solution,
            seed,
        }
    }

    #[inline]
    pub fn is_clue(&self, idx: usize) -> bool {
        self.clue_index[idx].is_some()
    }

    #[inline]
    pub fn clue_at(&self, idx: usize) -> Option<&Clue> {
        self.clue_index[idx].map(|k| &self.clues[k as usize].1)
    }

}

/// What one clue forces right now, in a single pass - no cascading.
///
/// Only the rule *"the black runs around a clue must match its numbers"* is
/// used, and only on the eight cells around that one clue. Returns `(cell,
/// value)` pairs for the neighbours that are forced and still unknown.
pub fn clue_step(grid: &Grid, marks: &[u8], idx: usize, clue: &Clue) -> Vec<(usize, u8)> {
    let alive: Vec<u8> = feasible_masks(grid, idx, clue)
        .into_iter()
        .filter(|&m| mask_fits_marks(grid, idx, m, marks))
        .collect();
    if alive.is_empty() {
        return Vec::new(); // already impossible; the red marker reports it
    }
    let mut forced = Vec::new();
    for (b, nb) in grid.neighbors(idx).iter().enumerate() {
        let Some(nb) = nb else { continue };
        if marks[*nb] != UNKNOWN {
            continue;
        }
        let bit = 1u8 << b;
        if alive.iter().all(|m| m & bit != 0) {
            forced.push((*nb, BLACK));
        } else if alive.iter().all(|m| m & bit == 0) {
            forced.push((*nb, WHITE));
        }
    }
    forced
}

/// Cells that the clues alone force, given the current marks.
///
/// Same rule as [`clue_step`], but applied to a fixpoint over every clue, so a
/// deduction can feed the next one. Returns `(cell, value)` pairs for cells
/// that were still unknown.
pub fn clue_deductions(grid: &Grid, marks: &[u8], clues: &[(usize, Clue)]) -> Vec<(usize, u8)> {
    let mut state = marks.to_vec();
    let mut deduced = Vec::new();
    loop {
        let mut changed = false;
        for (idx, clue) in clues {
            for (cell, value) in clue_step(grid, &state, *idx, clue) {
                if state[cell] == UNKNOWN {
                    state[cell] = value;
                    deduced.push((cell, value));
                    changed = true;
                }
            }
        }
        if !changed {
            return deduced;
        }
    }
}

/// Apply one basic snapshot round, or one local clue case when basic rules stall.
/// Case branches contain one basic round and never explore further assumptions.
pub fn one_step_deductions(grid: &Grid, marks: &[u8], clues: &[(usize, Clue)]) -> Vec<(usize, u8)> {
    let direct = basic_step_deductions(grid, marks, clues);
    if !direct.is_empty() {
        return direct;
    }
    crate::clue_cases::clue_case_step(grid, marks, clues, None)
        .map(|step| step.marks)
        .unwrap_or_default()
}

/// One snapshot round with no case enumeration or recursive exploration.
pub(crate) fn basic_step_deductions(
    grid: &Grid,
    marks: &[u8],
    clues: &[(usize, Clue)],
) -> Vec<(usize, u8)> {
    let mut snapshot = marks.to_vec();
    for (cell, _) in clues {
        snapshot[*cell] = WHITE;
    }
    let mut proposed = vec![UNKNOWN; grid.len()];
    let mut conflicts = vec![false; grid.len()];
    let mut propose = |cell: usize, value: u8| {
        if snapshot[cell] != UNKNOWN {
            return;
        }
        if proposed[cell] != UNKNOWN && proposed[cell] != value {
            conflicts[cell] = true;
        }
        proposed[cell] = value;
    };
    for y in 0..grid.h.saturating_sub(1) {
        for x in 0..grid.w.saturating_sub(1) {
            let quad = [
                grid.idx(x, y),
                grid.idx(x + 1, y),
                grid.idx(x, y + 1),
                grid.idx(x + 1, y + 1),
            ];
            if quad.iter().filter(|&&i| snapshot[i] == BLACK).count() == 3 {
                for i in quad {
                    if snapshot[i] == UNKNOWN {
                        propose(i, WHITE);
                    }
                }
            }
        }
    }
    for cell in connection_bridges(grid, &snapshot) {
        propose(cell, BLACK);
    }
    for (cell, clue) in clues {
        for (index, value) in clue_step(grid, &snapshot, *cell, clue) {
            propose(index, value);
        }
    }
    proposed
        .into_iter()
        .enumerate()
        .filter(|&(cell, value)| value != UNKNOWN && !conflicts[cell])
        .collect()
}

/// Apply basic structural and clue deductions until no further cells change.
/// This closure is used for logical assessment and candidate screening.
/// Only unknown cells are returned; existing marks remain unchanged.
pub fn deduction_closure(grid: &Grid, marks: &[u8], clues: &[(usize, Clue)]) -> Vec<(usize, u8)> {
    let mut state = marks.to_vec();
    for (cell, _) in clues {
        state[*cell] = WHITE;
    }
    loop {
        let before = state.clone();
        for y in 0..grid.h.saturating_sub(1) {
            for x in 0..grid.w.saturating_sub(1) {
                let quad = [
                    grid.idx(x, y),
                    grid.idx(x + 1, y),
                    grid.idx(x, y + 1),
                    grid.idx(x + 1, y + 1),
                ];
                if quad.iter().filter(|&&i| state[i] == BLACK).count() == 3 {
                    for i in quad {
                        if state[i] == UNKNOWN {
                            state[i] = WHITE;
                        }
                    }
                }
            }
        }
        for cell in connection_bridges(grid, &state) {
            state[cell] = BLACK;
        }
        for (cell, value) in clue_deductions(grid, &state, clues) {
            state[cell] = value;
        }
        if state == before {
            break;
        }
    }
    state
        .iter()
        .enumerate()
        .filter(|&(i, &v)| {
            marks[i] == UNKNOWN && v != UNKNOWN && !clues.iter().any(|(c, _)| *c == i)
        })
        .map(|(i, &v)| (i, v))
        .collect()
}

/// Unknown vertices whose removal separates existing black cells are mandatory.
pub(crate) fn connection_bridges(grid: &Grid, state: &[u8]) -> Vec<usize> {
    let total_black = state.iter().filter(|&&cell| cell == BLACK).count();
    if total_black < 2 {
        return Vec::new();
    }
    let root = state.iter().position(|&cell| cell == BLACK).unwrap();
    let n = state.len();
    let mut entered = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut parent = vec![usize::MAX; n];
    let mut next_neighbor = vec![0; n];
    let mut black_below = vec![0; n];
    let mut required = vec![false; n];
    let mut stack = Vec::with_capacity(n);
    entered[root] = 0;
    black_below[root] = 1;
    stack.push(root);
    let mut clock = 1;
    while let Some(&cell) = stack.last() {
        if next_neighbor[cell] < 4 {
            let direction = next_neighbor[cell];
            next_neighbor[cell] += 1;
            let Some(nb) = grid.ortho(cell)[direction] else {
                continue;
            };
            if state[nb] == WHITE || nb == parent[cell] {
                continue;
            }
            if entered[nb] == usize::MAX {
                parent[nb] = cell;
                entered[nb] = clock;
                low[nb] = clock;
                clock += 1;
                black_below[nb] = usize::from(state[nb] == BLACK);
                stack.push(nb);
            } else {
                low[cell] = low[cell].min(entered[nb]);
            }
        } else {
            stack.pop();
            let previous = parent[cell];
            if previous != usize::MAX {
                // The black root remains outside every separated child subtree.
                if state[previous] == UNKNOWN
                    && low[cell] >= entered[previous]
                    && black_below[cell] > 0
                {
                    required[previous] = true;
                }
                low[previous] = low[previous].min(low[cell]);
                black_below[previous] += black_below[cell];
            }
        }
    }
    if black_below[root] != total_black {
        return Vec::new();
    }
    required
        .into_iter()
        .enumerate()
        .filter_map(|(cell, required)| required.then_some(cell))
        .collect()
}

/// Human readable clue, e.g. `1` or `12` for the clue `[1, 2]`.
pub fn clue_text(clue: &[u8]) -> String {
    if clue.is_empty() {
        "-".to_string()
    } else {
        clue.iter().map(|n| (b'0' + n) as char).collect()
    }
}

/// The masks of `clue` that are actually possible for cell `idx`: a cell outside
/// the grid is never black, so masks requiring one are dropped.
pub fn feasible_masks(grid: &Grid, idx: usize, clue: &Clue) -> Vec<u8> {
    let neighbors = grid.neighbors(idx);
    clue_masks(clue)
        .into_iter()
        .filter(|m| (0..8).all(|k| neighbors[k].is_some() || m & (1 << k) == 0))
        .collect()
}

/// Does neighbour mask `m` agree with the marks around cell `idx`?
pub fn mask_fits_marks(grid: &Grid, idx: usize, m: u8, marks: &[u8]) -> bool {
    grid.neighbors(idx)
        .iter()
        .enumerate()
        .all(|(k, nb)| match nb {
            None => true,
            Some(nb) => {
                let want_black = m & (1 << k) != 0;
                match marks[*nb] {
                    BLACK => want_black,
                    WHITE => !want_black,
                    _ => true,
                }
            }
        })
}

/// Can the clue at `idx` still be satisfied by the current marks?
pub fn clue_satisfiable(grid: &Grid, marks: &[u8], idx: usize, clue: &Clue) -> bool {
    feasible_masks(grid, idx, clue)
        .iter()
        .any(|&m| mask_fits_marks(grid, idx, m, marks))
}

/// Are the numbers of the clue at `idx` already accounted for?
///
/// True when the walls that are on the board already form the clue's runs, so no
/// further wall may be added around that cell and every neighbour that is still
/// undecided has to become empty. Walls that are still `UNKNOWN` do not spoil it
/// - the numbers are met, they simply still have to be marked white.
///
/// The numbers are compared the same way they are read: clockwise from *any*
/// starting point, so a rotated wall pattern counts just as much as an exact
/// match.
pub fn clue_satisfied(grid: &Grid, marks: &[u8], idx: usize, clue: &Clue) -> bool {
    matches_rotation(&runs_clockwise(black_mask(grid, marks, idx)), clue)
}

/// Flood fill orthogonal components over the cells where `keep` holds.
/// `id[i]` is set to the component number, or `usize::MAX` for skipped cells.
fn components<F: Fn(u8) -> bool>(grid: &Grid, marks: &[u8], keep: F, id: &mut [usize]) -> usize {
    for v in id.iter_mut() {
        *v = usize::MAX;
    }
    let mut count = 0usize;
    let mut stack = Vec::new();
    for start in 0..grid.len() {
        if id[start] != usize::MAX || !keep(marks[start]) {
            continue;
        }
        id[start] = count;
        stack.push(start);
        while let Some(c) = stack.pop() {
            let (x, y) = grid.xy(c);
            for &(dx, dy) in ORTHO.iter() {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx as usize >= grid.w || ny as usize >= grid.h {
                    continue;
                }
                let nb = grid.idx(nx as usize, ny as usize);
                if id[nb] == usize::MAX && keep(marks[nb]) {
                    id[nb] = count;
                    stack.push(nb);
                }
            }
        }
        count += 1;
    }
    count
}

/// Cells on the line between two cells, inclusive of both ends. Used so a
/// quick drag paints every cell the cursor crossed instead of leaving gaps.
pub fn line_cells(grid: &Grid, from: usize, to: usize) -> Vec<usize> {
    let (x0, y0) = grid.xy(from);
    let (x1, y1) = grid.xy(to);
    let (mut x, mut y) = (x0 as i32, y0 as i32);
    let (tx, ty) = (x1 as i32, y1 as i32);
    let dx = (tx - x).abs();
    let dy = (ty - y).abs();
    let sx = if x < tx { 1 } else { -1 };
    let sy = if y < ty { 1 } else { -1 };
    let mut err = dx - dy;
    let mut out = Vec::with_capacity((dx + dy) as usize + 1);
    loop {
        out.push(grid.idx(x as usize, y as usize));
        if x == tx && y == ty {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
    out
}

/// The orthogonally connected wall group containing `start`.
pub fn wall_component(grid: &Grid, marks: &[u8], start: usize) -> Vec<bool> {
    let mut out = vec![false; grid.len()];
    if marks[start] != BLACK {
        return out;
    }
    let mut stack = vec![start];
    out[start] = true;
    while let Some(c) = stack.pop() {
        let (x, y) = grid.xy(c);
        for &(dx, dy) in ORTHO.iter() {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx as usize >= grid.w || ny as usize >= grid.h {
                continue;
            }
            let nb = grid.idx(nx as usize, ny as usize);
            if !out[nb] && marks[nb] == BLACK {
                out[nb] = true;
                stack.push(nb);
            }
        }
    }
    out
}

/// Live feedback while the player is still filling the board.
pub struct LiveErrors {
    /// Cells that break a rule right now (drawn with a red outline).
    pub cells: Vec<bool>,
    /// Clue cells whose numbers can no longer be met (drawn in red).
    pub clues: Vec<bool>,
}

impl LiveErrors {
    pub fn any(&self) -> bool {
        self.cells.iter().any(|&b| b)
    }
}

/// Rules the player has already broken beyond repair.
///
/// Three things are reported:
///   * a 2x2 block of walls,
///   * a clue whose surrounding walls can no longer match its numbers,
///   * a wall group that can never join the main wall group any more, because
///     the cells between them are all marked empty. Groups that are merely
///     separate *for now* (undecided cells still connect them) are not flagged.
pub fn live_errors(
    grid: &Grid,
    marks: &[u8],
    clues: &[(usize, Clue)],
    focus: Option<usize>,
) -> LiveErrors {
    let n = grid.len();
    let mut errors = LiveErrors {
        cells: vec![false; n],
        clues: vec![false; n],
    };

    // 2x2 walls
    if grid.w >= 2 && grid.h >= 2 {
        for y in 0..grid.h - 1 {
            for x in 0..grid.w - 1 {
                let quad = [
                    grid.idx(x, y),
                    grid.idx(x + 1, y),
                    grid.idx(x, y + 1),
                    grid.idx(x + 1, y + 1),
                ];
                if quad.iter().all(|&i| marks[i] == BLACK) {
                    for &i in quad.iter() {
                        errors.cells[i] = true;
                    }
                }
            }
        }
    }

    // clues that can no longer be satisfied
    for (idx, clue) in clues {
        if !clue_satisfiable(grid, marks, *idx, clue) {
            errors.clues[*idx] = true;
            errors.cells[*idx] = true;
        }
    }

    // wall groups that are sealed off from the main play area
    //
    // `potential` groups the cells that could still become black (walls plus
    // undecided cells). Two wall groups in the same potential group can still
    // be joined later; groups in different potential groups never can.
    //
    // A lone wall group is deliberately *not* flagged, even when the player has
    // sealed it in: the rule is about two groups that can never meet again, not
    // about predicting that the solution must have walls elsewhere.
    let mut potential = vec![usize::MAX; n];
    let potential_count = components(grid, marks, |s| s != WHITE, &mut potential);
    let mut wall_id = vec![usize::MAX; n];
    let wall_groups = components(grid, marks, |s| s == BLACK, &mut wall_id);
    if wall_groups > 1 && potential_count > 1 {
        let mut potential_size = vec![0usize; potential_count];
        for i in 0..n {
            if potential[i] != usize::MAX {
                potential_size[potential[i]] += 1;
            }
        }
        // the main play area is the biggest potential group (ties prefer the
        // one the player just touched)
        let mut main = 0usize;
        for p in 1..potential_count {
            let focused = focus.map_or(false, |f| potential.get(f).copied() == Some(p));
            if potential_size[p] > potential_size[main]
                || (potential_size[p] == potential_size[main] && focused)
            {
                main = p;
            }
        }
        for i in 0..n {
            if wall_id[i] != usize::MAX && potential[i] != main {
                errors.cells[i] = true;
            }
        }
    }

    errors
}

/// Check a fully or partially filled grid against the rules.
/// Returns a list of human readable rule violations (empty = everything fine).
pub fn validate(grid: &Grid, cells: &[u8], clues: &[(usize, Clue)]) -> Vec<String> {
    let mut errors = Vec::new();

    for (i, c) in cells.iter().enumerate() {
        if *c == UNKNOWN {
            let (x, y) = grid.xy(i);
            errors.push(format!("cell ({x},{y}) is empty"));
        }
    }
    if !errors.is_empty() {
        return errors;
    }

    for (idx, clue) in clues {
        if cells[*idx] != WHITE {
            let (x, y) = grid.xy(*idx);
            errors.push(format!("clue cell ({x},{y}) must be empty"));
        }
        let actual = clue_of(cells, grid, *idx);
        if !matches_rotation(&actual, clue) {
            let (x, y) = grid.xy(*idx);
            errors.push(format!(
                "clue at ({x},{y}) is '{}' but the grid gives '{}'",
                clue_text(clue),
                clue_text(&actual)
            ));
        }
    }

    if grid.w >= 2 && grid.h >= 2 {
        for y in 0..grid.h - 1 {
            for x in 0..grid.w - 1 {
                let a = grid.idx(x, y);
                let b = grid.idx(x + 1, y);
                let c = grid.idx(x, y + 1);
                let d = grid.idx(x + 1, y + 1);
                if [a, b, c, d].iter().all(|&i| cells[i] == BLACK) {
                    errors.push(format!("2x2 black block at ({x},{y})"));
                }
            }
        }
    }

    // Connectivity of the black cells.
    let blacks: Vec<usize> = (0..cells.len()).filter(|&i| cells[i] == BLACK).collect();
    if let Some(&first) = blacks.first() {
        let mut seen = vec![false; cells.len()];
        let mut stack = vec![first];
        seen[first] = true;
        let mut reached = 0usize;
        while let Some(i) = stack.pop() {
            reached += 1;
            let (x, y) = grid.xy(i);
            for &(dx, dy) in ORTHO.iter() {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx as usize >= grid.w || ny as usize >= grid.h {
                    continue;
                }
                let n = grid.idx(nx as usize, ny as usize);
                if !seen[n] && cells[n] == BLACK {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        if reached != blacks.len() {
            errors.push(format!(
                "black cells are not connected ({} of {} reachable)",
                reached,
                blacks.len()
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_of_known_masks() {
        assert_eq!(runs_clockwise(0), Vec::<u8>::new());
        assert_eq!(runs_clockwise(0xFF), vec![8]);
        // single black at up-left
        assert_eq!(runs_clockwise(1 << 0), vec![1]);
        // up + up-right + right
        assert_eq!(runs_clockwise(0b0000_1110), vec![3]);
        // up-left, then up-right..right
        assert_eq!(runs_clockwise(0b0000_1101), vec![2, 1]);
        // alternating
        assert_eq!(runs_clockwise(0b1010_1010), vec![1, 1, 1, 1]);
    }

    #[test]
    fn clue_mask_counts() {
        assert_eq!(clue_masks(&[8]), vec![0xFF]);
        assert_eq!(clue_masks(&[1]).len(), 8);
        assert_eq!(clue_masks(&[2]).len(), 8);
        // two non adjacent cells on an 8-cycle: 8 * 5 / 2
        assert_eq!(clue_masks(&[1, 1]).len(), 20);
        // three non adjacent cells on an 8-cycle
        assert_eq!(clue_masks(&[1, 1, 1]).len(), 16);
        // four runs of one = alternating
        assert_eq!(clue_masks(&[1, 1, 1, 1]).len(), 2);
        // a 2-run (8 places) plus an isolated black (4 legal places)
        assert_eq!(clue_masks(&[2, 1]).len(), 32);
        // a 3-run (8 places) plus an isolated black (3 legal places)
        assert_eq!(clue_masks(&[3, 1]).len(), 24);
        assert!(clue_masks(&[1, 2]).contains(&0b0000_1101));
    }

    #[test]
    fn cached_patterns_match_cyclic_runs() {
        for mask in 0u16..256 {
            let runs = runs_clockwise(mask as u8);
            for offset in 0..runs.len().max(1) {
                let clue: Clue = runs
                    .iter()
                    .cycle()
                    .skip(offset)
                    .take(runs.len())
                    .copied()
                    .collect();
                let expected: Vec<u8> = (0u16..256)
                    .map(|m| m as u8)
                    .filter(|&m| matches_rotation(&runs_clockwise(m), &clue))
                    .collect();
                assert_eq!(clue_masks(&clue), expected);
            }
        }
    }

    #[test]
    fn bridge_analysis_matches_vertex_removal_on_every_three_by_three_state() {
        let grid = Grid::new(3, 3);
        for encoded in 0..3usize.pow(9) {
            let mut rest = encoded;
            let mut marks = vec![UNKNOWN; 9];
            for mark in &mut marks {
                *mark = (rest % 3) as u8;
                rest /= 3;
            }
            let blacks: Vec<_> = (0..9).filter(|&i| marks[i] == BLACK).collect();
            let connected = |blocked: usize| {
                let Some(&root) = blacks.first() else {
                    return true;
                };
                let mut seen = [false; 9];
                let mut stack = vec![root];
                seen[root] = true;
                while let Some(cell) = stack.pop() {
                    for &nb in grid.ortho(cell).iter().flatten() {
                        if nb != blocked && marks[nb] != WHITE && !seen[nb] {
                            seen[nb] = true;
                            stack.push(nb);
                        }
                    }
                }
                blacks.iter().all(|&i| seen[i])
            };
            let expected: Vec<_> = if blacks.len() >= 2 && connected(usize::MAX) {
                (0..9)
                    .filter(|&i| marks[i] == UNKNOWN && !connected(i))
                    .collect()
            } else {
                Vec::new()
            };
            assert_eq!(
                connection_bridges(&grid, &marks),
                expected,
                "state {encoded}"
            );
        }
    }

    #[test]
    fn bridge_analysis_handles_long_corridors_without_recursion() {
        let grid = Grid::new(3600, 1);
        let mut marks = vec![UNKNOWN; grid.len()];
        marks[0] = BLACK;
        marks[3599] = BLACK;
        assert_eq!(
            connection_bridges(&grid, &marks),
            (1..3599).collect::<Vec<_>>()
        );
    }

    #[test]
    fn impossible_clues() {
        // 1 + 2 + 3 = 6 black cells plus 3 separators needs 9 cells
        assert!(clue_masks(&[1, 2, 3]).is_empty());
        assert!(!clue_is_valid(&[1, 2, 3]));
        assert!(clue_is_valid(&[1, 2, 2]));
        assert!(clue_is_valid(&[8]));
        assert!(!clue_is_valid(&[9]));
        assert!(!clue_is_valid(&[]));
    }

    #[test]
    fn runs_are_cyclic() {
        // bits 0, 2, 3, 5, 6, 7: the run at 7..0 wraps around the corner
        let m = 0b1110_1101u8;
        assert_eq!(runs_clockwise(m), vec![2, 4]);
        // 0 1 1 0 1 1 0 1 -> runs [2,2,1]
        let m2 = 0b1011_0110u8;
        assert_eq!(runs_clockwise(m2), vec![2, 2, 1]);
        assert!(clue_masks(&[1, 2, 2]).contains(&m2));
    }

    #[test]
    fn clue_of_round_trip() {
        let grid = Grid::new(3, 3);
        let mut cells = vec![WHITE; 9];
        // black ring around the centre
        for i in 0..9 {
            if i != 4 {
                cells[i] = BLACK;
            }
        }
        assert_eq!(clue_of(&cells, &grid, 4), vec![8]);
        // only the corner is black
        let mut cells2 = vec![WHITE; 9];
        cells2[0] = BLACK;
        assert_eq!(clue_of(&cells2, &grid, 4), vec![1]);
    }

    #[test]
    fn clues_are_stored_smallest_first() {
        let grid = Grid::new(3, 3);
        let solution = vec![WHITE; 9];
        let puzzle = Puzzle::new(grid, vec![(4, vec![2u8, 1, 1])], solution, 7);
        assert_eq!(puzzle.clue_at(4).cloned(), Some(vec![1u8, 1, 2]));
        assert_eq!(clue_text(puzzle.clue_at(4).unwrap()), "112");
    }

    #[test]
    fn validate_detects_problems() {
        let grid = Grid::new(2, 2);
        let cells = vec![BLACK; 4];
        let errors = validate(&grid, &cells, &[]);
        assert!(errors.iter().any(|e| e.contains("2x2")));
    }

    #[test]
    fn wall_component_finds_the_group() {
        let grid = Grid::new(4, 4);
        let mut marks = vec![WHITE; 16];
        marks[0] = BLACK;
        marks[1] = BLACK;
        marks[5] = BLACK; // (1,1)
        marks[10] = BLACK; // separate
        let group = wall_component(&grid, &marks, 0);
        assert!(group[0] && group[1] && group[5]);
        assert!(!group[10]);
        assert_eq!(group.iter().filter(|&&b| b).count(), 3);
    }

    #[test]
    fn clue_satisfiability() {
        let grid = Grid::new(3, 3);
        let clue = vec![1u8];
        let mut marks = vec![UNKNOWN; 9];
        assert!(clue_satisfiable(&grid, &marks, 4, &clue));
        marks[0] = BLACK; // exactly one wall around the centre
        assert!(clue_satisfiable(&grid, &marks, 4, &clue));
        marks[2] = BLACK; // now two, a clue of 1 can never be met
        assert!(!clue_satisfiable(&grid, &marks, 4, &clue));
        marks[2] = UNKNOWN;
        assert!(clue_satisfiable(&grid, &marks, 4, &clue));
    }

    #[test]
    fn clue_satisfied_needs_every_number_accounted_for() {
        let grid = Grid::new(3, 3);
        let clue = vec![2u8];
        let mut marks = vec![UNKNOWN; 9];
        assert!(!clue_satisfied(&grid, &marks, 4, &clue), "no wall yet");
        marks[0] = BLACK; // a run of 1 of 2, still short
        assert!(!clue_satisfied(&grid, &marks, 4, &clue));
        marks[1] = BLACK; // a run of 2 - the numbers are met
        assert!(clue_satisfied(&grid, &marks, 4, &clue));
        // the neighbours that are still undecided simply have to become white
        marks[5] = WHITE;
        assert!(clue_satisfied(&grid, &marks, 4, &clue));
        marks[2] = BLACK; // a third wall would break the clue
        assert!(!clue_satisfied(&grid, &marks, 4, &clue));
        marks[2] = WHITE;
        assert!(clue_satisfied(&grid, &marks, 4, &clue));
    }

    #[test]
    fn a_clue_is_read_from_any_starting_point() {
        let grid = Grid::new(3, 3);
        let mut cells = vec![WHITE; 9];
        // around the centre: walls at the up-left (idx 0), up-right (idx 2), down
        // (idx 7) and down-right (idx 8) neighbours - a 2-run and two singles
        for i in [0usize, 2, 7, 8] {
            cells[i] = BLACK;
        }
        assert_eq!(clue_of(&cells, &grid, 4), vec![2, 1, 1]);
        // the player's walls may match any rotation of the numbers
        for written in [vec![2u8, 1, 1], vec![1, 1, 2], vec![1, 2, 1]] {
            assert!(
                clue_satisfied(&grid, &cells, 4, &written),
                "{written:?} describes the same clue"
            );
            let errors = validate(&grid, &cells, &[(4, written.clone())]);
            assert!(
                !errors.iter().any(|e| e.contains("clue")),
                "{written:?} was rejected: {errors:?}"
            );
        }
        // rotations pick out exactly the same neighbour masks
        assert_eq!(clue_masks(&[1, 1, 2]), clue_masks(&[2, 1, 1]));
        // a clue that is not a rotation is still a different clue
        assert!(!clue_satisfied(&grid, &cells, 4, &vec![2u8, 2]));
        assert!(!clue_satisfied(&grid, &cells, 4, &vec![1u8, 1, 1]));
        assert!(!clue_satisfied(&grid, &cells, 4, &vec![4u8]));
    }

    #[test]
    fn live_errors_flags_2x2() {
        let grid = Grid::new(4, 4);
        let mut marks = vec![UNKNOWN; 16];
        for i in [0usize, 1, 4, 5] {
            marks[i] = BLACK;
        }
        let errors = live_errors(&grid, &marks, &[], None);
        for i in [0usize, 1, 4, 5] {
            assert!(errors.cells[i], "cell {i} should be flagged");
        }
    }

    #[test]
    fn live_errors_flags_sealed_wall_group() {
        let grid = Grid::new(5, 5);
        let mut marks = vec![UNKNOWN; 25];
        marks[0] = BLACK; // (0,0)
        marks[24] = BLACK; // (4,4), in the open area
        // seal (0,0) off: all its orthogonal neighbours become empty
        marks[1] = WHITE;
        marks[5] = WHITE;
        let errors = live_errors(&grid, &marks, &[], Some(0));
        assert!(errors.cells[0], "the sealed group must be flagged");
        assert!(!errors.cells[24], "the open group must not be flagged");
    }

    #[test]
    fn live_errors_leaves_a_lone_wall_alone_even_when_sealed_in() {
        // A single wall group is never flagged: the isolation rule is about two
        // groups that can never meet again, not about guessing where the rest
        // of the solution's walls must be.
        let grid = Grid::new(5, 5);
        let mut marks = vec![UNKNOWN; 25];
        marks[12] = BLACK; // (2,2)
        for nb in [7usize, 8, 9, 11, 13, 17, 18, 19] {
            marks[nb] = WHITE;
        }
        let errors = live_errors(&grid, &marks, &[], Some(12));
        assert!(!errors.cells.iter().any(|&b| b), "a lone wall is not stranded");
    }

    #[test]
    fn live_errors_leaves_a_lone_open_wall_alone() {
        let grid = Grid::new(5, 5);
        let mut marks = vec![UNKNOWN; 25];
        marks[12] = BLACK;
        let errors = live_errors(&grid, &marks, &[], Some(12));
        assert!(!errors.cells.iter().any(|&b| b), "nothing is sealed yet");
    }

    #[test]
    fn live_errors_ignores_groups_that_can_still_join() {
        let grid = Grid::new(5, 5);
        let mut marks = vec![UNKNOWN; 25];
        marks[0] = BLACK;
        marks[24] = BLACK;
        // nothing sealed: both groups sit in the same undecided area
        let errors = live_errors(&grid, &marks, &[], None);
        assert!(!errors.cells.iter().any(|&b| b));
    }

    #[test]
    fn clue_only_deductions() {
        // 3x3, clue "1" at the centre. Marking one neighbour empty leaves
        // exactly one place for the single wall: the opposite one.
        let grid = Grid::new(3, 3);
        let clues = vec![(4usize, vec![1u8])];
        let mut marks = vec![UNKNOWN; 9];
        marks[4] = WHITE; // clue cells are given as empty
        let found = clue_deductions(&grid, &marks, &clues);
        assert!(found.is_empty(), "nothing is forced yet");

        // every neighbour empty except one -> that one must be the wall
        for i in [0usize, 1, 2, 3, 5, 6, 7] {
            marks[i] = WHITE;
        }
        let found = clue_deductions(&grid, &marks, &clues);
        assert_eq!(found, vec![(8, BLACK)]);

        // clue "2" with one neighbour already black forces the other wall
        let clues = vec![(4usize, vec![2u8])];
        let mut marks = vec![UNKNOWN; 9];
        marks[4] = WHITE;
        marks[0] = BLACK;
        for i in [2usize, 3, 5, 6, 7] {
            marks[i] = WHITE;
        }
        let found = clue_deductions(&grid, &marks, &clues);
        assert!(found.contains(&(1, BLACK)) || found.contains(&(3, BLACK)));
    }

    #[test]
    fn structural_steps_mark_every_three_black_square() {
        let grid = Grid::new(3, 2);
        let marks = vec![BLACK, BLACK, BLACK, UNKNOWN, BLACK, UNKNOWN];
        let found = one_step_deductions(&grid, &marks, &[]);
        assert_eq!(found, vec![(3, WHITE), (5, WHITE)]);
        assert!(one_step_deductions(&grid, &[BLACK; 6], &[]).is_empty());
    }

    #[test]
    fn bridges_require_black_cells_on_both_sides() {
        let grid = Grid::new(5, 1);
        assert_eq!(
            one_step_deductions(&grid, &[BLACK, UNKNOWN, UNKNOWN, UNKNOWN, BLACK], &[]),
            vec![(1, BLACK), (2, BLACK), (3, BLACK)]
        );
        assert!(
            one_step_deductions(&grid, &[BLACK, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN], &[])
                .is_empty()
        );
        assert!(
            one_step_deductions(&grid, &[BLACK, UNKNOWN, WHITE, UNKNOWN, BLACK], &[]).is_empty()
        );
        let grid = Grid::new(3, 3);
        let mut marks = vec![UNKNOWN; 9];
        marks[0] = BLACK;
        marks[8] = BLACK;
        assert!(one_step_deductions(&grid, &marks, &[]).is_empty());
    }

    #[test]
    fn square_and_bridge_consequences_use_separate_snapshots() {
        let grid = Grid::new(3, 2);
        let marks = vec![BLACK, BLACK, UNKNOWN, BLACK, UNKNOWN, BLACK];
        let found = one_step_deductions(&grid, &marks, &[]);
        assert_eq!(found, vec![(4, WHITE)]);
        let mut next = marks.clone();
        next[4] = WHITE;
        assert_eq!(one_step_deductions(&grid, &next, &[]), vec![(2, BLACK)]);
        assert_eq!(
            deduction_closure(&grid, &marks, &[]),
            vec![(2, BLACK), (4, WHITE)]
        );
    }

    #[test]
    fn structural_deductions_agree_with_small_valid_boards() {
        let grid = Grid::new(3, 3);
        for mask in 1..512 {
            let solution: Vec<u8> = (0..9)
                .map(|i| if mask & (1 << i) != 0 { BLACK } else { WHITE })
                .collect();
            if !validate(&grid, &solution, &[]).is_empty() {
                continue;
            }
            for visible in 0..512 {
                let marks: Vec<u8> = (0..9)
                    .map(|i| {
                        if visible & (1 << i) != 0 {
                            solution[i]
                        } else {
                            UNKNOWN
                        }
                    })
                    .collect();
                for (cell, value) in one_step_deductions(&grid, &marks, &[]) {
                    assert_eq!(
                        value, solution[cell],
                        "solution {mask}, visible {visible}, cell {cell}"
                    );
                    assert_eq!(marks[cell], UNKNOWN);
                }
            }
        }
    }

    #[test]
    fn one_round_does_not_cascade_between_clues() {
        let grid = Grid::new(4, 2);
        let clues = vec![(0, vec![3]), (2, vec![2])];
        let marks = vec![
            WHITE, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN,
        ];
        let first = one_step_deductions(&grid, &marks, &clues);
        assert_eq!(first, vec![(1, BLACK), (4, BLACK), (5, BLACK)]);
        let mut reversed = clues.clone();
        reversed.reverse();
        assert_eq!(one_step_deductions(&grid, &marks, &reversed), first);
        let mut next = marks.clone();
        for (cell, value) in first {
            next[cell] = value;
        }
        assert_eq!(
            one_step_deductions(&grid, &next, &clues),
            vec![(3, WHITE), (6, WHITE), (7, WHITE)]
        );
        assert_eq!(marks[1], UNKNOWN);
    }

    #[test]
    fn contradictory_clue_consequences_do_not_assign_an_arbitrary_value() {
        let grid = Grid::new(3, 1);
        let marks = vec![WHITE, UNKNOWN, WHITE];
        let clues = vec![(0, vec![1]), (2, vec![])];
        assert!(one_step_deductions(&grid, &marks, &clues).is_empty());
    }

    #[test]
    fn clue_patterns_use_existing_walls_to_confirm_multiple_cells() {
        let grid = Grid::new(4, 2);
        let marks = vec![
            UNKNOWN, BLACK, WHITE, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN,
        ];
        let mut step = clue_step(&grid, &marks, 2, &vec![2]);
        step.sort();
        assert_eq!(step, vec![(3, WHITE), (5, BLACK), (6, WHITE), (7, WHITE)]);
        assert_eq!(one_step_deductions(&grid, &marks, &[(2, vec![2])]), step);
    }

    #[test]
    fn single_clue_step_is_local() {
        // 4x3 grid, one clue at (1,1). Marking seven of its neighbours empty
        // leaves exactly one place for the wall the clue asks for.
        let grid = Grid::new(4, 3);
        let clue = vec![1u8];
        let mut marks = vec![UNKNOWN; 12];
        marks[5] = WHITE; // the clue cell itself, at (1,1)
        for i in [0usize, 1, 2, 4, 6, 8, 9] {
            marks[i] = WHITE;
        }
        let step = clue_step(&grid, &marks, 5, &clue);
        assert_eq!(step, vec![(10, BLACK)]);

        // the step only ever touches the eight cells of its own clue
        for (cell, _) in &step {
            let (x, y) = grid.xy(*cell);
            let (cx, cy) = grid.xy(5);
            assert!((x as i32 - cx as i32).abs() <= 1 && (y as i32 - cy as i32).abs() <= 1);
        }

        // one unknown too many: nothing is forced
        marks[9] = UNKNOWN;
        assert!(clue_step(&grid, &marks, 5, &clue).is_empty());
    }
    #[test]
    fn live_errors_flags_impossible_clue() {        let grid = Grid::new(3, 3);
        let mut marks = vec![UNKNOWN; 9];
        marks[0] = BLACK;
        marks[2] = BLACK;
        let clues = vec![(4usize, vec![1u8])];
        let errors = live_errors(&grid, &marks, &clues, None);
        assert!(errors.clues[4]);
        assert!(errors.cells[4]);
    }
}




