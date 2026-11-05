//! Conversions between character columns and terminal display cells.
//!
//! Wide characters (e.g. CJK, many emoji) occupy two cells, so the cursor's
//! character column and its on-screen column can differ.

use unicode_width::UnicodeWidthChar;

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// The display column of the character at `char_col`.
pub fn display_col(line: &str, char_col: usize) -> usize {
    line.chars().take(char_col).map(char_width).sum()
}

/// The part of `line` visible when scrolled `left` cells, `width` cells wide.
///
/// A wide character cut by the left edge is replaced with spaces so the rest
/// of the line stays aligned. One cut by the right edge is dropped.
pub fn visible_slice(line: &str, left: usize, width: usize) -> String {
    let right = left + width;
    let mut out = String::new();
    let mut col = 0;

    for c in line.chars() {
        if col >= right {
            break;
        }
        let w = char_width(c);
        if col >= left && col + w <= right {
            out.push(c);
        } else if col < left && col + w > left {
            out.extend(std::iter::repeat_n(' ', (col + w - left).min(width)));
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
        assert_eq!(display_col("abc", 2), 2);
        assert_eq!(display_col("日本語", 2), 4);
    }

    #[test]
    fn visible_slice_of_ascii() {
        assert_eq!(visible_slice("hello world", 0, 5), "hello");
        assert_eq!(visible_slice("hello world", 6, 100), "world");
        assert_eq!(visible_slice("hi", 5, 10), "");
    }

    #[test]
    fn wide_char_cut_on_left_is_padded() {
        assert_eq!(visible_slice("日本", 1, 10), " 本");
    }

    #[test]
    fn wide_char_cut_on_right_is_dropped() {
        assert_eq!(visible_slice("a日", 0, 2), "a");
    }
}
