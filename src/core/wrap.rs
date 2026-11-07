//! Soft wrapping: where a line breaks into screen rows.
//!
//! A line too wide for its window continues on the next screen rows. It
//! breaks after a space where it can, so words stay whole, and mid-word
//! only when a word is wider than the window. A space at the edge hangs
//! past it rather than pushing the word before it down. A wide character (two cells)
//! that would be cut by the edge moves to the next row. Display columns
//! (tabs, wide characters) are counted from the start of the line, as
//! without wrapping, so the text doesn't shift when wrapping is turned on.

use super::columns::char_width;

/// The char column each screen row of `line` starts at, when it's drawn
/// `width` cells wide: `[0]` for a line that fits. With `width` 0 (no room
/// at all) the line isn't broken.
pub fn row_starts(line: &str, width: usize, tab_width: usize) -> Vec<usize> {
    row_starts_indented(line, width, tab_width, 0)
}

/// [`row_starts`], with the rows after the first `continued` cells
/// narrower (they're indented, or marked as continuing).
pub fn row_starts_indented(
    line: &str,
    width: usize,
    tab_width: usize,
    continued: usize,
) -> Vec<usize> {
    let mut starts = vec![0];
    if width == 0 {
        return starts;
    }
    let later_width = width.saturating_sub(continued).max(1);
    // Display columns of the current row's start, and of each char.
    let mut row_start_cell = 0;
    let mut cells = Vec::with_capacity(line.len());
    // Where the row could break: after the last space seen in it.
    let mut break_at: Option<usize> = None;
    let mut cell = 0;
    for (i, c) in line.chars().enumerate() {
        let w = char_width(c, cell, tab_width);
        cells.push(cell);
        // A space may hang past the edge, so the break comes after it.
        let hangs = c == ' ';
        let width = if starts.len() == 1 {
            width
        } else {
            later_width
        };
        if !hangs && cell + w - row_start_cell > width && i > *starts.last().expect("never empty") {
            let at = match break_at {
                Some(at) if at > *starts.last().expect("never empty") => at,
                _ => i,
            };
            starts.push(at);
            row_start_cell = cells[at];
            break_at = None;
            // A word carried over may still not fit with this char.
            if cell + w - row_start_cell > later_width && at < i {
                starts.push(i);
                row_start_cell = cell;
            }
        }
        cell += w;
        if c == ' ' || c == '\t' {
            break_at = Some(i + 1);
        }
    }
    starts
}

/// Which of `starts`' rows char column `col` is on. A column past the end
/// of the line (the cursor after the last char) is on the last row.
pub fn row_of(starts: &[usize], col: usize) -> usize {
    starts.iter().rposition(|&start| start <= col).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `line` at `width`, as text.
    fn rows(line: &str, width: usize) -> Vec<String> {
        let starts = row_starts(line, width, 4);
        let chars: Vec<char> = line.chars().collect();
        let mut out = Vec::new();
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(chars.len());
            out.push(chars[start..end].iter().collect());
        }
        out
    }

    #[test]
    fn short_lines_are_one_row() {
        assert_eq!(row_starts("hello", 10, 4), [0]);
        assert_eq!(row_starts("", 10, 4), [0]);
        assert_eq!(row_starts("exactly10!", 10, 4), [0]);
    }

    #[test]
    fn breaks_after_spaces() {
        assert_eq!(rows("the quick brown fox", 10), ["the quick ", "brown fox"]);
        assert_eq!(
            rows("aaa bbb ccc ddd", 7),
            ["aaa bbb ", "ccc ddd"],
            "a space hangs"
        );
    }

    #[test]
    fn words_wider_than_the_window_are_cut() {
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(rows("a abcdefghij", 4), ["a ", "abcd", "efgh", "ij"]);
    }

    #[test]
    fn wide_characters_move_to_the_next_row_whole() {
        assert_eq!(rows("ab日本", 5), ["ab日", "本"]);
        assert_eq!(rows("日本語", 3), ["日", "本", "語"]);
    }

    #[test]
    fn tabs_count_from_the_line_start() {
        // `\t` at 0 is 4 cells; "ab" fits after it in 6, "cd" doesn't.
        assert_eq!(rows("\tab cd", 7), ["\tab ", "cd"]);
    }

    #[test]
    fn no_width_means_no_breaking() {
        assert_eq!(row_starts("anything at all", 0, 4), [0]);
    }

    #[test]
    fn continued_rows_can_be_narrower() {
        let starts = row_starts_indented("aaaa bbbb cccc dddd", 10, 4, 4);
        let chars: Vec<char> = "aaaa bbbb cccc dddd".chars().collect();
        let rows: Vec<String> = starts
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let e = starts.get(i + 1).copied().unwrap_or(chars.len());
                chars[s..e].iter().collect()
            })
            .collect();
        assert_eq!(
            rows,
            ["aaaa bbbb ", "cccc ", "dddd"],
            "6 cells after the first"
        );
    }

    #[test]
    fn which_row_a_column_is_on() {
        let starts = row_starts("the quick brown fox", 10, 4);
        assert_eq!(row_of(&starts, 0), 0);
        assert_eq!(row_of(&starts, 9), 0);
        assert_eq!(row_of(&starts, 10), 1);
        assert_eq!(row_of(&starts, 19), 1, "past the end");
    }
}
