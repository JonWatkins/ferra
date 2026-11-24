//! A document's closed folds: ranges of lines shown as one.
//!
//! Folds follow edits through the buffer's edit log (so undo, redo and
//! reloads move them too), brought up to date when they're next looked at:
//! a fold after an edit moves with its lines, an edit within its first line
//! leaves it be, and any other edit touching it opens it, as Vim does when
//! you change folded text.
//!
//! Closed folds live on the `Document` (`document/folds.rs`), as sorted,
//! non-overlapping line ranges in a `RefCell`, so readers with `&Document`
//! (drawing) can bring them up to date: like the syntax trees, they replay the
//! buffer's edit log, so undo and reloads move them as well. A fold after an
//! edit shifts with its lines, an edit within its first line leaves it, and
//! anything else touching it opens it. A different buffer, or a log that's
//! moved past, opens them all.
//!
//! Everything that walks lines treats a closed fold as one: `j`/`k` turn their
//! count of visible lines into buffer lines (`over_folds`, via
//! `Document::step_visible_lines`), `gj`/`gk` step with `next_visible_line`,
//! relative line numbers count the same way (`visible_lines_between`, so the
//! number beside a line is the count `j`/`k` take to it),
//! `enforce_cursor_rules` keeps the cursor on a fold's first line,
//! `apply_operator_over` widens line ranges to whole folds,
//! `TextLayout::rows_in` gives a closed fold one screen row (for keeping the
//! cursor in sight, the view's last line and the highlight prediction), and
//! drawing shows the first line, unwrapped, with a dim `⋯ N more lines`
//! (`ui.fold`). `Editor::reveal_cursor` opens folds at the cursor after
//! searches, jump motions other than `gg`/`G`, mark jumps, undo and redo, as
//! Vim's default `foldopen` does.

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

    /// How many visible lines apart `a` and `b` are, a closed fold counting
    /// as one: the count `j`/`k` take from one to the other (relative line
    /// numbers show it). Both are lines that are shown (a fold's first line
    /// or one outside folds).
    pub fn visible_lines_between(&self, a: usize, b: usize) -> usize {
        let (lo, hi) = (a.min(b), a.max(b));
        let hidden: usize = self
            .folds()
            .closed
            .iter()
            .filter(|&&(first, last)| lo <= first && last < hi)
            .map(|&(first, last)| last - first)
            .sum();
        (hi - lo) - hidden
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
    fn visible_lines_between_counts_a_fold_as_one() {
        let mut d = doc();
        d.close_fold(2, 4);
        assert_eq!(d.visible_lines_between(0, 6), 4, "0 1 [2-4] 5 6");
        assert_eq!(d.visible_lines_between(6, 0), 4, "either way round");
        assert_eq!(d.visible_lines_between(2, 5), 1, "from the fold");
        assert_eq!(d.visible_lines_between(1, 2), 1, "onto the fold");
        assert_eq!(d.visible_lines_between(5, 6), 1);
        assert_eq!(d.visible_lines_between(3, 3), 0);
        // What `j` with that count reaches.
        assert_eq!(
            d.step_visible_lines(0, d.visible_lines_between(0, 6), true),
            6
        );
    }

    #[test]
    fn closing_takes_in_folds_inside() {
        let mut d = doc();
        d.close_fold(2, 3);
        d.close_fold(1, 5);
        assert_eq!(d.closed_folds(), [(1, 5)]);
    }
}
