//! Plain text rendering of puzzles, shared by the CLI (`--print`) and the
//! WebAssembly build, so both show exactly the same boards.

use crate::model::{clue_text, Puzzle, BLACK, WHITE};

/// A grid where clue cells show their numbers and everything else a dot.
pub fn render_puzzle(puzzle: &Puzzle) -> String {
    let mut out = String::new();
    for y in 0..puzzle.grid.h {
        for x in 0..puzzle.grid.w {
            let i = puzzle.grid.idx(x, y);
            let cell = match puzzle.clue_at(i) {
                Some(clue) => clue_text(clue),
                None => ".".to_string(),
            };
            out.push_str(&format!("{cell:>3}"));
        }
        out.push('\n');
    }
    out
}

/// A grid of marks: `#` wall, `.` empty, `?` undecided.
pub fn render_cells(puzzle: &Puzzle, cells: &[u8]) -> String {
    let mut out = String::new();
    for y in 0..puzzle.grid.h {
        for x in 0..puzzle.grid.w {
            let i = puzzle.grid.idx(x, y);
            out.push_str(match cells[i] {
                BLACK => "  #",
                WHITE => "  .",
                _ => "  ?",
            });
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Grid;

    #[test]
    fn renders_clues_and_marks() {
        let grid = Grid::new(2, 2);
        let mut solution = vec![WHITE; 4];
        solution[0] = BLACK;
        solution[1] = BLACK;
        let puzzle = Puzzle::new(grid, vec![(3, vec![2])], solution.clone(), 1);
        let text = render_puzzle(&puzzle);
        assert_eq!(text, "  .  .\n  .  2\n");
        let marks = render_cells(&puzzle, &solution);
        assert_eq!(marks, "  #  #\n  .  .\n");
    }
}
