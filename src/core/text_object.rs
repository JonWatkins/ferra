//! Text objects: ranges selected *around* the cursor rather than reached by
//! moving it (`iw`, `a"`, `i(`, ...). Pure functions, like motions.
//!
//! Each returns `None` when there's no such object at the cursor (e.g. `i"`
//! on a line with no quotes); the operator is then cancelled, as in Vim.

use super::motion::{CharClass, char_class, class_at, next, prev};
use super::{Buffer, Position, Range};

/// `iw`: the run of word, punctuation or whitespace characters under the cursor.
pub fn inner_word(buffer: &Buffer, pos: Position) -> Option<Range> {
    let (start, end) = class_run(buffer, pos)?;
    Some(Range::Chars { start, end })
}

/// `aw`: a word plus the whitespace after it (or before it, if there's none after).
/// On whitespace, it's the whitespace plus the following word.
pub fn a_word(buffer: &Buffer, pos: Position) -> Option<Range> {
    let (start, end) = class_run(buffer, pos)?;
    let line_len = buffer.line_len(pos.row);
    let is_space = |p: Position| class_at(buffer, p) == CharClass::Whitespace;

    let (start, end) = if is_space(pos) {
        let word_end = (end.col < line_len)
            .then(|| class_run(buffer, end))
            .flatten()
            .map_or(end, |(_, e)| e);
        (start, word_end)
    } else if end.col < line_len && is_space(end) {
        let (_, space_end) = class_run(buffer, end)?;
        (start, space_end)
    } else if start.col > 0 && is_space(start.with_col(start.col - 1)) {
        let (space_start, _) = class_run(buffer, start.with_col(start.col - 1))?;
        (space_start, end)
    } else {
        (start, end)
    };
    Some(Range::Chars { start, end })
}

/// The maximal run of same-class characters containing `pos`, on its line.
fn class_run(buffer: &Buffer, pos: Position) -> Option<(Position, Position)> {
    let chars: Vec<char> = buffer.line(pos.row).chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = pos.col.min(chars.len() - 1);
    let class = char_class(chars[col]);

    let start = (0..col)
        .rev()
        .take_while(|&c| char_class(chars[c]) == class)
        .last()
        .unwrap_or(col);
    let end = (col..chars.len())
        .take_while(|&c| char_class(chars[c]) == class)
        .last()
        .map_or(col, |c| c + 1);
    Some((pos.with_col(start), pos.with_col(end)))
}

/// `i"` / `a"`: the quoted string around (or after) the cursor, on its line.
pub fn quoted(buffer: &Buffer, pos: Position, quote: char, around: bool) -> Option<Range> {
    let quotes: Vec<usize> = buffer
        .line(pos.row)
        .chars()
        .enumerate()
        .filter_map(|(i, c)| (c == quote).then_some(i))
        .collect();

    let (open, close) = quotes
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        // The pair containing the cursor, or else the first one after it.
        .find(|&(_, close)| pos.col <= close)?;

    let (start, end) = if around {
        (open, close + 1)
    } else {
        (open + 1, close)
    };
    Some(Range::Chars {
        start: pos.with_col(start),
        end: pos.with_col(end),
    })
}

/// `i(` / `a(` and friends: the innermost bracket pair enclosing the cursor.
/// Works across lines and respects nesting.
pub fn bracketed(
    buffer: &Buffer,
    pos: Position,
    open: char,
    close: char,
    around: bool,
) -> Option<Range> {
    let open_pos = find_open(buffer, pos, open, close)?;
    let close_pos = find_close(buffer, open_pos, open, close)?;

    if around {
        return Some(Range::Chars {
            start: open_pos,
            end: close_pos.with_col(close_pos.col + 1),
        });
    }

    let start = open_pos.with_col(open_pos.col + 1);
    // Vim: when the brackets sit on their own lines, the inner object is the
    // lines between them, leaving the bracket lines intact.
    let open_ends_line = start.col == buffer.line_len(open_pos.row);
    let close_starts_line = buffer
        .line(close_pos.row)
        .chars()
        .take(close_pos.col)
        .all(char::is_whitespace);
    if open_ends_line && close_starts_line && close_pos.row > open_pos.row {
        if close_pos.row - open_pos.row == 1 {
            return Some(Range::Chars {
                start: close_pos,
                end: close_pos,
            });
        }
        return Some(Range::Lines {
            first: open_pos.row + 1,
            last: close_pos.row - 1,
        });
    }
    Some(Range::Chars {
        start,
        end: close_pos,
    })
}

