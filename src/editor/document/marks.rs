//! A document's marks (`ma` … `'a`), kept in place as the text changes.
//!
//! Marks are kept by each `Document` (`document/marks.rs`) and adjusted by its
//! `insert` and `delete`, so they move with text inserted or deleted before
//! them. Global marks (`A`-`Z`) record which buffer they're in and reuse that
//! buffer's mark under the same letter.
//!
//! Undo and redo move marks too, as Vim does: each undo step keeps where the
//! marks were before it (`begin_step`), and undoing it moves the marks along
//! with the changes undo makes, then puts the ones it kept back
//! (`undone_or_redone`). Following the changes alone would leave a mark on a
//! deleted line one line down once the line came back, since the restored
//! text goes in front of it. Redo swaps the positions back the same way. Marks
//! set after the step only follow the changes. `'` and `` ` ``
//! are motions whose target comes from the mark (`MotionArgs::target`), so they
//! work after operators and in visual mode.

use std::collections::HashMap;

use crate::core::{Change, Position, Revision, end_of};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Marks {
    marks: HashMap<char, Position>,
    /// For each undo step, where the marks were on the other side of it:
    /// before it while it's done, after it while it's undone.
    steps: HashMap<Revision, HashMap<char, Position>>,
}

impl Marks {
    pub fn set(&mut self, name: char, position: Position) {
        self.marks.insert(name, position);
    }

    pub fn get(&self, name: char) -> Option<Position> {
        self.marks.get(&name).copied()
    }

    /// An undo step that will be `step` is starting, the latest being
    /// `latest`: keeps where the marks are now, for undoing it. Steps after
    /// `latest` were undone and can't be redone any more, so are forgotten.
    pub fn begin_step(&mut self, step: Revision, latest: Revision) {
        self.steps.retain(|&id, _| id <= latest);
        if !self.marks.is_empty() {
            self.steps.insert(step, self.marks.clone());
        }
    }

    /// Undo step `step` was undone or redone, making `changes`: marks move
    /// with them, then those kept on the other side of it go back there,
    /// and where they were is kept for going back again.
    pub fn undone_or_redone(&mut self, step: Revision, changes: &[Change]) {
        let before = self.marks.clone();
        for change in changes {
            match change {
                Change::Insert { at, text } => self.after_insert(*at, end_of(*at, text)),
                Change::Delete { at, text } => self.after_delete(*at, end_of(*at, text)),
            }
        }
        if let Some(kept) = self.steps.remove(&step) {
            self.marks.extend(kept);
        }
        if !before.is_empty() {
            self.steps.insert(step, before);
        }
    }

    /// Text was inserted at `at`, ending at `end`: marks at or after `at`
    /// move with the text that followed.
    pub fn after_insert(&mut self, at: Position, end: Position) {
        for mark in self.marks.values_mut() {
            if *mark < at {
                continue;
            }
            *mark = if mark.row == at.row {
                Position::new(end.row, end.col + (mark.col - at.col))
            } else {
                Position::new(mark.row + (end.row - at.row), mark.col)
            };
        }
    }

    /// Text from `start` to `end` was deleted: marks inside it move to
    /// `start`, and marks after it move back with the text that followed.
    pub fn after_delete(&mut self, start: Position, end: Position) {
        for mark in self.marks.values_mut() {
            if *mark < start {
                continue;
            }
            *mark = if *mark < end {
                start
            } else if mark.row == end.row {
                Position::new(start.row, start.col + (mark.col - end.col))
            } else {
                Position::new(mark.row - (end.row - start.row), mark.col)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn marks(list: &[(char, Position)]) -> Marks {
        let mut m = Marks::default();
        list.iter().for_each(|&(c, p)| m.set(c, p));
        m
    }

    #[test]
    fn inserts_before_move_marks_and_after_dont() {
        let mut m = marks(&[('a', pos(0, 5)), ('b', pos(2, 1)), ('c', pos(0, 1))]);
        // Insert "x\ny" at (0, 3): ends at (1, 1).
        m.after_insert(pos(0, 3), pos(1, 1));
        assert_eq!(
            m.get('a'),
            Some(pos(1, 3)),
            "same line, after: carried along"
        );
        assert_eq!(m.get('b'), Some(pos(3, 1)), "later line: shifted down");
        assert_eq!(m.get('c'), Some(pos(0, 1)), "before: untouched");
    }

    #[test]
    fn deletes_pull_marks_back() {
        let mut m = marks(&[
            ('a', pos(1, 2)),
            ('b', pos(3, 4)),
            ('c', pos(5, 0)),
            ('d', pos(0, 0)),
        ]);
        // Delete from (1, 0) to (3, 2).
        m.after_delete(pos(1, 0), pos(3, 2));
        assert_eq!(m.get('a'), Some(pos(1, 0)), "inside: to the start");
        assert_eq!(m.get('b'), Some(pos(1, 2)), "on the end line: joined");
        assert_eq!(m.get('c'), Some(pos(3, 0)), "later: shifted up");
        assert_eq!(m.get('d'), Some(pos(0, 0)), "before: untouched");
    }

    #[test]
    fn undo_puts_marks_back_and_redo_swaps_them_again() {
        let mut m = marks(&[('a', pos(2, 3)), ('b', pos(5, 0))]);
        m.begin_step(1, 0);
        // Step 1 deletes line 2 ("two\n").
        let deleted = Change::Delete {
            at: pos(2, 0),
            text: "two\n".into(),
        };
        m.after_delete(pos(2, 0), pos(3, 0));
        assert_eq!((m.get('a'), m.get('b')), (Some(pos(2, 0)), Some(pos(4, 0))));
        m.set('c', pos(3, 1));

        m.undone_or_redone(1, &[deleted.inverted()]);
        assert_eq!(m.get('a'), Some(pos(2, 3)), "back on its own line");
        assert_eq!(m.get('b'), Some(pos(5, 0)));
        assert_eq!(m.get('c'), Some(pos(4, 1)), "set later: follows the text");

        m.undone_or_redone(1, &[deleted]);
        assert_eq!(m.get('a'), Some(pos(2, 0)), "redo: where it was after");
        assert_eq!(m.get('b'), Some(pos(4, 0)));
        assert_eq!(m.get('c'), Some(pos(3, 1)));
    }

    #[test]
    fn a_new_step_forgets_undone_ones() {
        let mut m = marks(&[('a', pos(0, 0))]);
        m.begin_step(1, 0);
        m.begin_step(2, 1);
        m.undone_or_redone(2, &[]);
        // Step 2 is undone; a new step (3) replaces it.
        m.begin_step(3, 1);
        let mut kept: Vec<_> = m.steps.keys().copied().collect();
        kept.sort();
        assert_eq!(kept, [1, 3]);
    }

    #[test]
    fn unknown_marks_are_none() {
        assert_eq!(Marks::default().get('z'), None);
    }
}
