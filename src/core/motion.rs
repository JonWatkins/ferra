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
pub(super) enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

pub(super) fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

pub(super) fn class_at(buffer: &Buffer, pos: Position) -> CharClass {
    buffer
        .char_at(pos)
        .map_or(CharClass::Whitespace, char_class)
}

fn is_empty_line(buffer: &Buffer, row: usize) -> bool {
    buffer.line_len(row) == 0
}

/// The next position, including each line's end-of-line slot.
pub(super) fn next(buffer: &Buffer, pos: Position) -> Option<Position> {
    if pos.col < buffer.line_len(pos.row) {
        Some(Position::new(pos.row, pos.col + 1))
    } else if pos.row + 1 < buffer.line_count() {
        Some(Position::new(pos.row + 1, 0))
    } else {
        None
    }
}

/// The previous position, including each line's end-of-line slot.
pub(super) fn prev(buffer: &Buffer, pos: Position) -> Option<Position> {
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

/// End of the word under the cursor (or of the next word, from whitespace).
/// Unlike `e`, a cursor already on the last character of a word stays put.
///
/// This gives `cw` its Vim behaviour: it changes to the end of the current
/// word rather than swallowing the whitespace after it like `dw` does.
pub fn current_word_end(buffer: &Buffer, pos: Position) -> Position {
    let class = class_at(buffer, pos);
    let at_word_end = next(buffer, pos).is_none_or(|n| class_at(buffer, n) != class);
    if class != CharClass::Whitespace && at_word_end {
        pos
    } else {
        word_end(buffer, pos)
    }
}

/// An `f`/`t`/`F`/`T` search for a character on the current line.
/// Remembered by the editor so `;` and `,` can repeat it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharSearch {
    pub ch: char,
    pub forward: bool,
    /// `t`/`T`: stop just before (or after) the character rather than on it.
    pub till: bool,
}

impl CharSearch {
    /// The same search in the other direction (for `,`).
    pub fn reversed(self) -> Self {
        Self {
            forward: !self.forward,
            ..self
        }
    }

    /// Finds the `count`th match on the cursor's line. `None` if there are
    /// fewer matches.
    ///
    /// When `repeating` (`;`, `,`) a till search skips a match right next to
    /// the cursor, so repeating `tx` moves on instead of staying put.
    pub fn target(
        self,
        buffer: &Buffer,
        pos: Position,
        count: usize,
        repeating: bool,
    ) -> Option<Position> {
        let line: Vec<char> = buffer.line(pos.row).chars().collect();
        let skip = usize::from(self.till && repeating);
        let col = if self.forward {
            let from = pos.col + 1 + skip;
            (from..line.len())
                .filter(|&c| line[c] == self.ch)
                .nth(count.max(1) - 1)?
        } else {
            let until = pos.col.checked_sub(skip)?;
            (0..until)
                .rev()
                .filter(|&c| line[c] == self.ch)
                .nth(count.max(1) - 1)?
        };
        let col = match (self.till, self.forward) {
            (false, _) => col,
            (true, true) => col - 1,
            (true, false) => col + 1,
        };
        Some(pos.with_col(col))
    }
}

/// `%`: the bracket matching the one under the cursor, or the first bracket
/// after it on the line.
pub fn matching_bracket(buffer: &Buffer, pos: Position) -> Option<Position> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    let (bracket_col, bracket) = buffer
        .line(pos.row)
        .chars()
        .enumerate()
        .skip(pos.col)
        .find(|(_, c)| PAIRS.iter().any(|&(o, c2)| *c == o || *c == c2))?;
    let at = pos.with_col(bracket_col);
    let &(open, close) = PAIRS.iter().find(|&&(o, c)| bracket == o || bracket == c)?;
    if bracket == open {
        super::text_object::find_close(buffer, at, open, close)
    } else {
        super::text_object::find_open(buffer, at, open, close)
    }
}

/// `}`: the next empty line after the current paragraph, or the end of the
/// buffer. Empty means truly empty, as in Vim; a line of spaces isn't a
/// paragraph boundary.
pub fn paragraph_forward(buffer: &Buffer, pos: Position, count: usize) -> Position {
    let last = buffer.line_count() - 1;
    let empty = |row: usize| buffer.line_len(row) == 0;
    let mut row = pos.row;
    for _ in 0..count.max(1) {
        while row < last && empty(row) {
            row += 1;
        }
        while row < last && !empty(row) {
            row += 1;
        }
    }
    if empty(row) {
        Position::new(row, 0)
    } else {
        buffer.line_end(row)
    }
}

