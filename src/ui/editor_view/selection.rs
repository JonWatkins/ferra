//! Which cells of a line a selection covers.

use crate::core::Range;

/// The selected character columns on `row`, as a half-open `[start, end)`.
///
/// `end` may be `line_len + 1`, meaning the line break is selected too; it's
/// drawn as one highlighted cell past the text, as in Vim, so selected empty
/// lines stay visible.
pub fn selected_cols(range: Range, row: usize, line_len: usize) -> Option<(usize, usize)> {
    let with_break = line_len + 1;
    match range {
        Range::Lines { first, last } if (first..=last).contains(&row) => Some((0, with_break)),
        Range::Lines { .. } => None,
        Range::Chars { start, end } => {
            if row < start.row || row > end.row {
                return None;
            }
            let from = if row == start.row { start.col } else { 0 };
            let to = if row == end.row { end.col } else { with_break };
            (from < to).then_some((from, to))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position;

    fn chars(start: (usize, usize), end: (usize, usize)) -> Range {
        Range::Chars {
            start: Position::new(start.0, start.1),
            end: Position::new(end.0, end.1),
        }
    }

    #[test]
    fn single_line_char_selection() {
        let r = chars((0, 2), (0, 5));
        assert_eq!(selected_cols(r, 0, 10), Some((2, 5)));
        assert_eq!(selected_cols(r, 1, 10), None);
    }

    #[test]
    fn multi_line_char_selection_includes_line_breaks() {
        let r = chars((0, 3), (2, 1));
        assert_eq!(selected_cols(r, 0, 5), Some((3, 6)));
        assert_eq!(selected_cols(r, 1, 4), Some((0, 5)));
        assert_eq!(selected_cols(r, 2, 4), Some((0, 1)));
    }

    #[test]
    fn selection_ending_at_line_start_skips_that_line() {
        let r = chars((0, 0), (1, 0));
        assert_eq!(selected_cols(r, 0, 3), Some((0, 4)));
        assert_eq!(selected_cols(r, 1, 3), None);
    }

    #[test]
    fn line_selection_covers_whole_lines() {
        let r = Range::Lines { first: 1, last: 2 };
        assert_eq!(selected_cols(r, 0, 3), None);
        assert_eq!(selected_cols(r, 1, 3), Some((0, 4)));
        assert_eq!(
            selected_cols(r, 2, 0),
            Some((0, 1)),
            "empty line shows one cell"
        );
    }
}
