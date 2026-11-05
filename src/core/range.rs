//! Ranges of text that operators act on.

use super::motion::first_non_blank;
use super::{Buffer, Position};

/// How a motion's target relates to the text it covers, as in Vim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    /// Covers up to, but not including, the target (`w`, `b`, `h`, `l`, `$`).
    Exclusive,
    /// Covers the target character too (`e`).
    Inclusive,
    /// Covers whole lines from the cursor's to the target's (`j`, `k`, `gg`, `G`).
    Linewise,
}

/// A span of text for an operator to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// Characters from `start` (inclusive) to `end` (exclusive).
    Chars { start: Position, end: Position },
    /// Whole lines, `first` to `last` inclusive.
    Lines { first: usize, last: usize },
}

impl Range {
    /// The range covered by moving from `from` to `to` with a motion of `kind`.
    pub fn from_motion(buffer: &Buffer, from: Position, to: Position, kind: MotionKind) -> Range {
        let (start, end) = if from <= to { (from, to) } else { (to, from) };
        match kind {
            MotionKind::Linewise => Range::Lines {
                first: start.row,
                last: end.row,
            },
            MotionKind::Inclusive => {
                let end = end.with_col((end.col + 1).min(buffer.line_len(end.row)));
                Range::Chars { start, end }
            }
            // Vim's `:h exclusive` rules for a motion ending at the start of a
            // later line: it covers whole lines if it also started at (or
            // before) the first non-blank, so `d}` removes the paragraph
            // cleanly; otherwise it stops at the end of the previous line.
            MotionKind::Exclusive if end.col == 0 && end.row > start.row => {
                if start.col <= first_non_blank(buffer, start.row) {
                    Range::Lines {
                        first: start.row,
                        last: end.row - 1,
                    }
                } else {
                    Range::Chars {
                        start,
                        end: buffer.line_end(end.row - 1),
                    }
                }
            }
            MotionKind::Exclusive => Range::Chars { start, end },
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Range::Chars { start, end } if start == end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn chars(start: Position, end: Position) -> Range {
        Range::Chars { start, end }
    }

    #[test]
    fn exclusive_excludes_target_and_orders_endpoints() {
        let b = Buffer::from_text("hello world");
        let r = Range::from_motion(&b, pos(0, 6), pos(0, 0), MotionKind::Exclusive);
        assert_eq!(r, chars(pos(0, 0), pos(0, 6)));
    }

    #[test]
    fn inclusive_includes_target() {
        let b = Buffer::from_text("hello");
        let r = Range::from_motion(&b, pos(0, 0), pos(0, 4), MotionKind::Inclusive);
        assert_eq!(r, chars(pos(0, 0), pos(0, 5)));
    }

    #[test]
    fn inclusive_is_clamped_to_line_end() {
        let b = Buffer::from_text("ab");
        let r = Range::from_motion(&b, pos(0, 0), pos(0, 2), MotionKind::Inclusive);
        assert_eq!(r, chars(pos(0, 0), pos(0, 2)));
    }

    #[test]
    fn exclusive_motion_to_next_line_start_from_mid_line_stops_at_line_end() {
        let b = Buffer::from_text("foo bar\nbaz");
        let r = Range::from_motion(&b, pos(0, 4), pos(1, 0), MotionKind::Exclusive);
        assert_eq!(r, chars(pos(0, 4), pos(0, 7)));
    }

    #[test]
    fn exclusive_motion_from_line_start_to_line_start_is_linewise() {
        let b = Buffer::from_text("  a\nb\n\nc");
        let r = Range::from_motion(&b, pos(0, 2), pos(2, 0), MotionKind::Exclusive);
        assert_eq!(r, Range::Lines { first: 0, last: 1 });
    }

    #[test]
    fn linewise_covers_rows() {
        let b = Buffer::from_text("a\nb\nc");
        let r = Range::from_motion(&b, pos(2, 0), pos(1, 0), MotionKind::Linewise);
        assert_eq!(r, Range::Lines { first: 1, last: 2 });
    }

    #[test]
    fn emptiness() {
        assert!(chars(pos(0, 1), pos(0, 1)).is_empty());
        assert!(!Range::Lines { first: 0, last: 0 }.is_empty());
    }
}
