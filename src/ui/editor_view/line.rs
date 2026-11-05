//! Conversions between character columns and terminal display cells.
//!
//! The cursor's character column and its on-screen column differ when a line
//! has wide characters (CJK, many emoji: two cells) or tabs (which extend to
//! the next tab stop). Everything that maps between the two goes through
//! here, so the cursor, text and highlights always line up.

use unicode_width::UnicodeWidthChar;

/// Cells taken by `c` when it starts at display column `col`.
fn char_width(c: char, col: usize, tab_width: usize) -> usize {
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

/// The part of `line` visible when scrolled `left` cells, `width` cells wide,
/// ready to draw: tabs are expanded to spaces.
///
/// A wide character or tab cut by the left edge is replaced with spaces so
/// the rest of the line stays aligned. A wide character cut by the right edge
/// is dropped.
pub fn visible_slice(line: &str, left: usize, width: usize, tab_width: usize) -> String {
    let right = left + width;
    let mut out = String::new();
    let mut col = 0;

    for c in line.chars() {
        if col >= right {
            break;
        }
        let w = char_width(c, col, tab_width);
        let visible_cells = (col + w).min(right).saturating_sub(col.max(left));
        if c == '\t' {
            out.extend(std::iter::repeat_n(' ', visible_cells));
        } else if col >= left && col + w <= right {
            out.push(c);
        } else if col < left && col + w > left {
            out.extend(std::iter::repeat_n(' ', visible_cells));
        }
        col += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_col_counts_wide_chars_twice() {
        assert_eq!(display_col("abc", 2, 4), 2);
        assert_eq!(display_col("日本語", 2, 4), 4);
    }

    #[test]
    fn tabs_extend_to_the_next_tab_stop() {
        assert_eq!(display_col("\tx", 1, 4), 4);
        assert_eq!(display_col("ab\tx", 3, 4), 4);
        assert_eq!(display_col("abcd\tx", 5, 4), 8, "a full tab at a stop");
        assert_eq!(display_col("\t\tx", 2, 8), 16);
    }

    #[test]
    fn visible_slice_of_ascii() {
        assert_eq!(visible_slice("hello world", 0, 5, 4), "hello");
        assert_eq!(visible_slice("hello world", 6, 100, 4), "world");
        assert_eq!(visible_slice("hi", 5, 10, 4), "");
    }

    #[test]
    fn tabs_are_drawn_as_spaces() {
        assert_eq!(visible_slice("a\tb", 0, 10, 4), "a   b");
        assert_eq!(visible_slice("\tx", 0, 10, 2), "  x");
    }

    #[test]
    fn tabs_cut_by_either_edge() {
        assert_eq!(visible_slice("\tx", 2, 10, 4), "  x", "left edge");
        assert_eq!(visible_slice("a\tb", 0, 3, 4), "a  ", "right edge");
    }

    #[test]
    fn wide_char_cut_on_left_is_padded() {
        assert_eq!(visible_slice("日本", 1, 10, 4), " 本");
    }

    #[test]
    fn wide_char_cut_on_right_is_dropped() {
        assert_eq!(visible_slice("a日", 0, 2, 4), "a");
    }
}
