mod file;
mod folds;
mod marks;
mod virtual_text;

pub use virtual_text::{DisplayLine, RangeHighlight, VirtualText};

pub use file::{DiskState, OpenInfo, SaveInfo};

use std::borrow::Cow;
use std::path::PathBuf;

use crate::core::{Buffer, Change, History, IndentUnit, Position, Revision, TextFormat};

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
    /// The encoding the file on disk is in; a different one in `format`
    /// (from `:fileencoding`) is an unsaved change.
    saved_encoding: crate::core::FileEncoding,
    /// The file's modified time when last read or written, to detect
    /// changes made by other programs.
    disk_stamp: Option<crate::files::FileStamp>,
    /// How many times it has been saved to its own file (language servers
    /// are told of each).
    saves: u64,
    /// The file's modified time (or `None`: deleted) when a change on disk
    /// was last reported, so it's reported once.
    disk_acknowledged: Option<Option<crate::files::FileStamp>>,
    /// Counts edits made through `insert`/`delete` (not undo/redo), so the
    /// app can tell whether a command sequence changed the text.
    edit_count: u64,
    /// Local marks (`ma`), kept in place as the text changes.
    marks: marks::Marks,
    /// Where recent changes were, oldest first (`g;`, `g,`, `'.`).
    changes: Vec<Position>,
    /// Which change `g;`/`g,` is at; `None` until they're used after an edit.
    change_index: Option<usize>,
    /// Rows being followed through edits (`:g` visits them); `None` once
    /// a row's line is deleted.
    tracked_rows: Option<Vec<Option<usize>>>,
    /// A language server's diagnostics, moved along as the text is edited
    /// until it sends new ones.
    diagnostics: Vec<crate::editor::diagnostics::Diagnostic>,
    /// Text drawn in lines that isn't in the file (inlay hints), moved
    /// along as the text is edited.
    virtual_text: Vec<VirtualText>,
    /// Ranges plugins have highlighted, moved along as the text is edited.
    highlights: Vec<RangeHighlight>,
    /// How the file was indented when read (see [`crate::core::indent`]).
    detected_indent: Option<IndentUnit>,
    /// The indentation chosen with `:indent`, over any detected one.
    chosen_indent: Option<IndentUnit>,
    /// Closed folds (`zc`), kept in step with edits.
    folds: folds::Folds,
}

impl Document {
    /// How many times it has been saved to its own file.
    pub fn saves(&self) -> u64 {
        self.saves
    }

