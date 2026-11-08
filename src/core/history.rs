//! Undo/redo history built from invertible changes.
//!
//! Every edit is recorded as a [`Change`]. Changes are collected into a
//! pending transaction until [`History::commit`] is called; each committed
//! transaction is one undo step. The editor decides where the boundaries are
//! (in Vim terms: one per normal-mode command, one per insert session).

use super::{Buffer, Position};

/// A single, invertible edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Insert { at: Position, text: String },
    Delete { at: Position, text: String },
}

impl Change {
    /// The change that undoes this one.
    pub fn inverted(&self) -> Change {
        match self {
            Change::Insert { at, text } => Change::Delete {
                at: *at,
                text: text.clone(),
            },
            Change::Delete { at, text } => Change::Insert {
                at: *at,
                text: text.clone(),
            },
        }
    }

    pub fn apply(&self, buffer: &mut Buffer) {
        match self {
            Change::Insert { at, text } => {
                buffer.insert_text(*at, text);
            }
            Change::Delete { at, text } => {
                buffer.delete_range(*at, end_of(*at, text));
            }
        }
    }

    /// Where the change starts. Used to place the cursor after undo/redo.
    pub fn position(&self) -> Position {
        match self {
            Change::Insert { at, .. } | Change::Delete { at, .. } => *at,
        }
    }

    /// Tries to fold `next` into `self`, so that typing a word or holding
    /// backspace is stored as one change rather than one per character.
    fn merge(&mut self, next: &Change) -> bool {
        match (self, next) {
            (
                Change::Insert { at, text },
                Change::Insert {
                    at: next_at,
                    text: next_text,
                },
            ) if end_of(*at, text) == *next_at => {
                text.push_str(next_text);
                true
            }
            (
                Change::Delete { at, text },
                Change::Delete {
                    at: next_at,
                    text: next_text,
                },
            ) if end_of(*next_at, next_text) == *at => {
                text.insert_str(0, next_text);
                *at = *next_at;
                true
            }
            _ => false,
        }
    }
}

/// The position just after `text` if it were inserted at `at`.
pub fn end_of(at: Position, text: &str) -> Position {
    match text.rsplit_once('\n') {
        None => Position::new(at.row, at.col + text.chars().count()),
        Some((before, last)) => {
            let newlines = before.matches('\n').count() + 1;
            Position::new(at.row + newlines, last.chars().count())
        }
    }
}

/// A committed undo step. Its id names the buffer state it produces.
#[derive(Debug, Clone)]
struct Transaction {
    id: Revision,
    changes: Vec<Change>,
}

/// Identifies a buffer state in the history, so callers can tell whether
/// they're back at a given state (e.g. the last save) after undo/redo.
/// `0` is the original, unedited state.
pub type Revision = u64;

#[derive(Debug, Default, Clone)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    pending: Vec<Change>,
    last_id: Revision,
}

impl History {
    /// Records a change that has already been applied to the buffer.
    pub fn record(&mut self, change: Change) {
        self.redo.clear();
        if let Some(last) = self.pending.last_mut()
            && last.merge(&change)
        {
            return;
        }
        self.pending.push(change);
    }

    /// Closes the pending transaction, making it one undo step.
    pub fn commit(&mut self) {
        if !self.pending.is_empty() {
            self.last_id += 1;
            self.undo.push(Transaction {
                id: self.last_id,
                changes: std::mem::take(&mut self.pending),
            });
        }
    }

    /// Whether there are recorded changes not yet committed.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The revision of the last committed state (ignoring pending changes).
    pub fn revision(&self) -> Revision {
        self.undo.last().map_or(0, |t| t.id)
    }

    /// Undoes the last transaction. Returns where the cursor should go, or
    /// `None` if there was nothing to undo.
    pub fn undo(&mut self, buffer: &mut Buffer) -> Option<Position> {
        self.undo_noting(buffer, &mut Vec::new())
    }

    /// Like [`undo`](Self::undo), adding the changes made to `made`, in
    /// order, so what follows the text (diagnostics) can follow them too.
    pub fn undo_noting(&mut self, buffer: &mut Buffer, made: &mut Vec<Change>) -> Option<Position> {
        self.commit();
        let transaction = self.undo.pop()?;
        for change in transaction.changes.iter().rev() {
            let inverted = change.inverted();
            inverted.apply(buffer);
            made.push(inverted);
        }
        let cursor = transaction.changes.first().map(Change::position);
        self.redo.push(transaction);
        cursor
    }

    /// Redoes the last undone transaction.
    pub fn redo(&mut self, buffer: &mut Buffer) -> Option<Position> {
        self.redo_noting(buffer, &mut Vec::new())
    }

