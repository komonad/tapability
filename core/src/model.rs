//! Core Tapa model: grid geometry, cell states, clue encoding and validation.
//!
//! Tapa rules implemented here:
//!   * a clue cell is never black;
//!   * the black cells around a clue form runs; read clockwise starting from any
//!     position, the run lengths must match the clue numbers in order;
//!   * all black cells are orthogonally connected;
//!   * no 2x2 block of cells is completely black.

pub const UNKNOWN: u8 = 0;
pub const BLACK: u8 = 1;
pub const WHITE: u8 = 2;

/// Orthogonal neighbour deltas (up, right, down, left).
pub const ORTHO: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// The eight neighbours of a cell, in clockwise order starting from up-left.
/// Tapa clue numbers describe the black runs in exactly this cyclic order.
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

/// A clue: the lengths of the black runs around a cell, clockwise from an
/// arbitrary starting point. Only single digits occur (a run is at most 8).
pub type Clue = Vec<u8>;

/// Extract the run lengths of the black cells of an 8-bit neighbour mask,
/// read clockwise. The starting point is irrelevant: any position directly
/// after a white neighbour yields the same cyclic sequence.
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
    if clue.is_empty() {
        // A clue without numbers means "no black neighbour at all".
        return vec![0];
    }
    (0u16..256)
        .map(|m| m as u8)
        .filter(|&m| matches_rotation(&runs_clockwise(m), clue))
        .collect()
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

/// The clue produced by a fully filled grid at cell `idx`.
pub fn clue_of(cells: &[u8], grid: &Grid, idx: usize) -> Clue {
    let mut mask = 0u8;
    for (k, nb) in grid.neighbors(idx).iter().enumerate() {
        if let Some(nb) = nb {
            if cells[*nb] == BLACK {
                mask |= 1 << k;
            }
        }
    }
    runs_clockwise(mask)
}

/// A generated puzzle: clues plus the unique solution they describe.
#[derive(Clone)]
pub struct Puzzle {
    pub grid: Grid,
    /// Clues as (cell index, numbers), sorted by cell index.
    pub clues: Vec<(usize, Clue)>,
    /// cell index -> index into `clues`
    clue_index: Vec<Option<u16>>,
    pub solution: Vec<u8>,
    pub seed: u64,
}

impl Puzzle {
    pub fn new(grid: Grid, mut clues: Vec<(usize, Clue)>, solution: Vec<u8>, seed: u64) -> Puzzle {
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
    // This also catches a *single* wall group that the player has sealed off
    // with empty marks and clue cells: the solution needs walls outside that
    // pocket too, and none of them could ever reach it.
    let mut potential = vec![usize::MAX; n];
    let potential_count = components(grid, marks, |s| s != WHITE, &mut potential);
    let mut wall_id = vec![usize::MAX; n];
    let _wall_groups = components(grid, marks, |s| s == BLACK, &mut wall_id);
    if potential_count > 1 {
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
    fn live_errors_flags_a_lone_wall_that_is_sealed_in() {
        // the only wall on the board, cut off by empty marks
        let grid = Grid::new(5, 5);
        let mut marks = vec![UNKNOWN; 25];
        marks[12] = BLACK; // (2,2)
        for nb in [7usize, 8, 9, 11, 13, 17, 18, 19] {
            marks[nb] = WHITE;
        }
        let errors = live_errors(&grid, &marks, &[], Some(12));
        assert!(errors.cells[12], "a lone sealed wall must be flagged");
    }

    #[test]
    fn live_errors_flags_a_wall_sealed_in_by_clue_cells() {
        // clue cells count as empty, so a pocket of clue cells seals a wall too
        let grid = Grid::new(5, 5);
        let mut marks = vec![UNKNOWN; 25];
        marks[12] = BLACK; // (2,2)
        let clues: Vec<(usize, Clue)> = [7usize, 8, 9, 11, 13, 17, 18, 19]
            .iter()
            .map(|&i| (i, vec![1u8]))
            .collect();
        for (i, _) in clues.iter() {
            marks[*i] = WHITE; // clue cells are given as empty
        }
        let errors = live_errors(&grid, &marks, &clues, Some(12));
        assert!(errors.cells[12], "a wall inside a ring of clues is stranded");
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
    fn one_step_deductions() {
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




