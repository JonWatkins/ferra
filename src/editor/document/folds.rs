//! A document's closed folds: ranges of lines shown as one.
//!
//! Folds follow edits through the buffer's edit log (so undo, redo and
//! reloads move them too), brought up to date when they're next looked at:
//! a fold after an edit moves with its lines, an edit within its first line
//! leaves it be, and any other edit touching it opens it, as Vim does when
//! you change folded text.

use std::cell::RefCell;

use super::Document;
use crate::core::Buffer;

#[derive(Debug, Clone, Default)]
pub(super) struct Folds {
    inner: RefCell<Inner>,
}

#[derive(Debug, Clone, Default)]
struct Inner {
    buffer_id: u64,
    /// The buffer's `edit_seq` the folds were last brought up to.
    seq: u64,
    /// Closed folds, `(first, last)` lines, sorted and not overlapping.
    closed: Vec<(usize, usize)>,
}

impl Inner {
    fn sync(&mut self, buffer: &Buffer) {
        if self.buffer_id != buffer.id() {
            self.buffer_id = buffer.id();
            self.seq = buffer.edit_seq();
            self.closed.clear();
            return;
        }
        let Some(edits) = buffer.edits_since(self.seq) else {
            self.closed.clear();
            self.seq = buffer.edit_seq();
            return;
        };
        for edit in edits {
            let (start, old_end, new_end) = (edit.start.0, edit.old_end.0, edit.new_end.0);
            let delta = new_end as isize - old_end as isize;
            self.closed.retain_mut(|(first, last)| {
                if start > *last {
                    true
                } else if old_end < *first {
                    *first = first.saturating_add_signed(delta);
                    *last = last.saturating_add_signed(delta);
                    true
                } else {
                    // Within the first line only: the fold stays.
                    start == *first && old_end == *first && new_end == *first
                }
            });
        }
        self.seq = buffer.edit_seq();
    }
}

impl Document {
    fn folds(&self) -> std::cell::RefMut<'_, Inner> {
        let mut inner = self.folds.inner.borrow_mut();
        inner.sync(&self.buffer);
        inner
    }

    /// The closed folds, `(first, last)` lines, in order.
    pub fn closed_folds(&self) -> Vec<(usize, usize)> {
        self.folds().closed.clone()
    }

    /// The closed fold `row` is in, if any.
    pub fn closed_fold_at(&self, row: usize) -> Option<(usize, usize)> {
        self.folds()
            .closed
            .iter()
            .copied()
            .find(|&(first, last)| first <= row && row <= last)
    }

    /// Closes lines `first..=last` into a fold (folds inside it become
    /// part of it).
    pub fn close_fold(&mut self, first: usize, last: usize) {
        if last <= first {
            return;
        }
        let mut folds = self.folds();
        folds.closed.retain(|&(a, b)| b < first || a > last);
        let at = folds.closed.partition_point(|&(a, _)| a < first);
        folds.closed.insert(at, (first, last));
    }

    /// Opens the closed fold `row` is in. Returns whether there was one.
    pub fn open_fold_at(&mut self, row: usize) -> bool {
        let mut folds = self.folds();
        let before = folds.closed.len();
        folds.closed.retain(|&(a, b)| row < a || row > b);
        folds.closed.len() != before
    }

    /// Opens every fold.
    pub fn open_all_folds(&mut self) {
        self.folds().closed.clear();
    }

    /// The line `j` goes to from `row`: the next one, or the one after the
    /// closed fold `row` is in. `None` at the end.
    pub fn next_visible_line(&self, row: usize) -> Option<usize> {
        let after = match self.closed_fold_at(row) {
            Some((_, last)) => last + 1,
            None => row + 1,
        };
        (after < self.buffer.line_count()).then_some(after)
    }

    /// The line `k` goes to from `row`: the one before, or the first line
    /// of the closed fold that's in. `None` at the start.
    pub fn previous_visible_line(&self, row: usize) -> Option<usize> {
        let start = self.closed_fold_at(row).map_or(row, |(first, _)| first);
        let above = start.checked_sub(1)?;
        Some(self.closed_fold_at(above).map_or(above, |(first, _)| first))
    }

    /// The line `count` visible lines down (or up) from `row`, stepping over
    /// closed folds, as far as the buffer goes.
    pub fn step_visible_lines(&self, row: usize, count: usize, down: bool) -> usize {
        let mut row = row;
        for _ in 0..count {
            let next = if down {
                self.next_visible_line(row)
            } else {
                self.previous_visible_line(row)
            };
            match next {
                Some(next) => row = next,
                None => break,
            }
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use crate::core::Position;
    use crate::editor::Document;

    fn doc() -> Document {
        Document::with_text("0\n1\n2\n3\n4\n5\n6")
    }

    #[test]
    fn closing_opening_and_stepping_over() {
        let mut d = doc();
        d.close_fold(2, 4);
        assert_eq!(d.closed_fold_at(3), Some((2, 4)));
        assert_eq!(d.next_visible_line(1), Some(2));
        assert_eq!(d.next_visible_line(2), Some(5), "over the fold");
        assert_eq!(d.previous_visible_line(5), Some(2), "onto its first line");
        assert_eq!(d.step_visible_lines(0, 3, true), 5);
        assert_eq!(d.step_visible_lines(6, 2, false), 2);
        assert!(d.open_fold_at(3));
        assert_eq!(d.closed_folds(), []);
    }

    #[test]
    fn folds_follow_edits_and_open_when_touched() {
        let mut d = doc();
        d.close_fold(2, 4);
        d.insert(Position::new(0, 0), "new\n");
        assert_eq!(d.closed_folds(), [(3, 5)], "moved down with its lines");
        d.insert(Position::new(3, 1), "!");
        assert_eq!(d.closed_folds(), [(3, 5)], "an edit on its first line");
        d.undo();
        d.undo();
        assert_eq!(d.closed_folds(), [(2, 4)], "undo moves it back");
        d.insert(Position::new(3, 0), "x");
        assert_eq!(d.closed_folds(), [], "an edit inside opens it");
    }

    #[test]
    fn closing_takes_in_folds_inside() {
        let mut d = doc();
        d.close_fold(2, 3);
        d.close_fold(1, 5);
        assert_eq!(d.closed_folds(), [(1, 5)]);
    }
}
