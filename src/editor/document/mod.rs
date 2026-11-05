mod file;

pub use file::{OpenInfo, SaveInfo};

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::core::{Buffer, Change, History, Position, Revision, TextFormat};

/// Identifies a document among the editor's open buffers. Shown to users
/// starting from 1.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(pub u64);

impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0 + 1)
    }
}

/// A buffer, the cursor editing it, its undo history, and the file it
/// belongs to (if any).
///
/// All text changes go through [`insert`](Self::insert) and
/// [`delete`](Self::delete), which record them in the history. Higher-level
/// behaviour (counts, modes, Vim semantics) lives in the commands. Opening
/// and saving live in the `file` submodule.
#[derive(Debug, Default, Clone)]
pub struct Document {
    id: DocumentId,
    buffer: Buffer,
    cursor: Position,
    /// Column to aim for when moving vertically through shorter lines.
    sticky_col: Option<usize>,
    history: History,
    path: Option<PathBuf>,
    format: TextFormat,
    /// The history revision that matches the file on disk.
    saved_revision: Revision,
    /// The file's modified time when last read or written, to detect
    /// changes made by other programs.
    disk_modified: Option<SystemTime>,
    /// Counts edits made through `insert`/`delete` (not undo/redo), so the
    /// app can tell whether a command sequence changed the text.
    edit_count: u64,
}

