//! Character columns and display columns.
//!
//! A character column counts chars; a display column counts terminal cells.
//! They differ for wide characters (two cells) and tabs (up to the next tab
//! stop). Visual block selections are rectangles of display columns, so the
//! editor needs this as much as the UI does.

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

#[cfg(test)]
mod tests {
    use super::*;

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
