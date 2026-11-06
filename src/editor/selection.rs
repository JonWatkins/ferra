//! Visual-mode selections.
//!
//! A selection is an anchor (where `v`/`V` was pressed) plus the cursor.
//! It converts to the same [`Range`] that motions and text objects produce,
//! so operators work on it unchanged.

use super::{Editor, Mode};
use crate::core::{Buffer, Position, Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// `v`: characters from anchor to cursor, inclusive.
    Char,
    /// `V`: whole lines from anchor to cursor.
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub kind: SelectionKind,
}

impl Selection {
    /// The text covered when the cursor is at `cursor`.
    pub fn range(&self, buffer: &Buffer, cursor: Position) -> Range {
        let (start, end) = if self.anchor <= cursor {
            (self.anchor, cursor)
        } else {
            (cursor, self.anchor)
        };
        match self.kind {
            SelectionKind::Line => Range::Lines {
                first: start.row,
                last: end.row,
            },
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

/// A finished selection, remembered for `gv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LastSelection {
    selection: Selection,
    cursor: Position,
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
                self.selection = Some(Selection { anchor, kind });
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
        self.set_mode(Mode::Normal);
        Some(range)
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
        self.selection = Some(Selection { anchor, kind });
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
        });
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
        };
        assert_eq!(s.range(&b, pos(1, 0)), chars(pos(1, 0), pos(2, 0)));
    }

    #[test]
    fn char_range_on_last_empty_line_stops_at_end() {
        let b = Buffer::from_text("a\n");
        let s = Selection {
            anchor: pos(1, 0),
            kind: SelectionKind::Char,
        };
        assert_eq!(s.range(&b, pos(1, 0)), chars(pos(1, 0), pos(1, 0)));
    }

    #[test]
    fn line_range_covers_rows() {
        let b = Buffer::from_text("a\nb\nc");
        let s = Selection {
            anchor: pos(2, 0),
            kind: SelectionKind::Line,
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
