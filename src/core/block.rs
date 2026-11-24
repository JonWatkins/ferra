//! Rectangular (visual block) selections.
//!
//! A block covers the same display columns on each of its rows, so it lines
//! up on screen even with tabs, wide characters and virtual text (inlay
//! hints): each row's columns come as a [`LineCells`], the line as drawn.
//! Virtual text takes cells but is never part of the block; a row whose
//! block cells are all virtual text has nothing in it. A character partly
//! inside the block counts as inside; [`Block::row`] says how many of its
//! cells stick out, so edits can keep those as spaces (as Vim does). With
//! `to_eol` (`$` in block mode) each row's block runs to the end of that row.

use super::columns::LineCells;

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
    /// (`(row, char col)`), where `line(row)` says where each row's
    /// characters are drawn.
    pub fn between(
        a: (usize, usize),
        b: (usize, usize),
        to_eol: bool,
        line: impl Fn(usize) -> LineCells,
    ) -> Block {
        let cells = |(row, col): (usize, usize)| line(row).cells(col);
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

    /// The characters of a line inside the block, as a half-open char
    /// range. Empty (at the line's end) when the line stops before the
    /// block, or has only virtual text in it.
    pub fn chars_in(&self, line: &LineCells) -> (usize, usize) {
        let mut start = None;
        let mut end = 0;
        for i in 0..line.len() {
            let (first, last) = line.cells(i);
            let inside = last >= self.left && (self.to_eol || first <= self.right);
            if inside {
                start.get_or_insert(i);
                end = i + 1;
            } else if first > self.right && !self.to_eol {
                break;
            }
        }
        match start {
            Some(start) => (start, end),
            None => (line.len(), line.len()),
        }
    }

    /// The block's part of a line: [`Block::chars_in`], plus the cells of
    /// its first and last characters that stick out of the block (a tab or
    /// wide character cut by an edge).
    pub fn row(&self, line: &LineCells) -> BlockRow {
        let (start, end) = self.chars_in(line);
        let mut row = BlockRow {
            start,
            end,
            before: 0,
            after: 0,
        };
        if start < end {
            let (first, _) = line.cells(start);
            row.before = self.left.saturating_sub(first);
            if !self.to_eol {
                let (_, last) = line.cells(end - 1);
                row.after = last.saturating_sub(self.right);
            }
        }
        row
    }

    /// The block's text on `line` (drawn as `cells`), with the covered
    /// cells of characters cut by its edges as spaces. Virtual text in the
    /// block isn't part of it.
    pub fn text_in(&self, line: &str, cells: &LineCells) -> String {
        let row = self.row(cells);
        if row.before == 0 && row.after == 0 {
            return line.chars().skip(row.start).take(row.len()).collect();
        }
        let mut text = String::new();
        for (i, c) in line.chars().enumerate().skip(row.start).take(row.len()) {
            let (first, last) = cells.cells(i);
            let width = last - first + 1;
            let mut outside = 0;
            if i == row.start {
                outside += row.before;
            }
            if i + 1 == row.end {
                outside += row.after;
            }
            if outside == 0 {
                text.push(c);
            } else {
                text.push_str(&" ".repeat(width.saturating_sub(outside)));
            }
        }
        text
    }

    /// Whether a line's text reaches the block's left edge.
    pub fn reaches(&self, line: &LineCells) -> bool {
        line.end() > self.left
    }
}

/// One row of a block: the characters `start..end`, of which the first
/// sticks `before` cells out to the left and the last `after` cells out to
/// the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRow {
    pub start: usize,
    pub end: usize,
    pub before: usize,
    pub after: usize,
}

