//! Visual-mode selections.
//!
//! A selection is an anchor (where `v`/`V` was pressed) plus the cursor.
//! It converts to the same [`Range`] that motions and text objects produce,
//! so operators work on it unchanged.

use super::{Editor, Mode};
use crate::core::columns::{char_col_at, display_col};
use crate::core::{Buffer, Position, Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// `v`: characters from anchor to cursor, inclusive.
    Char,
    /// `V`: whole lines from anchor to cursor.
    Line,
    /// `<C-v>`: a rectangle with the anchor and cursor at opposite corners
    /// (see [`Editor::selection_block`]).
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub kind: SelectionKind,
    /// VS Code's kind (`keymap = "vscode"`): up to the cursor, not
    /// including the character there.
    pub exclusive: bool,
}

impl Selection {
    /// The text covered when the cursor is at `cursor`. A block gives its
    /// whole lines, for commands that act on lines (`J`, `:`, `X`).
    pub fn range(&self, buffer: &Buffer, cursor: Position) -> Range {
        let (start, end) = if self.anchor <= cursor {
            (self.anchor, cursor)
        } else {
            (cursor, self.anchor)
        };
        match self.kind {
            SelectionKind::Line | SelectionKind::Block => Range::Lines {
                first: start.row,
                last: end.row,
            },
            SelectionKind::Char if self.exclusive => Range::Chars { start, end },
            SelectionKind::Char => {
                // Inclusive of the character under `end`. On an empty line (or
                // past the last character) that "character" is the line break.
                let end = if end.col < buffer.line_len(end.row) {
                    end.with_col(end.col + 1)
                } else if end.row + 1 < buffer.line_count() {
                    Position::new(end.row + 1, 0)
                } else {
                    buffer.line_end(end.row)
                };
                Range::Chars { start, end }
            }
        }
    }
}

/// How much text a selection covered, for `.` to cover the same amount from
/// wherever the cursor is (Vim's `visual-repeat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualShape {
    /// Characters within one line: this many.
    Chars(usize),
    /// Characters over several lines: this many lines, ending at this
    /// column on the last.
    CharLines { lines: usize, end_col: usize },
    /// This many whole lines.
    Lines(usize),
    /// This many rows, this many display columns wide (or to each row's
    /// end).
    Block {
        rows: usize,
        width: usize,
        to_eol: bool,
    },
}

/// A finished selection, remembered for `gv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LastSelection {
    selection: Selection,
    cursor: Position,
    /// A block that ran to each row's end (`$`).
    to_eol: bool,
}

