//! Operators (`d`, `c`, `y`, `>`, `<`, `gu`, `gU`, `g~`) and applying them to
//! ranges.

use super::registers::{Register, StoreKind};
use super::transform::CaseChange;
use super::{Editor, Mode};
use crate::core::{Position, Range, motion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lowercase,
    Uppercase,
    ToggleCase,
}

/// An operator waiting for a motion or text object (operator-pending mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingOperator {
    pub operator: Operator,
    /// Count typed before the operator (the `2` in `2d3w`).
    pub count: Option<usize>,
    pub register: Option<char>,
}

impl Editor {
    /// Enters operator-pending mode for `operator`.
    pub fn begin_operator(
        &mut self,
        operator: Operator,
        count: Option<usize>,
        register: Option<char>,
    ) {
        self.set_mode(Mode::OperatorPending);
        self.pending_operator = Some(PendingOperator {
            operator,
            count,
            register,
        });
    }

    /// Takes the pending operator (if any) and returns to normal mode.
    pub fn take_pending_operator(&mut self) -> Option<PendingOperator> {
        let pending = self.pending_operator.take();
        if self.mode() == Mode::OperatorPending {
            self.set_mode(Mode::Normal);
        }
        pending
    }

    /// Applies `operator` to `range`, storing the affected text in `register`.
    pub fn apply_operator(
        &mut self,
        operator: Operator,
        range: Range,
        register: Option<char>,
    ) -> Result<(), String> {
        self.apply_operator_over(operator, range, register, false)
    }

    /// [`Editor::apply_operator`] for a range from a motion. A delete over a
    /// jump motion (`d%`, `d}`) is kept in `"1` even within one line.
    pub fn apply_operator_over(
        &mut self,
        operator: Operator,
        range: Range,
        register: Option<char>,
        jump: bool,
    ) -> Result<(), String> {
        // These transform the text in place and don't touch registers.
        match operator {
            Operator::Indent | Operator::Outdent => {
                self.shift_lines(range, operator == Operator::Indent);
                return Ok(());
            }
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                let change = match operator {
                    Operator::Lowercase => CaseChange::Lower,
                    Operator::Uppercase => CaseChange::Upper,
                    _ => CaseChange::Toggle,
                };
                self.change_case(range, change);
                return Ok(());
            }
            Operator::Delete | Operator::Change | Operator::Yank => {}
        }

        let buffer = self.document.buffer();
        let value = match range {
            Range::Chars { start, end } => Register::charwise(buffer.text_range(start, end)),
            Range::Lines { first, last } => Register::linewise(buffer.lines_text(first, last)),
        };

        // Like Vim, an empty change (e.g. `x` on an empty line) leaves the
        // registers alone, but `c` still enters insert mode.
        if !range.is_empty() {
            let kind = match operator {
                Operator::Yank => StoreKind::Yank,
                _ if jump => StoreKind::DeleteOverJump,
                _ => StoreKind::Delete,
            };
            self.registers.store(register, value, kind)?;
        }