pub(super) fn find_open(
    buffer: &Buffer,
    pos: Position,
    open: char,
    close: char,
) -> Option<Position> {
    let mut depth = 0usize;
    let mut cursor = Some(pos);
    while let Some(p) = cursor {
        match buffer.char_at(p) {
            Some(c) if c == open => {
                if depth == 0 {
                    return Some(p);
                }
                depth -= 1;
            }
            // A close bracket under the cursor belongs to the pair we want.
            Some(c) if c == close && p != pos => depth += 1,
            _ => {}
        }
        cursor = prev(buffer, p);
    }
    None
}

pub(super) fn find_close(
    buffer: &Buffer,
    open_pos: Position,
    open: char,
    close: char,
) -> Option<Position> {
    let mut depth = 0usize;
    let mut cursor = next(buffer, open_pos);
    while let Some(p) = cursor {
        match buffer.char_at(p) {
            Some(c) if c == close => {
                if depth == 0 {
                    return Some(p);
                }
                depth -= 1;
            }
            Some(c) if c == open => depth += 1,
            _ => {}
        }
        cursor = next(buffer, p);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    /// Applies a text object and returns the selected text.
    fn select(
        text: &str,
        at: Position,
        f: impl Fn(&Buffer, Position) -> Option<Range>,
    ) -> Option<String> {
        let b = Buffer::from_text(text);
        Some(match f(&b, at)? {
            Range::Chars { start, end } => b.text_range(start, end),
            Range::Lines { first, last } => b.lines()[first..=last].join("\n"),
        })
    }

    #[test]
    fn inner_word_selects_word_punctuation_or_space() {
        assert_eq!(select("foo bar", pos(0, 5), inner_word).unwrap(), "bar");
        assert_eq!(select("a.b", pos(0, 1), inner_word).unwrap(), ".");
        assert_eq!(select("a   b", pos(0, 2), inner_word).unwrap(), "   ");
        assert_eq!(select("", pos(0, 0), inner_word), None);
    }

    #[test]
    fn a_word_includes_trailing_space() {
        assert_eq!(select("foo bar baz", pos(0, 5), a_word).unwrap(), "bar ");
    }

    #[test]
    fn a_word_uses_leading_space_at_line_end() {
        assert_eq!(select("foo bar", pos(0, 5), a_word).unwrap(), " bar");
    }

    #[test]
    fn a_word_on_space_includes_next_word() {
        assert_eq!(select("foo  bar", pos(0, 3), a_word).unwrap(), "  bar");
    }

    #[test]
    fn quoted_inside_and_around() {
        let text = r#"say "hello there" now"#;
        let inner = |b: &Buffer, p| quoted(b, p, '"', false);
        let around = |b: &Buffer, p| quoted(b, p, '"', true);
        assert_eq!(select(text, pos(0, 8), inner).unwrap(), "hello there");
        assert_eq!(select(text, pos(0, 8), around).unwrap(), r#""hello there""#);
        assert_eq!(
            select(text, pos(0, 4), inner).unwrap(),
            "hello there",
            "on quote"
        );
    }

    #[test]
    fn quoted_finds_next_string_after_cursor() {
        let inner = |b: &Buffer, p| quoted(b, p, '\'', false);
        assert_eq!(select("x = 'a'", pos(0, 0), inner).unwrap(), "a");
        assert_eq!(select("no quotes", pos(0, 0), inner), None);
    }

    #[test]
    fn bracketed_respects_nesting() {
        let inner = |b: &Buffer, p| bracketed(b, p, '(', ')', false);
        let around = |b: &Buffer, p| bracketed(b, p, '(', ')', true);
        let text = "f(a, g(b), c)";
        assert_eq!(select(text, pos(0, 3), inner).unwrap(), "a, g(b), c");
        assert_eq!(select(text, pos(0, 7), inner).unwrap(), "b");
        assert_eq!(select(text, pos(0, 7), around).unwrap(), "(b)");
        assert_eq!(
            select(text, pos(0, 12), inner).unwrap(),
            "a, g(b), c",
            "on close"
        );
        assert_eq!(
            select(text, pos(0, 1), inner).unwrap(),
            "a, g(b), c",
            "on open"
        );
        assert_eq!(select("no brackets", pos(0, 3), inner), None);
    }

    #[test]
    fn bracketed_block_on_own_lines_is_linewise() {
        let inner = |b: &Buffer, p| bracketed(b, p, '{', '}', false);
        let b = Buffer::from_text("fn x() {\n    a;\n    b;\n}");
        assert_eq!(
            inner(&b, pos(1, 4)),
            Some(Range::Lines { first: 1, last: 2 })
        );
    }

    #[test]
    fn bracketed_across_lines_charwise() {
        let inner = |b: &Buffer, p| bracketed(b, p, '[', ']', false);
        assert_eq!(select("[1,\n2]", pos(1, 0), inner).unwrap(), "1,\n2");
    }
}