    pub fn with_text(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            ..Self::default()
        }
    }

    /// How the file was indented when it was read, if that could be told.
    pub fn detected_indent(&self) -> Option<IndentUnit> {
        self.detected_indent
    }

    /// The indentation picked with `:indent`, if any.
    pub fn chosen_indent(&self) -> Option<IndentUnit> {
        self.chosen_indent
    }

    /// `:indent`: indents this buffer with `unit` from now on, or `None` to
    /// go back to the detected or configured style.
    pub fn choose_indent(&mut self, unit: Option<IndentUnit>) {
        self.chosen_indent = unit;
    }

    /// Looks at the text again to see how it's indented.
    pub fn detect_indent(&mut self) {
        self.detected_indent = crate::core::indent::detect(&self.buffer);
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
        self.history.has_pending()
            || self.history.revision() != self.saved_revision
            || self.format.encoding != self.saved_encoding
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

    /// Remembers where a change happened. Changes on the same line as the
    /// last one (typing a word, say) update it rather than adding another.
    fn record_change(&mut self, at: Position) {
        match self.changes.last_mut() {
            Some(last) if last.row == at.row => *last = at,
            _ => {
                self.changes.push(at);
                if self.changes.len() > 100 {
                    self.changes.remove(0);
                }
            }
        }
        self.change_index = None;
    }

    /// Where the last change was (`'.`).
    pub fn last_change(&self) -> Option<Position> {
        self.changes.last().map(|&p| self.clamp(p))
    }

    /// `g;` (`older`) / `g,`: the next change position in that direction,
    /// clamped into the buffer. `None` at either end of the list.
    pub fn step_change(&mut self, older: bool, count: usize) -> Option<Position> {
        let last = self.changes.len().checked_sub(1)?;
        let index = match (self.change_index, older) {
            // The first `g;` goes to the newest change itself.
            (None, true) => last.checked_sub(count.max(1) - 1)?,
            (None, false) => return None,
            (Some(i), true) => i.checked_sub(count.max(1))?,
            (Some(i), false) => i.checked_add(count.max(1)).filter(|&i| i <= last)?,
        };
        self.change_index = Some(index);
        Some(self.clamp(self.changes[index]))
    }

    fn clamp(&self, position: Position) -> Position {
        let row = position.row.min(self.buffer.line_count() - 1);
        Position::new(row, position.col.min(self.buffer.line_len(row)))
    }

    /// Sets mark `name` at the cursor.
    pub fn set_mark(&mut self, name: char) {
        self.marks.set(name, self.cursor);
    }

    /// Where mark `name` is, clamped into the buffer (undo can move text
    /// without moving marks).
    pub fn mark(&self, name: char) -> Option<Position> {
        let mark = self.marks.get(name)?;
        let row = mark.row.min(self.buffer.line_count() - 1);
        Some(Position::new(row, mark.col.min(self.buffer.line_len(row))))
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
        self.marks.after_insert(at, end);
        self.track_insert(at, end);
        self.diagnostics_after_insert(at, end);
        self.record_change(at);
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
        self.marks.after_delete(start, end);
        self.track_delete(start, end);
        self.diagnostics_after_delete(start, end);
        self.record_change(start);
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

    // --- Diagnostics -------------------------------------------------------

    /// A language server's diagnostics for the text, in order.
    pub fn diagnostics(&self) -> &[crate::editor::diagnostics::Diagnostic] {
        &self.diagnostics
    }

    /// Replaces the diagnostics (sorted here).
    pub fn set_diagnostics(
        &mut self,
        mut diagnostics: Vec<crate::editor::diagnostics::Diagnostic>,
    ) {
        diagnostics.sort_by_key(|d| (d.start, d.severity));
        self.diagnostics = diagnostics;
    }

    /// Text went in at `at`, ending at `end`: what's after it moves along.
    /// A diagnostic's start moves with text put in right at it (it's still
    /// on the same word), its end doesn't (typing after a word doesn't
    /// stretch its underline).
    fn diagnostics_after_insert(&mut self, at: Position, end: Position) {
        self.virtual_text_after_insert(at, end);
        let shift = |p: Position| {
            if p.row == at.row {
                Position::new(end.row, end.col + (p.col - at.col))
            } else {
                Position::new(p.row + (end.row - at.row), p.col)
            }
        };
        for d in &mut self.diagnostics {
            if d.start >= at {
                d.start = shift(d.start);
            }
            if d.end > at {
                d.end = shift(d.end);
            }
            d.end = d.end.max(d.start);
        }
    }

    /// `start..end` was deleted: what's after it moves back, and what was
    /// in it goes to `start`.
    fn diagnostics_after_delete(&mut self, start: Position, end: Position) {
        self.virtual_text_after_delete(start, end);
        let shift = |p: Position| {
            if p <= start {
                p
            } else if p < end {
                start
            } else if p.row == end.row {
                Position::new(start.row, start.col + (p.col - end.col))
            } else {
                Position::new(p.row - (end.row - start.row), p.col)
            }
        };
        for d in &mut self.diagnostics {
            d.start = shift(d.start);
            d.end = shift(d.end);
        }
    }

    /// Follows changes made other than by `insert` and `delete` (undo).
    fn diagnostics_after_changes(&mut self, changes: &[Change]) {
        for change in changes {
            match change {
                Change::Insert { at, text } => {
                    self.diagnostics_after_insert(*at, crate::core::end_of(*at, text));
                }
                Change::Delete { at, text } => {
                    self.diagnostics_after_delete(*at, crate::core::end_of(*at, text));
                }
            }
        }
    }

    // --- Tracked rows -----------------------------------------------------

    /// Starts following `rows` through edits: they shift as lines are
    /// added or removed above them, and are forgotten when their line is
    /// deleted. Used by `:g`, which runs a command on each matching line.
    pub fn track_rows(&mut self, rows: Vec<usize>) {
        self.tracked_rows = Some(rows.into_iter().map(Some).collect());
    }

    /// Where tracked row `index` is now, or `None` if its line was deleted.
    pub fn tracked_row(&self, index: usize) -> Option<usize> {
        self.tracked_rows.as_ref()?.get(index).copied().flatten()
    }

    pub fn is_tracking_rows(&self) -> bool {
        self.tracked_rows.is_some()
    }

    pub fn stop_tracking_rows(&mut self) {
        self.tracked_rows = None;
    }

    fn track_insert(&mut self, at: Position, end: Position) {
        let added = end.row - at.row;
        if added == 0 {
            return;
        }
        for row in self.tracked_rows.iter_mut().flatten().flatten() {
            if *row > at.row {
                *row += added;
            }
        }
    }

    fn track_delete(&mut self, start: Position, end: Position) {
        let removed = end.row - start.row;
        if removed == 0 {
            return;
        }
        // Deleting from a line start to a line start removes whole lines;
        // otherwise the end row's remainder joins the start row, so it's the
        // later rows that go.
        let gone = if start.col == 0 && end.col == 0 {
            start.row..end.row
        } else {
            start.row + 1..end.row + 1
        };
        for slot in self.tracked_rows.iter_mut().flatten() {
            match *slot {
                Some(row) if gone.contains(&row) => *slot = None,
                Some(row) if row > start.row => *slot = Some(row - removed),
                _ => {}
            }
        }
    }

    // --- History ----------------------------------------------------------

    /// Ends the current undo step.
    pub fn commit(&mut self) {
        self.history.commit();
    }

    /// Undoes one step. Returns `false` if there was nothing to undo.
    pub fn undo(&mut self) -> bool {
        let mut made = Vec::new();
        let undone = self.history.undo_noting(&mut self.buffer, &mut made);
        self.diagnostics_after_changes(&made);
        if let Some(pos) = undone {
            self.set_cursor(pos);
        }
        undone.is_some()
    }

    /// Redoes one step. Returns `false` if there was nothing to redo.
    pub fn redo(&mut self) -> bool {
        let mut made = Vec::new();
        let redone = self.history.redo_noting(&mut self.buffer, &mut made);
        self.diagnostics_after_changes(&made);
        if let Some(pos) = redone {
            self.set_cursor(pos);
        }
        redone.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn diagnostic(
        start: (usize, usize),
        end: (usize, usize),
    ) -> crate::editor::diagnostics::Diagnostic {
        crate::editor::diagnostics::Diagnostic {
            start: pos(start.0, start.1),
            end: pos(end.0, end.1),
            severity: crate::editor::diagnostics::Severity::Error,
            message: "x".into(),
            source: None,
        }
    }

    fn spans(doc: &Document) -> Vec<((usize, usize), (usize, usize))> {
        doc.diagnostics()
            .iter()
            .map(|d| ((d.start.row, d.start.col), (d.end.row, d.end.col)))
            .collect()
    }

    #[test]
    fn diagnostics_follow_their_text_through_edits() {
        // `bad` on the second line.
        let mut doc = Document::with_text("one\nlet bad = 1;\nthree");
        doc.set_diagnostics(vec![diagnostic((1, 4), (1, 7))]);
        doc.insert(pos(1, 0), "    ");
        assert_eq!(spans(&doc), [((1, 8), (1, 11))], "along the line");
        doc.insert(pos(0, 0), "zero\n");
        assert_eq!(spans(&doc), [((2, 8), (2, 11))], "down a line");
        doc.insert(pos(2, 11), "ly");
        assert_eq!(
            spans(&doc),
            [((2, 8), (2, 11))],
            "typing after it doesn't stretch it"
        );
        doc.insert(pos(2, 8), "very");
        assert_eq!(
            spans(&doc),
            [((2, 12), (2, 15))],
            "typing right before it moves it"
        );
        doc.delete(pos(0, 0), pos(2, 4));
        assert_eq!(spans(&doc), [((0, 8), (0, 11))], "lines above deleted");
        doc.delete(pos(0, 9), pos(0, 15));
        assert_eq!(
            spans(&doc),
            [((0, 8), (0, 9))],
            "partly deleted: what's left"
        );
        doc.delete(pos(0, 0), pos(0, 12));
        assert_eq!(spans(&doc), [((0, 0), (0, 0))], "all deleted: where it was");
    }

    #[test]
    fn diagnostics_follow_undo_and_redo() {
        let mut doc = Document::with_text("a\nbad");
        doc.set_diagnostics(vec![diagnostic((1, 0), (1, 3))]);
        doc.insert(pos(0, 0), "x\ny\n");
        doc.commit();
        assert_eq!(spans(&doc), [((3, 0), (3, 3))]);
        doc.undo();
        assert_eq!(spans(&doc), [((1, 0), (1, 3))]);
        doc.redo();
        assert_eq!(spans(&doc), [((3, 0), (3, 3))]);
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
    fn marks_follow_edits() {
        let mut doc = Document::with_text("a\nb\nc");
        doc.set_cursor(Position::new(2, 0));
        doc.set_mark('m');
        doc.set_cursor(Position::new(0, 0));
        doc.insert_str("new\n");
        assert_eq!(doc.mark('m'), Some(Position::new(3, 0)));
        doc.delete_lines(0, 1);
        assert_eq!(doc.mark('m'), Some(Position::new(1, 0)));
        assert_eq!(doc.mark('x'), None);
    }

    #[test]
    fn change_list_records_and_steps() {
        let mut doc = Document::with_text("a\nb\nc\nd");
        assert_eq!(doc.step_change(true, 1), None, "no changes yet");
        for row in [0, 2, 3] {
            doc.set_cursor(Position::new(row, 0));
            doc.insert_str("x");
        }
        doc.insert_str("y");
        assert_eq!(
            doc.last_change(),
            Some(Position::new(3, 1)),
            "same line: updated"
        );
        assert_eq!(doc.step_change(true, 1), Some(Position::new(3, 1)));
        assert_eq!(doc.step_change(true, 1), Some(Position::new(2, 0)));
        assert_eq!(doc.step_change(true, 5), None, "past the oldest");
        assert_eq!(doc.step_change(false, 1), Some(Position::new(3, 1)));
        assert_eq!(doc.step_change(false, 1), None, "past the newest");
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

    #[test]
    fn tracked_rows_follow_edits_and_forget_deleted_lines() {
        let mut doc = Document::with_text("0\n1\n2\n3\n4");
        doc.track_rows(vec![1, 2, 4]);
        doc.delete_lines(2, 2);
        assert_eq!(
            (doc.tracked_row(0), doc.tracked_row(1), doc.tracked_row(2)),
            (Some(1), None, Some(3))
        );
        doc.insert(Position::new(0, 1), "\nnew");
        assert_eq!((doc.tracked_row(0), doc.tracked_row(2)), (Some(2), Some(4)));
        // Deleting the last line deletes from the previous line's end.
        doc.delete_lines(4, 4);
        assert_eq!(doc.tracked_row(2), None);
        // Joining lines: the joined line goes.
        doc.track_rows(vec![0, 1]);
        doc.delete(Position::new(0, 1), Position::new(1, 0));
        assert_eq!((doc.tracked_row(0), doc.tracked_row(1)), (Some(0), None));
        doc.stop_tracking_rows();
        assert!(!doc.is_tracking_rows());
    }
}
