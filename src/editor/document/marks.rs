//! A document's marks (`ma` … `'a`), kept in place as the text changes.

use std::collections::HashMap;

use crate::core::Position;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Marks {
    marks: HashMap<char, Position>,
}

impl Marks {
    pub fn set(&mut self, name: char, position: Position) {
        self.marks.insert(name, position);
    }

    pub fn get(&self, name: char) -> Option<Position> {
        self.marks.get(&name).copied()
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
    fn unknown_marks_are_none() {
        assert_eq!(Marks::default().get('z'), None);
    }
}