impl Editor {
    /// `v` / `V`: starts a selection, switches its kind, or (pressing the
    /// same key again) leaves visual mode.
    pub fn toggle_visual(&mut self, kind: SelectionKind) {
        match self.selection {
            Some(selection) if self.mode() == Mode::Visual => {
                if selection.kind == kind {
                    self.set_mode(Mode::Normal);
                } else {
                    self.selection = Some(Selection { kind, ..selection });
                }
            }
            _ => {
                let anchor = self.document.cursor();
                self.set_mode(Mode::Visual);
                let exclusive = self.modeless();
                self.selection = Some(Selection {
                    anchor,
                    kind,
                    exclusive,
                });
                self.block_to_eol = false;
            }
        }
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    /// The selected range, if in visual mode.
    pub fn selection_range(&self) -> Option<Range> {
        let selection = self.selection?;
        Some(selection.range(self.document.buffer(), self.document.cursor()))
    }

    /// Takes the selected range and returns to normal mode, ready for an
    /// operator to act on it.
    pub fn take_selection(&mut self) -> Option<Range> {
        let range = self.selection_range()?;
        self.note_taken_shape();
        self.set_mode(Mode::Normal);
        Some(range)
    }

    /// The shape of the current selection.
    pub fn visual_shape(&self) -> Option<VisualShape> {
        let selection = self.selection?;
        let cursor = self.document.cursor();
        let (start, end) = if selection.anchor <= cursor {
            (selection.anchor, cursor)
        } else {
            (cursor, selection.anchor)
        };
        let lines = end.row - start.row + 1;
        Some(match selection.kind {
            SelectionKind::Line => VisualShape::Lines(lines),
            SelectionKind::Char if lines == 1 => VisualShape::Chars(end.col - start.col + 1),
            SelectionKind::Char => VisualShape::CharLines {
                lines,
                end_col: end.col,
            },
            SelectionKind::Block => {
                let block = crate::core::Block::between(
                    self.document.buffer(),
                    (selection.anchor.row, selection.anchor.col),
                    (cursor.row, cursor.col),
                    self.block_to_eol,
                    self.options.tab_width,
                );
                VisualShape::Block {
                    rows: lines,
                    width: block.right - block.left + 1,
                    to_eol: block.to_eol,
                }
            }
        })
    }

    /// Remembers the selection's shape as an operator takes it.
    pub(super) fn note_taken_shape(&mut self) {
        self.taken_shape = self.visual_shape();
    }

    /// The shape of the selection the last operator took, once: the app
    /// keeps it with the change, so `.` acts on as much text again.
    pub fn take_visual_shape(&mut self) -> Option<VisualShape> {
        self.taken_shape.take()
    }

    /// Selects text of `shape` starting at the cursor, in visual mode (for
    /// `.` after a visual operator). It's cut short at the end of a line or
    /// the buffer.
    pub fn select_shape(&mut self, shape: VisualShape) {
        let start = self.document.cursor();
        let mut anchor = start;
        let buffer = self.document.buffer();
        let last_row = |lines: usize| (start.row + lines.max(1) - 1).min(buffer.line_count() - 1);
        let last_char = |row: usize, col: usize| col.min(buffer.line_len(row).saturating_sub(1));
        let (kind, cursor, to_eol) = match shape {
            VisualShape::Chars(n) => (
                SelectionKind::Char,
                start.with_col(last_char(start.row, start.col + n.max(1) - 1)),
                false,
            ),
            VisualShape::CharLines { lines, end_col } => {
                let row = last_row(lines);
                (
                    SelectionKind::Char,
                    Position::new(row, last_char(row, end_col)),
                    false,
                )
            }
            VisualShape::Lines(lines) => {
                let row = last_row(lines);
                (
                    SelectionKind::Line,
                    Position::new(row, last_char(row, start.col)),
                    false,
                )
            }
            VisualShape::Block {
                rows,
                width,
                to_eol,
            } => {
                // Corners at the top left and bottom right, unless the
                // bottom row ends inside the block and the top one doesn't:
                // then the top right and bottom left, so the block keeps
                // its width.
                let tab = self.options.tab_width;
                let row = last_row(rows);
                let left = display_col(&buffer.line(start.row), start.col, tab);
                let right = left + width.max(1) - 1;
                let width_of = |row: usize| display_col(&buffer.line(row), usize::MAX, tab);
                let col_at = |row: usize, col: usize| {
                    last_char(row, char_col_at(&buffer.line(row), col, tab))
                };
                let short = width_of(row) <= right;
                if short && width_of(row) > left && width_of(start.row) > right {
                    anchor = Position::new(row, col_at(row, left));
                    (
                        SelectionKind::Block,
                        Position::new(start.row, col_at(start.row, right)),
                        to_eol,
                    )
                } else {
                    (
                        SelectionKind::Block,
                        Position::new(row, col_at(row, right)),
                        to_eol,
                    )
                }
            }
        };
        self.set_mode(Mode::Visual);
        self.selection = Some(Selection {
            anchor,
            kind,
            exclusive: self.modeless(),
        });
        self.block_to_eol = to_eol;
        self.document.set_cursor(cursor);
    }

    /// `o`: moves the cursor to the other end of the selection.
    pub fn swap_selection_ends(&mut self) {
        if let Some(selection) = &mut self.selection {
            let cursor = self.document.cursor();
            self.document.set_cursor(selection.anchor);
            selection.anchor = cursor;
        }
    }

    /// Selects `range` (from a text object), replacing the current selection.
    pub fn select_range(&mut self, range: Range) {
        let buffer = self.document.buffer();
        let (anchor, cursor, kind) = match range {
            Range::Chars { start, end } if start == end => return,
            // VS Code's selections end before the cursor.
            Range::Chars { start, end } if self.modeless() => (start, end, SelectionKind::Char),
            Range::Chars { start, end } => {
                // `end` is exclusive; the cursor sits on the last selected char.
                let last = if end.col > 0 {
                    end.with_col(end.col - 1)
                } else {
                    buffer.line_end(end.row - 1)
                };
                (start, last, SelectionKind::Char)
            }
            Range::Lines { first, last } => (
                Position::new(first, 0),
                buffer.line_end(last),
                SelectionKind::Line,
            ),
        };
        self.selection = Some(Selection {
            anchor,
            kind,
            exclusive: self.modeless(),
        });
        self.document.set_cursor(cursor);
    }

    /// `gv`: restores the last selection. Returns `false` if there was none.
    pub fn reselect(&mut self) -> bool {
        let Some(last) = self.last_selection else {
            return false;
        };
        let buffer = self.document.buffer();
        let clamp = |p: Position| {
            let row = p.row.min(buffer.line_count() - 1);
            Position::new(row, p.col.min(buffer.line_len(row)))
        };
        let anchor = clamp(last.selection.anchor);
        self.set_mode(Mode::Visual);
        self.selection = Some(Selection {
            anchor,
            kind: last.selection.kind,
            exclusive: last.selection.exclusive,
        });
        self.block_to_eol = last.to_eol;
        self.document.set_cursor(last.cursor);
        self.enforce_cursor_rules();
        true
    }

    /// First and last rows of the last visual selection (`'<` and `'>`).
    pub fn last_visual_rows(&self) -> Option<(usize, usize)> {
        let last = self.last_selection?;
        let (a, b) = (last.selection.anchor.row, last.cursor.row);
        Some((a.min(b), a.max(b)))
    }

    /// Called by `set_mode` when leaving visual mode.
    pub(super) fn end_selection(&mut self) {
        if let Some(selection) = self.selection.take() {
            self.last_selection = Some(LastSelection {
                selection,
                cursor: self.document.cursor(),
                to_eol: selection.kind == SelectionKind::Block && self.block_to_eol,
            });
        }
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

    #[test]
    fn char_range_is_inclusive_in_either_direction() {
        let b = Buffer::from_text("hello");
        let s = Selection {
            anchor: pos(0, 1),
            kind: SelectionKind::Char,
            exclusive: false,
        };
        assert_eq!(s.range(&b, pos(0, 3)), chars(pos(0, 1), pos(0, 4)));
        assert_eq!(s.range(&b, pos(0, 0)), chars(pos(0, 0), pos(0, 2)));
    }

    #[test]
    fn char_range_on_empty_line_includes_line_break() {
        let b = Buffer::from_text("a\n\nb");
        let s = Selection {
            anchor: pos(1, 0),
            kind: SelectionKind::Char,
            exclusive: false,
        };
        assert_eq!(s.range(&b, pos(1, 0)), chars(pos(1, 0), pos(2, 0)));
    }

    #[test]
    fn char_range_on_last_empty_line_stops_at_end() {
        let b = Buffer::from_text("a\n");
        let s = Selection {
            anchor: pos(1, 0),
            kind: SelectionKind::Char,
            exclusive: false,
        };
        assert_eq!(s.range(&b, pos(1, 0)), chars(pos(1, 0), pos(1, 0)));
    }

    #[test]
    fn line_range_covers_rows() {
        let b = Buffer::from_text("a\nb\nc");
        let s = Selection {
            anchor: pos(2, 0),
            kind: SelectionKind::Line,
            exclusive: false,
        };
        assert_eq!(s.range(&b, pos(1, 0)), Range::Lines { first: 1, last: 2 });
    }

    #[test]
    fn toggle_enters_switches_and_leaves() {
        let mut e = editor("abc", pos(0, 1));
        e.toggle_visual(SelectionKind::Char);
        assert_eq!(e.mode(), Mode::Visual);
        assert_eq!(e.selection().unwrap().anchor, pos(0, 1));

        e.toggle_visual(SelectionKind::Line);
        assert_eq!(e.selection().unwrap().kind, SelectionKind::Line);
        assert_eq!(e.selection().unwrap().anchor, pos(0, 1), "anchor kept");

        e.toggle_visual(SelectionKind::Line);
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn take_selection_returns_range_and_leaves_visual() {
        let mut e = editor("abc", pos(0, 0));
        e.toggle_visual(SelectionKind::Char);
        e.document.set_cursor(pos(0, 1));
        assert_eq!(e.take_selection(), Some(chars(pos(0, 0), pos(0, 2))));
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.take_selection(), None);
    }

    #[test]
    fn swap_ends() {
        let mut e = editor("abcdef", pos(0, 1));
        e.toggle_visual(SelectionKind::Char);
        e.document.set_cursor(pos(0, 4));
        e.swap_selection_ends();
        assert_eq!(e.document.cursor(), pos(0, 1));
        assert_eq!(e.selection().unwrap().anchor, pos(0, 4));
    }

    #[test]
    fn select_range_from_text_object() {
        let mut e = editor("foo bar", pos(0, 5));
        e.toggle_visual(SelectionKind::Char);
        e.select_range(chars(pos(0, 4), pos(0, 7)));
        assert_eq!(e.selection_range(), Some(chars(pos(0, 4), pos(0, 7))));
        assert_eq!(e.document.cursor(), pos(0, 6));
    }

    #[test]
    fn select_lines_switches_to_line_selection() {
        let mut e = editor("a\nb\nc", pos(1, 0));
        e.toggle_visual(SelectionKind::Char);
        e.select_range(Range::Lines { first: 0, last: 1 });
        assert_eq!(e.selection().unwrap().kind, SelectionKind::Line);
        assert_eq!(
            e.selection_range(),
            Some(Range::Lines { first: 0, last: 1 })
        );
    }

    #[test]
    fn reselect_restores_last_selection() {
        let mut e = editor("hello world", pos(0, 0));
        assert!(!e.reselect());
        e.toggle_visual(SelectionKind::Char);
        e.document.set_cursor(pos(0, 4));
        e.set_mode(Mode::Normal);
        e.document.set_cursor(pos(0, 9));

        assert!(e.reselect());
        assert_eq!(e.mode(), Mode::Visual);
        assert_eq!(e.selection_range(), Some(chars(pos(0, 0), pos(0, 5))));
    }

    #[test]
    fn shapes_are_measured_and_reselected_from_the_cursor() {
        let mut e = editor("abcdef\nabcdef\nabcdef\nabcdef", pos(0, 1));
        let measure = |e: &mut Editor, kind, to: Position| {
            e.toggle_visual(kind);
            e.document.set_cursor(to);
            let shape = e.visual_shape().unwrap();
            e.set_mode(Mode::Normal);
            e.document.set_cursor(pos(0, 1));
            shape
        };
        let char_shape = measure(&mut e, SelectionKind::Char, pos(0, 3));
        assert_eq!(char_shape, VisualShape::Chars(3));
        let lines_shape = measure(&mut e, SelectionKind::Char, pos(1, 4));
        assert_eq!(
            lines_shape,
            VisualShape::CharLines {
                lines: 2,
                end_col: 4
            }
        );
        assert_eq!(
            measure(&mut e, SelectionKind::Line, pos(2, 0)),
            VisualShape::Lines(3)
        );
        let block_shape = measure(&mut e, SelectionKind::Block, pos(1, 2));
        assert_eq!(
            block_shape,
            VisualShape::Block {
                rows: 2,
                width: 2,
                to_eol: false
            }
        );

        e.document.set_cursor(pos(2, 2));
        e.select_shape(char_shape);
        assert_eq!(e.selection_range(), Some(chars(pos(2, 2), pos(2, 5))));
        e.set_mode(Mode::Normal);
        e.document.set_cursor(pos(2, 0));
        e.select_shape(lines_shape);
        assert_eq!(e.selection_range(), Some(chars(pos(2, 0), pos(3, 5))));
        e.set_mode(Mode::Normal);
        e.document.set_cursor(pos(3, 3));
        e.select_shape(block_shape);
        let block = e.selection_block().unwrap();
        assert_eq!(
            (block.first, block.last, block.left, block.right),
            (3, 3, 3, 4),
            "cut short by the buffer's end"
        );
    }

    #[test]
    fn reselect_clamps_to_shrunk_buffer() {
        let mut e = editor("a\nb\nc", pos(0, 0));
        e.toggle_visual(SelectionKind::Line);
        e.document.set_cursor(pos(2, 0));
        e.set_mode(Mode::Normal);
        // As a real delete would, which also moves the cursor.
        e.document.delete_lines(1, 2);
        e.document.set_cursor(pos(0, 0));

        assert!(e.reselect());
        assert_eq!(
            e.selection_range(),
            Some(Range::Lines { first: 0, last: 0 })
        );
    }
}
