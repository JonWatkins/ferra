//! Character columns and display columns.
//!
//! A character column counts chars; a display column counts terminal cells.
//! They differ for wide characters (two cells) and tabs (up to the next tab
//! stop). Visual block selections are rectangles of display columns, so the
//! editor needs this as much as the UI does.
//!
//! [`LineCells`] is a line's display columns as drawn, which can also count
//! virtual text put in the line (the editor builds it from a line's display
//! line). Blocks are counted in it, so they line up over inlay hints.

use unicode_width::UnicodeWidthChar;

/// Cells taken by `c` when it starts at display column `col`.
pub fn char_width(c: char, col: usize, tab_width: usize) -> usize {
    if c == '\t' {
        let tab = tab_width.max(1);
        tab - col % tab
    } else {
        c.width().unwrap_or(0)
    }
}

/// The display column of the character at `char_col`.
pub fn display_col(line: &str, char_col: usize, tab_width: usize) -> usize {
    line.chars()
        .take(char_col)
        .fold(0, |col, c| col + char_width(c, col, tab_width))
}

/// The display width of the whole line.
pub fn display_width(line: &str, tab_width: usize) -> usize {
    display_col(line, usize::MAX, tab_width)
}

/// The cells (`start`, last cell inclusive) taken by the character at
/// `char_col`. Past the end of the line, it's one cell.
pub fn char_cells(line: &str, char_col: usize, tab_width: usize) -> (usize, usize) {
    let start = display_col(line, char_col, tab_width);
    let width = line
        .chars()
        .nth(char_col)
        .map_or(1, |c| char_width(c, start, tab_width).max(1));
    (start, start + width - 1)
}

/// The character column at display column `target`: the character covering
/// that cell, or the line length if the line is shorter.
pub fn char_col_at(line: &str, target: usize, tab_width: usize) -> usize {
    let mut col = 0;
    for (i, c) in line.chars().enumerate() {
        let width = char_width(c, col, tab_width);
        if target < col + width.max(1) {
            return i;
        }
        col += width;
    }
    line.chars().count()
}

/// Where each character of a line is on screen, as the line is drawn: with
/// tabs and wide characters, and with any text drawn in it that isn't in
/// the file (virtual text: inlay hints), which pushes what follows along.
///
/// Visual block selections and their edits work in these columns, so a
/// block lines up with what's on screen whatever hints are in the rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCells {
    /// For each character, and then the line's end: its first display
    /// column, its width, and the column of the gap before any virtual
    /// text in front of it (where text inserted before it is drawn).
    chars: Vec<CharCells>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CharCells {
    start: usize,
    width: usize,
    gap: usize,
}

impl LineCells {
    /// A line drawn as it is, with nothing put in.
    pub fn new(line: &str, tab_width: usize) -> LineCells {
        let mut chars = Vec::with_capacity(line.len() + 1);
        let mut col = 0;
        for c in line.chars() {
            let width = char_width(c, col, tab_width);
            chars.push(CharCells {
                start: col,
                width,
                gap: col,
            });
            col += width;
        }
        chars.push(CharCells {
            start: col,
            width: 1,
            gap: col,
        });
        LineCells { chars }
    }

    /// A line drawn as `drawn`, the real text with virtual text put in:
    /// real character `i` (or, for `i` the line's length, its end) is
    /// `drawn`'s character `at[i]`, and the gap before any virtual text in
    /// front of it is at `gaps[i]` (both counted in `drawn`'s chars).
    pub fn drawn(drawn: &str, at: &[usize], gaps: &[usize], tab_width: usize) -> LineCells {
        let mut cols = Vec::with_capacity(drawn.len() + 1);
        let mut widths = Vec::with_capacity(drawn.len());
        let mut col = 0;
        for c in drawn.chars() {
            let width = char_width(c, col, tab_width);
            cols.push(col);
            widths.push(width);
            col += width;
        }
        cols.push(col);
        let last = at.len().saturating_sub(1);
        let chars = at
            .iter()
            .zip(gaps)
            .enumerate()
            .map(|(i, (&at, &gap))| CharCells {
                start: cols[at.min(cols.len() - 1)],
                width: if i == last {
                    1
                } else {
                    widths.get(at).copied().unwrap_or(1)
                },
                gap: cols[gap.min(cols.len() - 1)],
            })
            .collect();
        LineCells { chars }
    }