        let doc = &mut self.document;
        match (operator, range) {
            (Operator::Yank, Range::Chars { start, .. }) => doc.set_cursor(start),
            (Operator::Yank, Range::Lines { first, .. }) => {
                let cursor = doc.cursor();
                if cursor.row > first {
                    doc.set_cursor(Position::new(first, cursor.col));
                }
            }
            (Operator::Delete, Range::Chars { start, end }) => {
                doc.delete(start, end);
                doc.set_cursor(start);
            }
            (Operator::Delete, Range::Lines { first, last }) => {
                doc.delete_lines(first, last);
                let row = first.min(doc.buffer().line_count() - 1);
                let col = motion::first_non_blank(doc.buffer(), row);
                doc.set_cursor(Position::new(row, col));
            }
            (Operator::Change, Range::Chars { start, end }) => {
                doc.delete(start, end);
                doc.set_cursor(start);
                self.set_mode(Mode::Insert);
            }
            (Operator::Change, Range::Lines { first, last }) => {
                // Keep one (now empty) line to type into.
                let end = doc.buffer().line_end(last);
                doc.delete(Position::new(first, 0), end);
                doc.set_cursor(Position::new(first, 0));
                self.set_mode(Mode::Insert);
            }
            (
                Operator::Indent
                | Operator::Outdent
                | Operator::Lowercase
                | Operator::Uppercase
                | Operator::ToggleCase,
                _,
            ) => unreachable!("handled before touching registers"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn editor(text: &str, cursor: Position) -> Editor {
        let mut editor = Editor::with_document(Document::with_text(text));
        editor.document.set_cursor(cursor);
        editor
    }

    fn chars(start: Position, end: Position) -> Range {
        Range::Chars { start, end }
    }

    fn unnamed(editor: &Editor) -> Option<Register> {
        editor.registers.get(None).cloned()
    }

    #[test]
    fn begin_and_take_pending_operator() {
        let mut e = editor("", pos(0, 0));
        e.begin_operator(Operator::Delete, Some(2), Some('a'));
        assert_eq!(e.mode(), Mode::OperatorPending);
        let pending = e.take_pending_operator().unwrap();
        assert_eq!(pending.count, Some(2));
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.take_pending_operator(), None);
    }

    #[test]
    fn leaving_operator_pending_mode_cancels_operator() {
        let mut e = editor("", pos(0, 0));
        e.begin_operator(Operator::Delete, None, None);
        e.set_mode(Mode::Normal);
        assert_eq!(e.take_pending_operator(), None);
    }

    #[test]
    fn delete_chars_stores_text() {
        let mut e = editor("hello world", pos(0, 0));
        e.apply_operator(Operator::Delete, chars(pos(0, 0), pos(0, 6)), None)
            .unwrap();
        assert_eq!(e.document.buffer().text(), "world");
        assert_eq!(unnamed(&e), Some(Register::charwise("hello ")));
    }

    #[test]
    fn delete_lines_moves_to_first_non_blank() {
        let mut e = editor("a\nb\n  c", pos(0, 0));
        e.apply_operator(Operator::Delete, Range::Lines { first: 0, last: 1 }, None)
            .unwrap();
        assert_eq!(e.document.buffer().text(), "  c");
        assert_eq!(e.document.cursor(), pos(0, 2));
        assert_eq!(unnamed(&e), Some(Register::linewise("a\nb")));
    }

    #[test]
    fn yank_does_not_modify_text() {
        let mut e = editor("abc", pos(0, 2));
        e.apply_operator(Operator::Yank, chars(pos(0, 0), pos(0, 2)), None)
            .unwrap();
        assert_eq!(e.document.buffer().text(), "abc");
        assert_eq!(e.document.cursor(), pos(0, 0));
        assert_eq!(e.registers.get(Some('0')), Some(&Register::charwise("ab")));
    }

    #[test]
    fn change_lines_keeps_an_empty_line_and_enters_insert() {
        let mut e = editor("a\nb\nc", pos(1, 0));
        e.apply_operator(Operator::Change, Range::Lines { first: 1, last: 1 }, None)
            .unwrap();
        assert_eq!(e.document.buffer().text(), "a\n\nc");
        assert_eq!(e.mode(), Mode::Insert);
        assert_eq!(e.document.cursor(), pos(1, 0));
    }

    #[test]
    fn empty_range_leaves_registers_alone() {
        let mut e = editor("", pos(0, 0));
        e.registers
            .store(None, Register::charwise("keep"), StoreKind::Yank)
            .unwrap();
        e.apply_operator(Operator::Delete, chars(pos(0, 0), pos(0, 0)), None)
            .unwrap();
        assert_eq!(unnamed(&e), Some(Register::charwise("keep")));
    }

    #[test]
    fn invalid_register_aborts_without_editing() {
        let mut e = editor("abc", pos(0, 0));
        let result = e.apply_operator(Operator::Delete, chars(pos(0, 0), pos(0, 1)), Some('%'));
        assert!(result.is_err());
        assert_eq!(e.document.buffer().text(), "abc");
    }
}
