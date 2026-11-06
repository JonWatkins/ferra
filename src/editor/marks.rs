//! Marks at the editor level: `a`–`z` belong to the current buffer;
//! `A`–`Z` remember which buffer they're in, so they jump across buffers.
//!
//! A global mark's position is kept as a mark in its buffer (under the same
//! letter), so it follows edits just like a local one.

use super::{DocumentId, Editor};
use crate::core::Position;

impl Editor {
    /// `m{name}`: marks the cursor position.
    pub fn set_mark(&mut self, name: char) -> Result<(), String> {
        match name {
            'a'..='z' => self.document.set_mark(name),
            'A'..='Z' => {
                self.document.set_mark(name);
                self.global_marks.insert(name, self.document.id());
            }
            _ => return Err(format!("Invalid mark: {name} (use a-z or A-Z)")),
        }
        Ok(())
    }

    /// The buffer and position of mark `name`.
    pub fn mark_target(&self, name: char) -> Result<(DocumentId, Position), String> {
        let not_set = || format!("Mark not set: {name}");
        let document = match name {
            'a'..='z' => &self.document,
            'A'..='Z' => {
                let id = *self.global_marks.get(&name).ok_or_else(not_set)?;
                self.document_by_id(id).ok_or_else(not_set)?
            }
            _ => return Err(format!("Invalid mark: {name}")),
        };
        let position = document.mark(name).ok_or_else(not_set)?;
        Ok((document.id(), position))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    #[test]
    fn local_marks_are_per_buffer() {
        let mut e = Editor::with_document(Document::with_text("a\nb"));
        e.document.set_cursor(Position::new(1, 0));
        e.set_mark('q').unwrap();
        assert_eq!(e.mark_target('q').unwrap().1, Position::new(1, 0));
        assert_eq!(e.mark_target('z').unwrap_err(), "Mark not set: z");
        assert!(e.set_mark('1').unwrap_err().contains("Invalid mark"));
    }

    #[test]
    fn global_marks_remember_their_buffer() {
        let mut e = Editor::with_document(Document::with_text("first"));
        e.set_mark('F').unwrap();
        let first = e.document.id();
        let mut other = Document::with_text("second");
        other.set_id(e.new_document_id());
        e.activate(other);

        let (id, position) = e.mark_target('F').unwrap();
        assert_eq!((id, position), (first, Position::new(0, 0)));
        assert!(e.mark_target('f').is_err(), "lowercase marks are local");
    }
}
