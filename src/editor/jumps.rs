//! The jump list (`<C-o>`, `<C-i>`) and the change list (`g;`, `g,`).
//!
//! A "jump" is a big move: `gg`, `G`, `%`, searches, marks, `:42`,
//! switching buffers. Before each one the position being left is recorded,
//! so `<C-o>` can go back to it, across buffers. `''` goes to the place
//! before the last jump.

use super::{DocumentId, Editor};
use crate::core::Position;

const LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jump {
    pub document: DocumentId,
    pub position: Position,
}

impl Jump {
    /// Jumps on the same line of the same buffer count as one.
    fn same_line(&self, other: &Jump) -> bool {
        self.document == other.document && self.position.row == other.position.row
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JumpList {
    entries: Vec<Jump>,
    /// Where `<C-o>`/`<C-i>` are in the list; `entries.len()` when not
    /// browsing.
    index: usize,
}

impl JumpList {
    /// Records `jump` as the newest entry. Jumping while browsing the list
    /// drops the entries after the current one, like browser history.
    pub fn push(&mut self, jump: Jump) {
        self.entries.truncate(self.index);
        self.entries.retain(|e| !e.same_line(&jump));
        self.entries.push(jump);
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
        self.index = self.entries.len();
    }

    /// `<C-o>`: the entry before the current one. `current` is where the
    /// cursor is, remembered on the first step back so `<C-i>` can return.
    pub fn back(&mut self, current: Jump) -> Option<Jump> {
        if self.index == self.entries.len() {
            self.entries.retain(|e| !e.same_line(&current));
            self.entries.push(current);
            self.index = self.entries.len() - 1;
        }
        self.index = self.index.checked_sub(1)?;
        Some(self.entries[self.index])
    }

    /// `<C-i>`: the entry after the current one.
    pub fn forward(&mut self) -> Option<Jump> {
        if self.index + 1 >= self.entries.len() {
            return None;
        }
        self.index += 1;
        Some(self.entries[self.index])
    }

    /// Drops entries in a buffer that's been closed.
    pub fn forget(&mut self, document: DocumentId) {
        let before = self.entries.len();
        let at = self.index;
        let removed_before_index = self.entries[..at.min(before)]
            .iter()
            .filter(|e| e.document == document)
            .count();
        self.entries.retain(|e| e.document != document);
        self.index = at
            .saturating_sub(removed_before_index)
            .min(self.entries.len());
    }
}

impl Editor {
    fn here(&self) -> Jump {
        Jump {
            document: self.document.id(),
            position: self.document.cursor(),
        }
    }

    /// Records the cursor position before a jump, for `<C-o>` and `''`.
    pub fn push_jump(&mut self) {
        let here = self.here();
        self.jumps.push(here);
        self.last_jump_origin = Some(here);
    }

    /// Where the cursor was before the last jump (`''`).
    pub fn last_jump_origin(&self) -> Option<Jump> {
        self.last_jump_origin
    }

    /// `<C-o>`: back `count` entries. Returns `false` at the start.
    pub fn jump_back(&mut self, count: usize) -> bool {
        let mut target = None;
        for _ in 0..count.max(1) {
            let here = target.unwrap_or_else(|| self.here());
            match self.next_open_jump(|list| list.back(here)) {
                Some(jump) => target = Some(jump),
                None => break,
            }
        }
        self.go_to_jump(target)
    }

    /// `<C-i>`: forward `count` entries. Returns `false` at the end.
    pub fn jump_forward(&mut self, count: usize) -> bool {
        let mut target = None;
        for _ in 0..count.max(1) {
            match self.next_open_jump(JumpList::forward) {
                Some(jump) => target = Some(jump),
                None => break,
            }
        }
        self.go_to_jump(target)
    }

    /// Steps through the list, skipping entries in buffers since closed.
    fn next_open_jump(
        &mut self,
        mut step: impl FnMut(&mut JumpList) -> Option<Jump>,
    ) -> Option<Jump> {
        loop {
            let jump = step(&mut self.jumps)?;
            if self.document_by_id(jump.document).is_some() {
                return Some(jump);
            }
            self.jumps.forget(jump.document);
        }
    }

    fn go_to_jump(&mut self, jump: Option<Jump>) -> bool {
        let Some(jump) = jump else {
            return false;
        };
        self.switch_to(jump.document);
        self.document.set_cursor(jump.position);
        self.enforce_cursor_rules();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn jump(doc: u64, row: usize) -> Jump {
        Jump {
            document: DocumentId(doc),
            position: Position::new(row, 0),
        }
    }

    #[test]
    fn back_and_forward_like_browser_history() {
        let mut list = JumpList::default();
        list.push(jump(0, 1));
        list.push(jump(0, 5));
        assert_eq!(list.back(jump(0, 9)), Some(jump(0, 5)));
        assert_eq!(list.back(jump(0, 9)), Some(jump(0, 1)));
        assert_eq!(list.back(jump(0, 9)), None, "at the start");
        assert_eq!(list.forward(), Some(jump(0, 5)));
        assert_eq!(
            list.forward(),
            Some(jump(0, 9)),
            "back to where browsing began"
        );
        assert_eq!(list.forward(), None);
    }

    #[test]
    fn jumping_while_browsing_drops_the_forward_entries() {
        let mut list = JumpList::default();
        list.push(jump(0, 1));
        list.push(jump(0, 2));
        list.back(jump(0, 3));
        list.back(jump(0, 3));
        list.push(jump(0, 7));
        assert_eq!(list.forward(), None);
        assert_eq!(list.back(jump(0, 8)), Some(jump(0, 7)));
    }

    #[test]
    fn same_line_entries_are_merged() {
        let mut list = JumpList::default();
        list.push(jump(0, 1));
        list.push(jump(0, 4));
        list.push(jump(0, 1));
        assert_eq!(list.back(jump(0, 9)), Some(jump(0, 1)));
        assert_eq!(list.back(jump(0, 9)), Some(jump(0, 4)));
        assert_eq!(list.back(jump(0, 9)), None);
    }

    #[test]
    fn bounded_size() {
        let mut list = JumpList::default();
        for row in 0..150 {
            list.push(jump(0, row));
        }
        assert_eq!(list.entries.len(), LIMIT);
    }

    #[test]
    fn editor_jumps_across_buffers_and_skips_closed_ones() {
        let mut e = Editor::with_document(Document::with_text("a\nb\nc"));
        e.document.set_cursor(Position::new(2, 0));
        e.push_jump();
        let mut other = Document::with_text("other");
        other.set_id(e.new_document_id());
        e.activate(other);

        assert!(e.jump_back(1));
        assert_eq!(e.document.buffer().text(), "a\nb\nc");
        assert_eq!(e.document.cursor(), Position::new(2, 0));
        assert!(e.jump_forward(1));
        assert_eq!(e.document.buffer().text(), "other");

        // Close the first buffer: its entries are skipped.
        e.switch_to(DocumentId(0));
        e.close_buffer();
        assert!(!e.jump_back(1));
    }
}
