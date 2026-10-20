//! One local clue enumeration with immediate consequences on the current board.
use crate::{
    clock::now_ms,
    model::{
        basic_step_deductions, feasible_masks, mask_fits_marks, Clue, Grid, BLACK, UNKNOWN, WHITE,
    },
};

pub struct ClueCaseStep {
    pub focus: Vec<usize>,
    pub marks: Vec<(usize, u8)>,
}

struct Context<'a> {
    grid: &'a Grid,
    domains: Vec<(usize, Vec<u8>)>,
}

impl Context<'_> {
    /// Check immediate rule violations, without deriving additional marks.
    fn consistent(&self, marks: &[u8]) -> bool {
        let grid = self.grid;
        for y in 0..grid.h.saturating_sub(1) {
            for x in 0..grid.w.saturating_sub(1) {
                let i = grid.idx(x, y);
                if [i, i + 1, i + grid.w, i + grid.w + 1]
                    .iter()
                    .all(|&cell| marks[cell] == BLACK)
                {
                    return false;
                }
            }
        }
        for (cell, masks) in &self.domains {
            if marks[*cell] == BLACK
                || !masks
                    .iter()
                    .any(|&m| mask_fits_marks(grid, *cell, m, marks))
            {
                return false;
            }
        }
        let Some(root) = marks.iter().position(|&v| v == BLACK) else {
            return true;
        };
        let mut seen = vec![false; grid.len()];
        let mut stack = vec![root];
        seen[root] = true;
        while let Some(cell) = stack.pop() {
            for &next in grid.ortho(cell).iter().flatten() {
                if !seen[next] && marks[next] != WHITE {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        marks
            .iter()
            .enumerate()
            .all(|(i, &v)| v != BLACK || seen[i])
    }

    fn branch(
        &self,
        snapshot: &[u8],
        clues: &[(usize, Clue)],
        cell: usize,
        mask: u8,
    ) -> Option<Vec<u8>> {
        let mut branch = snapshot.to_vec();
        for (bit, neighbor) in self.grid.neighbors(cell).iter().enumerate() {
            if let Some(neighbor) = neighbor {
                branch[*neighbor] = if mask & (1 << bit) != 0 { BLACK } else { WHITE };
            }
        }
        if !self.consistent(&branch) {
            return None;
        }
        // All consequences read the assumed distribution, never each other.
        for (cell, value) in basic_step_deductions(self.grid, &branch, clues) {
            branch[cell] = value;
        }
        self.consistent(&branch).then_some(branch)
    }
}

/// Try one clue at a time and intersect its surviving distributions after one
/// basic round. Branches never call this function or run a deduction closure.
/// A deadline can discard an unfinished clue, never certify a partial intersection.
pub fn clue_case_step(
    grid: &Grid,
    marks: &[u8],
    clues: &[(usize, Clue)],
    deadline: Option<f64>,
) -> Option<ClueCaseStep> {
    if clues.iter().any(|(cell, _)| marks[*cell] == BLACK) {
        return None;
    }
    let mut snapshot = marks.to_vec();
    for (cell, _) in clues {
        snapshot[*cell] = WHITE;
    }
    let mut domains: Vec<_> = clues
        .iter()
        .map(|(cell, clue)| {
            let masks = feasible_masks(grid, *cell, clue)
                .into_iter()
                .filter(|&m| mask_fits_marks(grid, *cell, m, &snapshot))
                .collect::<Vec<_>>();
            (*cell, masks)
        })
        .collect();
    domains.sort_by_key(|(cell, masks)| (masks.len(), *cell));
    let context = Context { grid, domains };
    if !context.consistent(&snapshot) {
        return None;
    }
    for (cell, masks) in &context.domains {
        if masks.len() < 2 {
            continue;
        }
        let mut common: Option<Vec<u8>> = None;
        for &mask in masks {
            if deadline.is_some_and(|limit| now_ms() >= limit) {
                return None;
            }
            let Some(branch) = context.branch(&snapshot, clues, *cell, mask) else {
                continue;
            };
            match &mut common {
                None => common = Some(branch),
                Some(common) => {
                    for (value, next) in common.iter_mut().zip(branch) {
                        if *value != next {
                            *value = UNKNOWN;
                        }
                    }
                }
            }
            if common.as_ref().is_some_and(|common| {
                common
                    .iter()
                    .zip(&snapshot)
                    .all(|(&v, &old)| old != UNKNOWN || v == UNKNOWN)
            }) {
                break;
            }
        }
        let common = common?;
        let marks: Vec<_> = common
            .into_iter()
            .enumerate()
            .filter(|&(i, v)| snapshot[i] == UNKNOWN && v != UNKNOWN)
            .collect();
        if !marks.is_empty() {
            let mut focus: Vec<_> = std::iter::once(*cell)
                .chain(grid.neighbors(*cell).iter().flatten().copied())
                .chain(marks.iter().map(|&(i, _)| i))
                .collect();
            focus.sort_unstable();
            focus.dedup();
            focus.retain(|i| *i != *cell);
            focus.insert(0, *cell);
            return Some(ClueCaseStep { focus, marks });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{clue_of, validate};
    use crate::rng::Rng;

    #[test]
    fn shared_effects_can_confirm_cells_outside_the_enumerated_clue() {
        let grid = Grid::new(4, 3);
        let clues = vec![(0, vec![1]), (5, vec![1])];
        let marks = vec![
            WHITE, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, WHITE, UNKNOWN, WHITE, WHITE, UNKNOWN, WHITE,
            UNKNOWN,
        ];
        assert!(basic_step_deductions(&grid, &marks, &clues).is_empty());
        let step = clue_case_step(&grid, &marks, &clues, None).unwrap();
        assert_eq!(step.focus[0], 0);
        assert_eq!(step.marks, vec![(3, WHITE), (6, WHITE), (9, WHITE), (11, WHITE)]);
        assert!(step
            .marks
            .iter()
            .all(|(i, _)| !grid.neighbors(0).contains(&Some(*i))));
        let mut reversed = clues.clone();
        reversed.reverse();
        assert_eq!(
            clue_case_step(&grid, &marks, &reversed, None)
                .unwrap()
                .marks,
            step.marks
        );
        assert_eq!(
            crate::model::one_step_deductions(&grid, &marks, &clues),
            step.marks
        );
        assert_eq!(marks[6], UNKNOWN);
    }

    #[test]
    fn distant_black_connectivity_can_eliminate_local_distributions() {
        let grid = Grid::new(4, 3);
        let clues = vec![(4, vec![1])];
        let marks = vec![
            UNKNOWN, UNKNOWN, BLACK, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN,
            WHITE, WHITE,
        ];
        assert!(basic_step_deductions(&grid, &marks, &clues).is_empty());
        let step = clue_case_step(&grid, &marks, &clues, None).unwrap();
        assert_eq!(step.marks, vec![(0, WHITE), (8, WHITE), (9, WHITE)]);
    }

    #[test]
    fn a_branch_stops_before_a_second_deduction_round() {
        let grid = Grid::new(5, 3);
        let clues = vec![(0, vec![1]), (2, vec![1]), (4, vec![1])];
        let mut marks = vec![UNKNOWN; 15];
        for (cell, _) in &clues {
            marks[*cell] = WHITE;
        }
        let context = Context {
            grid: &grid,
            domains: clues
                .iter()
                .map(|(cell, clue)| (*cell, feasible_masks(&grid, *cell, clue)))
                .collect(),
        };
        let branch = context.branch(&marks, &clues, 0, 1 << 4).unwrap();
        assert_eq!(branch[6], BLACK);
        assert_eq!(branch[3], WHITE);
        assert_eq!(branch[8], WHITE);
        assert_eq!(branch[9], UNKNOWN);
        assert_eq!(
            basic_step_deductions(&grid, &branch, &clues),
            vec![(9, BLACK)]
        );
    }

    #[test]
    fn empty_or_unfinished_case_sets_never_produce_conclusions() {
        let grid = Grid::new(4, 3);
        let clues = vec![(4, vec![1])];
        let marks = vec![
            UNKNOWN, UNKNOWN, BLACK, UNKNOWN, WHITE, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN,
            WHITE, WHITE,
        ];
        assert!(clue_case_step(&grid, &marks, &clues, Some(now_ms() - 1.0)).is_none());
        let mut impossible = marks.clone();
        impossible[0] = BLACK;
        impossible[8] = BLACK;
        assert!(clue_case_step(&grid, &impossible, &clues, None).is_none());
    }

    #[test]
    fn conclusions_agree_with_every_completion_of_small_positions() {
        let grid = Grid::new(3, 3);
        let solutions: Vec<Vec<u8>> = (0..512)
            .map(|mask| {
                (0..9)
                    .map(|i| if mask & (1 << i) != 0 { BLACK } else { WHITE })
                    .collect()
            })
            .filter(|solution: &Vec<u8>| validate(&grid, solution, &[]).is_empty())
            .collect();
        let mut rng = Rng::new(825);
        let mut deductions = 0;
        for solution in &solutions {
            for _ in 0..4 {
                let clues: Vec<_> = (0..9)
                    .filter(|&i| solution[i] == WHITE && rng.below(3) == 0)
                    .map(|i| (i, clue_of(solution, &grid, i)))
                    .collect();
                let mut marks: Vec<_> = solution
                    .iter()
                    .map(|&v| if rng.below(5) == 0 { v } else { UNKNOWN })
                    .collect();
                for (cell, _) in &clues {
                    marks[*cell] = WHITE;
                }
                let Some(step) = clue_case_step(&grid, &marks, &clues, None) else {
                    continue;
                };
                deductions += step.marks.len();
                for completion in &solutions {
                    if marks
                        .iter()
                        .zip(completion)
                        .any(|(&m, &v)| m != UNKNOWN && m != v)
                        || !validate(&grid, completion, &clues).is_empty()
                    {
                        continue;
                    }
                    for &(cell, value) in &step.marks {
                        assert_eq!(marks[cell], UNKNOWN);
                        assert_eq!(completion[cell], value, "marks {marks:?}, clues {clues:?}");
                    }
                }
            }
        }
        assert!(deductions > 0);
    }
}
