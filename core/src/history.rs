//! Undoable edit groups and the deduction selection belonging to each position.
use std::collections::VecDeque;

const MAX_SAVED_MARKS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    before: Vec<(usize, u8)>,
    selection: Vec<usize>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    edits: VecDeque<Edit>,
    saved_marks: usize,
}
impl History {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
    pub fn clear(&mut self) {
        self.edits.clear();
        self.saved_marks = 0;
    }

    /// Commit only actual changes. A group is retained or evicted as a whole.
    pub fn apply(
        &mut self,
        cells: &mut [u8],
        selection: &mut Vec<usize>,
        changes: impl IntoIterator<Item = (usize, u8)>,
        select_changes: bool,
    ) -> usize {
        let mut original = vec![None; cells.len()];
        for (cell, value) in changes {
            original[cell].get_or_insert(cells[cell]);
            cells[cell] = value;
        }
        let before: Vec<_> = original
            .into_iter()
            .enumerate()
            .filter_map(|(i, v)| v.filter(|&v| v != cells[i]).map(|v| (i, v)))
            .collect();
        let count = before.len();
        if count == 0 {
            return 0;
        }
        let previous = std::mem::take(selection);
        if select_changes {
            selection.extend(before.iter().map(|&(i, _)| i));
        }
        self.saved_marks += count;
        self.edits.push_back(Edit {
            before,
            selection: previous,
        });
        while self.saved_marks > MAX_SAVED_MARKS && self.edits.len() > 1 {
            self.saved_marks -= self.edits.pop_front().unwrap().before.len();
        }
        count
    }
    /// Restore the previous board and its selection with one undo action.
    pub fn undo(&mut self, cells: &mut [u8], selection: &mut Vec<usize>) -> Option<usize> {
        let edit = self.edits.pop_back()?;
        self.saved_marks -= edit.before.len();
        let focus = edit.before[0].0;
        for (cell, value) in edit.before {
            cells[cell] = value;
        }
        *selection = edit.selection;
        Some(focus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_restores_whole_rounds_and_the_previous_selection() {
        let mut h = History::default();
        let mut cells = vec![0; 5];
        let mut selection = Vec::new();
        h.apply(&mut cells, &mut selection, [(0, 1), (1, 2)], true);
        assert_eq!(selection, vec![0, 1]);
        h.apply(&mut cells, &mut selection, [(2, 2), (3, 1)], true);
        assert_eq!(selection, vec![2, 3]);
        h.undo(&mut cells, &mut selection);
        assert_eq!(cells, vec![1, 2, 0, 0, 0]);
        assert_eq!(selection, vec![0, 1]);
        h.apply(&mut cells, &mut selection, [(4, 1)], false);
        assert!(selection.is_empty());
        h.undo(&mut cells, &mut selection);
        assert_eq!(selection, vec![0, 1]);
        h.undo(&mut cells, &mut selection);
        assert_eq!(cells, vec![0; 5]);
        assert!(selection.is_empty() && h.is_empty());
    }
    #[test]
    fn noops_preserve_selection_and_do_not_consume_history() {
        let mut h = History::default();
        let mut cells = vec![0; 3];
        let mut selection = Vec::new();
        h.apply(&mut cells, &mut selection, [(0, 1)], true);
        h.apply(&mut cells, &mut selection, [(0, 1), (1, 2), (1, 0)], false);
        assert_eq!(selection, vec![0]);
        h.undo(&mut cells, &mut selection);
        assert!(h.is_empty());
        assert_eq!(cells, vec![0; 3]);
    }
    #[test]
    fn history_limits_never_leave_a_partial_group() {
        let mut h = History::default();
        let mut cells = vec![0; 3000];
        let mut selection = Vec::new();
        h.apply(&mut cells, &mut selection, (0..3000).map(|i| (i, 1)), true);
        h.apply(&mut cells, &mut selection, (0..3000).map(|i| (i, 2)), true);
        h.undo(&mut cells, &mut selection);
        assert_eq!(cells, vec![1; 3000]);
        assert!(h.is_empty());
    }
}