/// `{`: the previous empty line before the current paragraph, or the start
/// of the buffer.
pub fn paragraph_backward(buffer: &Buffer, pos: Position, count: usize) -> Position {
    let empty = |row: usize| buffer.line_len(row) == 0;
    let mut row = pos.row;
    for _ in 0..count.max(1) {
        while row > 0 && empty(row) {
            row -= 1;
        }
        while row > 0 && !empty(row) {
            row -= 1;
        }
    }
    Position::new(row, 0)
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
    fn current_word_end_stays_on_last_char() {
        let b = Buffer::from_text("ab cd");
        assert_eq!(current_word_end(&b, pos(0, 0)), pos(0, 1));
        assert_eq!(current_word_end(&b, pos(0, 1)), pos(0, 1));
        assert_eq!(current_word_end(&b, pos(0, 2)), pos(0, 4));
    }

    fn search(ch: char, forward: bool, till: bool) -> CharSearch {
        CharSearch { ch, forward, till }
    }

    #[test]
    fn find_char_forward_and_backward() {
        let b = Buffer::from_text("a.b.c.d");
        assert_eq!(
            search('.', true, false).target(&b, pos(0, 0), 1, false),
            Some(pos(0, 1))
        );
        assert_eq!(
            search('.', true, false).target(&b, pos(0, 0), 3, false),
            Some(pos(0, 5))
        );
        assert_eq!(
            search('.', false, false).target(&b, pos(0, 6), 2, false),
            Some(pos(0, 3))
        );
        assert_eq!(
            search('.', true, false).target(&b, pos(0, 0), 4, false),
            None
        );
        assert_eq!(
            search('x', true, false).target(&b, pos(0, 0), 1, false),
            None
        );
    }

    #[test]
    fn find_does_not_match_under_cursor() {
        let b = Buffer::from_text("aXa");
        assert_eq!(
            search('a', true, false).target(&b, pos(0, 0), 1, false),
            Some(pos(0, 2))
        );
        assert_eq!(
            search('a', false, false).target(&b, pos(0, 2), 1, false),
            Some(pos(0, 0))
        );
    }

    #[test]
    fn till_stops_before_the_char() {
        let b = Buffer::from_text("call(x)");
        assert_eq!(
            search('(', true, true).target(&b, pos(0, 0), 1, false),
            Some(pos(0, 3))
        );
        assert_eq!(
            search('c', false, true).target(&b, pos(0, 6), 1, false),
            Some(pos(0, 1))
        );
    }

    #[test]
    fn repeated_till_skips_adjacent_match() {
        let b = Buffer::from_text("a,b,c");
        let t = search(',', true, true);
        // Already just before the first comma: plain `t,` stays put...
        assert_eq!(t.target(&b, pos(0, 0), 1, false), Some(pos(0, 0)));
        // ...but `;` moves on to before the next one.
        assert_eq!(t.target(&b, pos(0, 0), 1, true), Some(pos(0, 2)));
        let back = t.reversed();
        assert_eq!(back.target(&b, pos(0, 4), 1, true), Some(pos(0, 2)));
    }

    #[test]
    fn find_stays_on_the_line() {
        let b = Buffer::from_text("ab\nb");
        assert_eq!(
            search('b', true, false).target(&b, pos(0, 1), 1, false),
            None
        );
    }

    #[test]
    fn matching_bracket_both_ways_and_nested() {
        let b = Buffer::from_text("f(a[0], {b})");
        assert_eq!(matching_bracket(&b, pos(0, 1)), Some(pos(0, 11)));
        assert_eq!(matching_bracket(&b, pos(0, 11)), Some(pos(0, 1)));
        assert_eq!(matching_bracket(&b, pos(0, 3)), Some(pos(0, 5)));
        assert_eq!(matching_bracket(&b, pos(0, 8)), Some(pos(0, 10)));
    }

    #[test]
    fn matching_bracket_searches_forward_on_line_and_across_lines() {
        let b = Buffer::from_text("if x {\n  y\n}");
        assert_eq!(matching_bracket(&b, pos(0, 0)), Some(pos(2, 0)));
        assert_eq!(matching_bracket(&b, pos(1, 0)), None);
    }

    #[test]
    fn paragraph_motions() {
        let b = Buffer::from_text("a\nb\n\n\nc\nd\n\ne");
        assert_eq!(paragraph_forward(&b, pos(0, 0), 1), pos(2, 0));
        assert_eq!(
            paragraph_forward(&b, pos(2, 0), 1),
            pos(6, 0),
            "skips the blank run"
        );
        assert_eq!(paragraph_forward(&b, pos(0, 0), 2), pos(6, 0));
        assert_eq!(
            paragraph_forward(&b, pos(7, 0), 1),
            pos(7, 1),
            "end of buffer"
        );
        assert_eq!(paragraph_backward(&b, pos(5, 0), 1), pos(3, 0));
        assert_eq!(paragraph_backward(&b, pos(3, 0), 1), pos(0, 0));
        assert_eq!(paragraph_backward(&b, pos(7, 0), 2), pos(3, 0));
    }

    #[test]
    fn whitespace_lines_are_not_paragraph_boundaries() {
        let b = Buffer::from_text("a\n  \nb\n\nc");
        assert_eq!(paragraph_forward(&b, pos(0, 0), 1), pos(3, 0));
    }

    #[test]
    fn first_non_blank_skips_indent() {
        let b = Buffer::from_text("    x\n   ");
        assert_eq!(first_non_blank(&b, 0), 4);
        assert_eq!(first_non_blank(&b, 1), 0);
    }
}
