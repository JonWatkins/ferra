//! Pure cursor motions over a [`Buffer`].
//!
//! Each function takes a buffer and a starting position and returns the
//! target position. They never mutate anything, which makes them trivial to
//! test and to reuse later for operators (e.g. `dw`) and selections.
//!
//! The end of each line is treated as a whitespace "newline" slot, so word
//! motions flow naturally across lines. Empty lines act as word boundaries,
//! like in Vim.

use super::{Buffer, Position};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

fn class_at(buffer: &Buffer, pos: Position) -> CharClass {
    buffer
        .char_at(pos)
        .map_or(CharClass::Whitespace, char_class)
}

fn is_empty_line(buffer: &Buffer, row: usize) -> bool {
    buffer.line_len(row) == 0
}

/// The next position, including each line's end-of-line slot.
fn next(buffer: &Buffer, pos: Position) -> Option<Position> {
    if pos.col < buffer.line_len(pos.row) {
        Some(Position::new(pos.row, pos.col + 1))
    } else if pos.row + 1 < buffer.line_count() {
        Some(Position::new(pos.row + 1, 0))
    } else {
        None
    }
}

/// The previous position, including each line's end-of-line slot.
fn prev(buffer: &Buffer, pos: Position) -> Option<Position> {
    if pos.col > 0 {
        Some(Position::new(pos.row, pos.col - 1))
    } else if pos.row > 0 {
        Some(Position::new(pos.row - 1, buffer.line_len(pos.row - 1)))
    } else {
        None
    }
}

/// `w`: start of the next word.
pub fn word_forward(buffer: &Buffer, pos: Position) -> Position {
    let mut p = pos;
    let start = class_at(buffer, p);

    if start != CharClass::Whitespace {
        while let Some(n) = next(buffer, p) {
            p = n;
            if class_at(buffer, p) != start {
                break;
            }
        }
    }

    loop {
        if class_at(buffer, p) != CharClass::Whitespace {
            return p;
        }
        if p != pos && p.col == 0 && is_empty_line(buffer, p.row) {
            return p;
        }
        match next(buffer, p) {
            Some(n) => p = n,
            None => return p,
        }
    }
}

/// `b`: start of the current or previous word.
pub fn word_backward(buffer: &Buffer, pos: Position) -> Position {
    let Some(mut p) = prev(buffer, pos) else {
        return pos;
    };

    while class_at(buffer, p) == CharClass::Whitespace {
        if is_empty_line(buffer, p.row) {
            return p;
        }
        match prev(buffer, p) {
            Some(n) => p = n,
            None => return p,
        }
    }

    let class = class_at(buffer, p);
    while let Some(n) = prev(buffer, p) {
        if class_at(buffer, n) != class {
            break;
        }
        p = n;
    }
    p
}

/// `e`: end of the current or next word.
pub fn word_end(buffer: &Buffer, pos: Position) -> Position {
    let Some(mut p) = next(buffer, pos) else {
        return pos;
    };

    while class_at(buffer, p) == CharClass::Whitespace {
        match next(buffer, p) {
            Some(n) => p = n,
            None => return p,
        }
    }

    let class = class_at(buffer, p);
    while let Some(n) = next(buffer, p) {
        if class_at(buffer, n) != class {
            break;
        }
        p = n;
    }
    p
}

/// `^`: column of the first non-whitespace character on `row`.
pub fn first_non_blank(buffer: &Buffer, row: usize) -> usize {
    buffer
        .line(row)
        .chars()
        .position(|c| !c.is_whitespace())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn word_forward_skips_to_next_word() {
        let b = Buffer::from_text("hello world");
        assert_eq!(word_forward(&b, pos(0, 0)), pos(0, 6));
        assert_eq!(word_forward(&b, pos(0, 2)), pos(0, 6));
    }

    #[test]
    fn word_forward_stops_at_punctuation() {
        let b = Buffer::from_text("foo.bar");
        assert_eq!(word_forward(&b, pos(0, 0)), pos(0, 3));
        assert_eq!(word_forward(&b, pos(0, 3)), pos(0, 4));
    }

    #[test]
    fn word_forward_crosses_lines() {
        let b = Buffer::from_text("foo\n  bar");
        assert_eq!(word_forward(&b, pos(0, 0)), pos(1, 2));
    }

    #[test]
    fn word_forward_stops_on_empty_line() {
        let b = Buffer::from_text("foo\n\nbar");
        assert_eq!(word_forward(&b, pos(0, 0)), pos(1, 0));
        assert_eq!(word_forward(&b, pos(1, 0)), pos(2, 0));
    }

    #[test]
    fn word_forward_at_end_of_buffer_goes_to_end() {
        let b = Buffer::from_text("foo");
        assert_eq!(word_forward(&b, pos(0, 0)), pos(0, 3));
    }

    #[test]
    fn word_backward_moves_to_word_start() {
        let b = Buffer::from_text("hello world");
        assert_eq!(word_backward(&b, pos(0, 8)), pos(0, 6));
        assert_eq!(word_backward(&b, pos(0, 6)), pos(0, 0));
    }

    #[test]
    fn word_backward_crosses_lines() {
        let b = Buffer::from_text("foo\nbar");
        assert_eq!(word_backward(&b, pos(1, 0)), pos(0, 0));
    }

    #[test]
    fn word_backward_at_start_is_noop() {
        let b = Buffer::from_text("foo");
        assert_eq!(word_backward(&b, pos(0, 0)), pos(0, 0));
    }

    #[test]
    fn word_end_moves_to_end_of_word() {
        let b = Buffer::from_text("hello world");
        assert_eq!(word_end(&b, pos(0, 0)), pos(0, 4));
        assert_eq!(word_end(&b, pos(0, 4)), pos(0, 10));
    }

    #[test]
    fn word_end_handles_punctuation() {
        let b = Buffer::from_text("a.b");
        assert_eq!(word_end(&b, pos(0, 0)), pos(0, 1));
    }

    #[test]
    fn first_non_blank_skips_indent() {
        let b = Buffer::from_text("    x\n   ");
        assert_eq!(first_non_blank(&b, 0), 4);
        assert_eq!(first_non_blank(&b, 1), 0);
    }
}
