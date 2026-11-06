//! Rectangular (visual block) selections.
//!
//! A block covers the same display columns on each of its rows, so it lines
//! up on screen even with tabs and wide characters. A character partly
//! inside the block counts as inside. With `to_eol` (`$` in block mode) each
//! row's block runs to the end of that row.

use super::Buffer;
use super::columns::{char_cells, display_col};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub first: usize,
    pub last: usize,
    /// First display column covered.
    pub left: usize,
    /// Last display column covered (inclusive).
    pub right: usize,
    /// Every row runs to its end, however long (`$`).
    pub to_eol: bool,
}

impl Block {
    /// The block with corners at the characters `a` and `b`
    /// (`(row, char col)`).
    pub fn between(
        buffer: &Buffer,
        a: (usize, usize),
        b: (usize, usize),
        to_eol: bool,
        tab_width: usize,
    ) -> Block {
        let cells = |(row, col): (usize, usize)| char_cells(&buffer.line(row), col, tab_width);
        let (a_cells, b_cells) = (cells(a), cells(b));
        Block {
            first: a.0.min(b.0),
            last: a.0.max(b.0),
            left: a_cells.0.min(b_cells.0),
            right: a_cells.1.max(b_cells.1),
            to_eol,
        }
    }

    pub fn rows(&self) -> std::ops::RangeInclusive<usize> {
        self.first..=self.last
    }

    /// The characters of `line` inside the block, as a half-open char range.
    /// Empty (at the line's end) when the line stops before the block.
    pub fn chars_in(&self, line: &str, tab_width: usize) -> (usize, usize) {
        let mut start = None;
        let mut end = 0;
        let mut col = 0;
        for (i, c) in line.chars().enumerate() {
            let width = super::columns::char_width(c, col, tab_width).max(1);
            let last_cell = col + width - 1;
            let inside = last_cell >= self.left && (self.to_eol || col <= self.right);
            if inside {
                start.get_or_insert(i);
                end = i + 1;
            } else if col > self.right && !self.to_eol {
                break;
            }
            col += width;
        }
        match start {
            Some(start) => (start, end),
            None => {
                let len = line.chars().count();
                (len, len)
            }
        }
    }

    /// Whether `line` reaches the block's left edge.
    pub fn reaches(&self, line: &str, tab_width: usize) -> bool {
        display_col(line, usize::MAX, tab_width) > self.left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(left: usize, right: usize) -> Block {
        Block {
            first: 0,
            last: 0,
            left,
            right,
            to_eol: false,
        }
    }

    #[test]
    fn chars_in_plain_text() {
        assert_eq!(block(1, 2).chars_in("abcd", 4), (1, 3));
        assert_eq!(block(1, 2).chars_in("a", 4), (1, 1), "too short: empty");
        assert_eq!(block(1, 2).chars_in("", 4), (0, 0));
    }

    #[test]
    fn partly_covered_tabs_and_wide_chars_count_as_inside() {
        // The tab covers cells 1-3.
        assert_eq!(block(2, 2).chars_in("a\tb", 4), (1, 2));
        // 日 covers cells 0-1, 本 2-3.
        assert_eq!(block(1, 2).chars_in("日本x", 4), (0, 2));
    }

    #[test]
    fn to_eol_takes_the_rest_of_each_line() {
        let b = Block {
            to_eol: true,
            ..block(1, 1)
        };
        assert_eq!(b.chars_in("abcdef", 4), (1, 6));
        assert_eq!(b.chars_in("a", 4), (1, 1));
    }

    #[test]
    fn between_uses_display_columns() {
        let buffer = Buffer::from_text("\tx\nabcdef");
        // The tab's cells are 0-3; `c` is cell 2.
        let b = Block::between(&buffer, (0, 0), (1, 2), false, 4);
        assert_eq!((b.first, b.last, b.left, b.right), (0, 1, 0, 3));
        assert!(b.reaches("abcdef", 4));
        assert!(!Block { left: 6, ..b }.reaches("abcdef", 4));
    }
}