    /// Like [`redo`](Self::redo), adding the changes made to `made`.
    pub fn redo_noting(&mut self, buffer: &mut Buffer, made: &mut Vec<Change>) -> Option<Position> {
        self.commit();
        let transaction = self.redo.pop()?;
        for change in &transaction.changes {
            change.apply(buffer);
            made.push(change.clone());
        }
        let cursor = transaction.changes.first().map(Change::position);
        self.undo.push(transaction);
        cursor
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || !self.pending.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn insert(row: usize, col: usize, text: &str) -> Change {
        Change::Insert {
            at: pos(row, col),
            text: text.into(),
        }
    }

    fn delete(row: usize, col: usize, text: &str) -> Change {
        Change::Delete {
            at: pos(row, col),
            text: text.into(),
        }
    }

    /// Applies `change` to `buffer` and records it, as the document does.
    fn edit(buffer: &mut Buffer, history: &mut History, change: Change) {
        change.apply(buffer);
        history.record(change);
    }

    #[test]
    fn end_of_handles_multiline_text() {
        assert_eq!(end_of(pos(1, 2), "abc"), pos(1, 5));
        assert_eq!(end_of(pos(1, 2), "a\nbc"), pos(2, 2));
        assert_eq!(end_of(pos(0, 0), "\n\n"), pos(2, 0));
    }

    #[test]
    fn inverted_changes_restore_buffer() {
        let original = Buffer::from_text("hello\nworld");
        for change in [insert(0, 2, "X\nY"), delete(0, 3, "lo\nw")] {
            let mut buffer = original.clone();
            change.apply(&mut buffer);
            change.inverted().apply(&mut buffer);
            assert_eq!(buffer, original, "{change:?}");
        }
    }

    #[test]
    fn consecutive_inserts_merge() {
        let mut history = History::default();
        history.record(insert(0, 0, "a"));
        history.record(insert(0, 1, "b"));
        history.record(insert(0, 2, "\n"));
        history.record(insert(1, 0, "c"));
        assert_eq!(history.pending, vec![insert(0, 0, "ab\nc")]);
    }

    #[test]
    fn backspaces_merge() {
        let mut history = History::default();
        history.record(delete(0, 2, "c"));
        history.record(delete(0, 1, "b"));
        assert_eq!(history.pending, vec![delete(0, 1, "bc")]);
    }

    #[test]
    fn non_adjacent_changes_do_not_merge() {
        let mut history = History::default();
        history.record(insert(0, 0, "a"));
        history.record(insert(0, 5, "b"));
        assert_eq!(history.pending.len(), 2);
    }

    #[test]
    fn undo_and_redo_a_transaction() {
        let mut buffer = Buffer::from_text("abc");
        let mut history = History::default();
        edit(&mut buffer, &mut history, delete(0, 0, "a"));
        edit(&mut buffer, &mut history, insert(0, 2, "!"));
        history.commit();
        assert_eq!(buffer.text(), "bc!");

        assert_eq!(history.undo(&mut buffer), Some(pos(0, 0)));
        assert_eq!(buffer.text(), "abc");
        assert_eq!(history.redo(&mut buffer), Some(pos(0, 0)));
        assert_eq!(buffer.text(), "bc!");
    }

    #[test]
    fn each_commit_is_one_step() {
        let mut buffer = Buffer::default();
        let mut history = History::default();
        edit(&mut buffer, &mut history, insert(0, 0, "a"));
        history.commit();
        edit(&mut buffer, &mut history, insert(0, 1, "b"));
        history.commit();

        history.undo(&mut buffer);
        assert_eq!(buffer.text(), "a");
        history.undo(&mut buffer);
        assert_eq!(buffer.text(), "");
        assert_eq!(history.undo(&mut buffer), None);
    }

    #[test]
    fn undo_commits_pending_changes_first() {
        let mut buffer = Buffer::default();
        let mut history = History::default();
        edit(&mut buffer, &mut history, insert(0, 0, "a"));
        assert!(history.can_undo());
        history.undo(&mut buffer);
        assert_eq!(buffer.text(), "");
    }

    #[test]
    fn revisions_track_undo_and_redo() {
        let mut buffer = Buffer::default();
        let mut history = History::default();
        assert_eq!(history.revision(), 0);

        edit(&mut buffer, &mut history, insert(0, 0, "a"));
        assert!(history.has_pending());
        assert_eq!(
            history.revision(),
            0,
            "pending changes have no revision yet"
        );
        history.commit();
        let first = history.revision();
        edit(&mut buffer, &mut history, insert(0, 1, "b"));
        history.commit();
        let second = history.revision();
        assert_ne!(first, second);

        history.undo(&mut buffer);
        assert_eq!(history.revision(), first);
        history.redo(&mut buffer);
        assert_eq!(history.revision(), second);
        history.undo(&mut buffer);
        history.undo(&mut buffer);
        assert_eq!(history.revision(), 0);
    }

    #[test]
    fn diverging_edit_gets_a_new_revision() {
        let mut buffer = Buffer::default();
        let mut history = History::default();
        edit(&mut buffer, &mut history, insert(0, 0, "a"));
        history.commit();
        let undone = history.revision();
        history.undo(&mut buffer);
        edit(&mut buffer, &mut history, insert(0, 0, "b"));
        history.commit();
        assert_ne!(history.revision(), undone, "ids are never reused");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buffer = Buffer::default();
        let mut history = History::default();
        edit(&mut buffer, &mut history, insert(0, 0, "a"));
        history.undo(&mut buffer);
        assert!(history.can_redo());
        edit(&mut buffer, &mut history, insert(0, 0, "b"));
        assert!(!history.can_redo());
    }
}