impl BlockRow {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// What's left in place of the deleted characters: spaces for the cells
    /// outside the block.
    pub fn remainder(&self) -> String {
        " ".repeat(self.before + self.after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(line: &str) -> LineCells {
        LineCells::new(line, 4)
    }

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
        assert_eq!(block(1, 2).chars_in(&cells("abcd")), (1, 3));
        assert_eq!(
            block(1, 2).chars_in(&cells("a")),
            (1, 1),
            "too short: empty"
        );
        assert_eq!(block(1, 2).chars_in(&cells("")), (0, 0));
    }

    #[test]
    fn partly_covered_tabs_and_wide_chars_count_as_inside() {
        // The tab covers cells 1-3.
        assert_eq!(block(2, 2).chars_in(&cells("a\tb")), (1, 2));
        // 日 covers cells 0-1, 本 2-3.
        assert_eq!(block(1, 2).chars_in(&cells("日本x")), (0, 2));
    }

    #[test]
    fn rows_count_the_cells_that_stick_out() {
        // The tab covers cells 1-3; the block is cells 2-4.
        let row = block(2, 4).row(&cells("a\tbc"));
        assert_eq!(
            (row.start, row.end, row.before, row.after),
            (1, 3, 1, 0),
            "the tab's first cell is outside"
        );
        assert_eq!(block(2, 4).text_in("a\tbc", &cells("a\tbc")), "  b");
        assert_eq!(row.remainder(), " ");
        // A tab across the right edge (cells 0-3, block 0-1).
        let row = block(0, 1).row(&cells("\tx"));
        assert_eq!((row.before, row.after), (0, 2));
        assert_eq!(block(0, 1).text_in("\tx", &cells("\tx")), "  ");
        // Wide characters cut by either edge.
        assert_eq!(
            block(1, 2).text_in("日本x", &cells("日本x")),
            "  ",
            "half of each"
        );
        assert_eq!(
            block(0, 3).text_in("日本x", &cells("日本x")),
            "日本",
            "nothing cut"
        );
        let eol = Block {
            to_eol: true,
            ..block(2, 2)
        };
        assert_eq!(eol.row(&cells("a\tbcdef")).after, 0);
    }

    #[test]
    fn to_eol_takes_the_rest_of_each_line() {
        let b = Block {
            to_eol: true,
            ..block(1, 1)
        };
        assert_eq!(b.chars_in(&cells("abcdef")), (1, 6));
        assert_eq!(b.chars_in(&cells("a")), (1, 1));
    }

    #[test]
    fn between_uses_display_columns() {
        let lines = ["\tx", "abcdef"];
        // The tab's cells are 0-3; `c` is cell 2.
        let b = Block::between((0, 0), (1, 2), false, |row| cells(lines[row]));
        assert_eq!((b.first, b.last, b.left, b.right), (0, 1, 0, 3));
        assert!(b.reaches(&cells("abcdef")));
        assert!(!Block { left: 6, ..b }.reaches(&cells("abcdef")));
    }

    #[test]
    fn virtual_text_takes_cells_but_is_never_in_the_block() {
        // `f(1, 2)` drawn as `f(x: 1, y: 2)`.
        let drawn = "f(x: 1, y: 2)";
        let at = [0, 1, 5, 6, 7, 11, 12, 13];
        let gaps = [0, 1, 2, 6, 7, 8, 12, 13];
        let line = LineCells::drawn(drawn, &at, &gaps, 4);
        // On screen, cells 2-6 are `x: 1,`.
        let b = block(2, 6);
        assert_eq!(
            b.chars_in(&line),
            (2, 4),
            "`1,`: the hint's cells hold no text"
        );
        assert_eq!(b.text_in("f(1, 2)", &line), "1,");
        // Cells 2-4 are only the hint.
        assert_eq!(block(2, 4).chars_in(&line), (7, 7));
        assert!(block(2, 4).reaches(&line));
        // The corners are where the chars are drawn.
        let plain = cells("f(1, 2)");
        let b = Block::between((0, 2), (1, 2), false, |row| {
            if row == 0 {
                plain.clone()
            } else {
                line.clone()
            }
        });
        assert_eq!((b.left, b.right), (2, 5), "from `1` above to `1` below");
    }
}