    /// The number of characters in the line.
    pub fn len(&self) -> usize {
        self.chars.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, col: usize) -> CharCells {
        self.chars[col.min(self.len())]
    }

    /// The display column of the character at `col` (`display_col`).
    pub fn start(&self, col: usize) -> usize {
        self.get(col).start
    }

    /// The cells (`start`, last cell inclusive) taken by the character at
    /// `col` (`char_cells`). Past the end of the line, it's one cell.
    pub fn cells(&self, col: usize) -> (usize, usize) {
        let c = self.get(col);
        (c.start, c.start + c.width.max(1) - 1)
    }

    /// Where text inserted before the character at `col` is drawn: before
    /// any virtual text in front of it, as typing there pushes that along.
    pub fn gap(&self, col: usize) -> usize {
        self.get(col).gap
    }

    /// The display column just after the line's own text (any virtual
    /// text at its end isn't counted): where text added at the end goes.
    pub fn end(&self) -> usize {
        self.gap(self.len())
    }

    /// The character column at display column `target`: the character
    /// covering that cell, or the first one after it (a cell of virtual
    /// text), or the line length if the line ends before it
    /// (`char_col_at`).
    pub fn char_at(&self, target: usize) -> usize {
        self.chars[..self.len()]
            .iter()
            .position(|c| target < c.start + c.width.max(1))
            .unwrap_or(self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_cells_match_the_plain_functions() {
        for line in ["a\tb", "日本x", "", "abc", "ab\t\tcd"] {
            let cells = LineCells::new(line, 4);
            for col in 0..=line.chars().count() + 1 {
                assert_eq!(cells.start(col), display_col(line, col, 4));
                assert_eq!(cells.cells(col), char_cells(line, col, 4), "{line:?} {col}");
            }
            for target in 0..12 {
                assert_eq!(cells.char_at(target), char_col_at(line, target, 4));
            }
            assert_eq!(cells.end(), display_width(line, 4));
        }
    }

    #[test]
    fn line_cells_count_virtual_text_in_the_line() {
        // `foo(1)` drawn as `foo(x: 1)`: the hint `x: ` is before the `1`.
        let drawn = "foo(x: 1)";
        let at = [0, 1, 2, 3, 7, 8, 9];
        let gaps = [0, 1, 2, 3, 4, 8, 9];
        let cells = LineCells::drawn(drawn, &at, &gaps, 4);
        assert_eq!(cells.len(), 6);
        assert_eq!(cells.cells(4), (7, 7), "the 1 is drawn after the hint");
        assert_eq!(cells.gap(4), 4, "text typed before it goes before the hint");
        assert_eq!(cells.char_at(5), 4, "a hint's cell: the char after it");
        assert_eq!(cells.char_at(8), 5);
        assert_eq!(cells.end(), 9);
        // A tab after a hint is drawn to the next stop from where it starts.
        let cells = LineCells::drawn("ab\tc", &[0, 2, 3, 4], &[0, 1, 3, 4], 4);
        assert_eq!(cells.cells(1), (2, 3), "`\\t` after the hint `b`");
    }

    #[test]
    fn tabs_and_wide_chars() {
        assert_eq!(display_col("a\tb", 2, 4), 4);
        assert_eq!(display_col("日本", 1, 4), 2);
        assert_eq!(display_width("\tx", 4), 5);
        assert_eq!(char_cells("a\tb", 1, 4), (1, 3));
        assert_eq!(char_cells("ab", 5, 4), (2, 2), "past the end: one cell");
    }

    #[test]
    fn char_col_at_finds_the_covering_char() {
        assert_eq!(char_col_at("a\tb", 2, 4), 1, "inside the tab");
        assert_eq!(char_col_at("a\tb", 4, 4), 2);
        assert_eq!(char_col_at("ab", 9, 4), 2, "past the end");
        assert_eq!(char_col_at("日本", 1, 4), 0);
    }
}