impl Document {
    pub fn with_text(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            ..Self::default()
        }
    }

    /// Display name for the UI: the path as given, or `[scratch]`.
    pub fn name(&self) -> Cow<'_, str> {
        match &self.path {
            Some(path) => path.to_string_lossy(),
            None => Cow::Borrowed("[scratch]"),
        }
    }

    /// Whether there are changes not yet saved. Undoing back to the saved
    /// state counts as unmodified again.
    pub fn is_modified(&self) -> bool {
        self.history.has_pending() || self.history.revision() != self.saved_revision
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    /// Gives the document its identity among the editor's buffers.
    pub(in crate::editor) fn set_id(&mut self, id: DocumentId) {
        self.id = id;
    }

    /// An unnamed, unmodified, empty document: nothing would be lost by
    /// replacing it.
    pub fn is_pristine_scratch(&self) -> bool {
        self.path.is_none()
            && !self.is_modified()
            && self.buffer.line_count() == 1
            && self.buffer.line_len(0) == 0
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    /// Increases with every edit; undo and redo don't count as edits.
    pub fn edit_count(&self) -> u64 {
        self.edit_count
    }

    // --- Cursor -----------------------------------------------------------

    /// Moves the cursor, clamping it into the buffer. Allows the end-of-line slot.
    pub fn set_cursor(&mut self, pos: Position) {
        let row = pos.row.min(self.buffer.line_count() - 1);
        let col = pos.col.min(self.buffer.line_len(row));
        self.cursor = Position::new(row, col);
        self.sticky_col = None;
    }

    /// Moves the cursor `delta` lines, keeping the column where possible.
    pub fn move_vertical(&mut self, delta: isize) {
        let max_row = self.buffer.line_count() - 1;
        let row = self.cursor.row.saturating_add_signed(delta).min(max_row);
        let target = *self.sticky_col.get_or_insert(self.cursor.col);
        self.cursor = Position::new(row, target.min(self.buffer.line_len(row)));
    }

    /// Keeps the cursor on a character, as required in normal mode.
    pub fn clamp_to_last_char(&mut self) {
        let max = self.buffer.line_len(self.cursor.row).saturating_sub(1);
        self.cursor.col = self.cursor.col.min(max);
    }

    // --- Recorded edits ---------------------------------------------------

    /// Inserts `text` at `at`, returning the position just after it.
    /// Doesn't move the cursor.
    pub fn insert(&mut self, at: Position, text: &str) -> Position {
        if text.is_empty() {
            return at;
        }
        let end = self.buffer.insert_text(at, text);
        self.edit_count += 1;
        self.history.record(Change::Insert {
            at,
            text: text.to_string(),
        });
        end
    }

    /// Deletes from `start` (inclusive) to `end` (exclusive), returning the
    /// removed text. Doesn't move the cursor.
    pub fn delete(&mut self, start: Position, end: Position) -> String {
        if start == end {
            return String::new();
        }
        let text = self.buffer.delete_range(start, end);
        self.edit_count += 1;
        self.history.record(Change::Delete {
            at: start,
            text: text.clone(),
        });
        text
    }

    /// Deletes whole lines `first..=last` (including their line breaks) and
    /// returns their text. Deleting every line leaves one empty line.
    pub fn delete_lines(&mut self, first: usize, last: usize) -> String {
        let text = self.buffer.lines_text(first, last);
        let (start, end) = if last + 1 < self.buffer.line_count() {
            (Position::new(first, 0), Position::new(last + 1, 0))
        } else if first > 0 {
            (self.buffer.line_end(first - 1), self.buffer.line_end(last))
        } else {
            (Position::new(0, 0), self.buffer.line_end(last))
        };
        self.delete(start, end);
        text
    }

    // --- Insert-mode conveniences (operate at the cursor) -----------------

    pub fn insert_char(&mut self, c: char) {
        self.insert_str(c.encode_utf8(&mut [0; 4]));
    }

    /// Inserts `s` at the cursor and moves the cursor past it.
    pub fn insert_str(&mut self, s: &str) {
        let end = self.insert(self.cursor, s);
        self.set_cursor(end);
    }

    /// Deletes the char before the cursor, joining lines at the line start.
    pub fn delete_backward(&mut self) {
        let Position { row, col } = self.cursor;
        let start = if col > 0 {
            Position::new(row, col - 1)
        } else if row > 0 {
            self.buffer.line_end(row - 1)
        } else {
            return;
        };
        self.delete(start, self.cursor);
        self.set_cursor(start);
    }

    /// Deletes the char under the cursor, joining lines at the line end.
    pub fn delete_forward(&mut self) {
        let Position { row, col } = self.cursor;
        let end = if col < self.buffer.line_len(row) {
            Position::new(row, col + 1)
        } else if row + 1 < self.buffer.line_count() {
            Position::new(row + 1, 0)
        } else {
            return;
        };
        self.delete(self.cursor, end);
    }

    /// Inserts an empty line below (or above) the cursor and moves onto it.
    pub fn open_line(&mut self, below: bool) {
        let row = self.cursor.row;
        if below {
            let end = self.insert(self.buffer.line_end(row), "\n");
            self.set_cursor(end);
        } else {
            self.insert(Position::new(row, 0), "\n");
            self.set_cursor(Position::new(row, 0));
        }
    }

    // --- History ----------------------------------------------------------

    /// Ends the current undo step.
    pub fn commit(&mut self) {
        self.history.commit();
    }

    /// Undoes one step. Returns `false` if there was nothing to undo.
    pub fn undo(&mut self) -> bool {
        self.history
            .undo(&mut self.buffer)
            .map(|pos| self.set_cursor(pos))
            .is_some()
    }

    /// Redoes one step. Returns `false` if there was nothing to redo.
    pub fn redo(&mut self) -> bool {
        self.history
            .redo(&mut self.buffer)
            .map(|pos| self.set_cursor(pos))
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn set_cursor_clamps_into_buffer() {
        let mut doc = Document::with_text("ab\nc");
        doc.set_cursor(pos(9, 9));
        assert_eq!(doc.cursor(), pos(1, 1));
    }

    #[test]
    fn vertical_movement_remembers_column() {
        let mut doc = Document::with_text("long line\nab\nlong line");
        doc.set_cursor(pos(0, 6));
        doc.move_vertical(1);
        assert_eq!(doc.cursor(), pos(1, 2));
        doc.move_vertical(1);
        assert_eq!(doc.cursor(), pos(2, 6));
    }

    #[test]
    fn vertical_movement_clamps_rows() {
        let mut doc = Document::with_text("a\nb");
        doc.move_vertical(-5);
        assert_eq!(doc.cursor().row, 0);
        doc.move_vertical(5);
        assert_eq!(doc.cursor().row, 1);
    }

    #[test]
    fn clamp_to_last_char_moves_off_line_end() {
        let mut doc = Document::with_text("abc");
        doc.set_cursor(pos(0, 3));
        doc.clamp_to_last_char();
        assert_eq!(doc.cursor().col, 2);
    }

    #[test]
    fn insert_str_advances_cursor_by_chars_and_lines() {
        let mut doc = Document::default();
        doc.insert_str("éé");
        assert_eq!(doc.cursor(), pos(0, 2));
        doc.insert_str("\nx");
        assert_eq!(doc.cursor(), pos(1, 1));
    }

    #[test]
    fn delete_backward_joins_lines() {
        let mut doc = Document::with_text("ab\ncd");
        doc.set_cursor(pos(1, 0));
        doc.delete_backward();
        assert_eq!(doc.buffer().text(), "abcd");
        assert_eq!(doc.cursor(), pos(0, 2));
    }

    #[test]
    fn delete_backward_at_start_is_noop() {
        let mut doc = Document::with_text("ab");
        doc.delete_backward();
        assert_eq!(doc.buffer().text(), "ab");
    }

    #[test]
    fn delete_forward_joins_lines_at_end() {
        let mut doc = Document::with_text("ab\ncd");
        doc.set_cursor(pos(0, 2));
        doc.delete_forward();
        assert_eq!(doc.buffer().text(), "abcd");
    }

    #[test]
    fn delete_lines_in_middle_at_end_and_all() {
        let mut doc = Document::with_text("a\nb\nc\nd");
        assert_eq!(doc.delete_lines(1, 2), "b\nc");
        assert_eq!(doc.buffer().text(), "a\nd");
        assert_eq!(doc.delete_lines(1, 1), "d");
        assert_eq!(doc.buffer().text(), "a");
        assert_eq!(doc.delete_lines(0, 0), "a");
        assert_eq!(doc.buffer().text(), "");
    }

    #[test]
    fn open_line_above_and_below() {
        let mut doc = Document::with_text("x");
        doc.open_line(true);
        assert_eq!(doc.cursor(), pos(1, 0));
        doc.open_line(false);
        assert_eq!(doc.buffer().text(), "x\n\n");
        assert_eq!(doc.cursor(), pos(1, 0));
    }

    #[test]
    fn edits_are_undoable_as_one_step_until_commit() {
        let mut doc = Document::with_text("abc");
        doc.set_cursor(pos(0, 3));
        doc.insert_str("de");
        doc.insert_str("\nf");
        doc.commit();
        doc.delete_lines(0, 0);
        doc.commit();

        assert!(doc.undo());
        assert_eq!(doc.buffer().text(), "abcde\nf");
        assert!(doc.undo());
        assert_eq!(doc.buffer().text(), "abc");
        assert_eq!(doc.cursor(), pos(0, 3));
        assert!(!doc.undo());

        assert!(doc.redo());
        assert_eq!(doc.buffer().text(), "abcde\nf");
    }

    #[test]
    fn modified_until_undone_back_to_saved_state() {
        let mut doc = Document::with_text("abc");
        assert!(!doc.is_modified());
        doc.insert_str("x");
        assert!(doc.is_modified(), "pending changes count");
        doc.commit();
        assert!(doc.is_modified());
        doc.undo();
        assert!(!doc.is_modified());
        doc.redo();
        assert!(doc.is_modified());
    }

    #[test]
    fn scratch_document_name() {
        assert_eq!(Document::default().name(), "[scratch]");
    }

    #[test]
    fn edit_count_ignores_undo_and_empty_edits() {
        let mut doc = Document::with_text("abc");
        doc.insert(Position::new(0, 0), "");
        assert_eq!(doc.edit_count(), 0);
        doc.insert_str("x");
        doc.delete_backward();
        assert_eq!(doc.edit_count(), 2);
        doc.undo();
        doc.redo();
        assert_eq!(doc.edit_count(), 2);
    }

    #[test]
    fn empty_edits_are_not_recorded() {
        let mut doc = Document::with_text("abc");
        doc.insert(pos(0, 0), "");
        doc.delete(pos(0, 1), pos(0, 1));
        assert!(!doc.undo());
    }
}
