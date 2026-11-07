//! Replace mode (`R`): typing over the text instead of inserting it.
//!
//! As in Vim it's insert mode with a difference, so insert-mode keys work
//! the same: typed chars take the place of the ones under the cursor
//! (adding at the end of a line), `<BS>` puts back what it overwrote (and
//! only moves left over text it didn't type), `<CR>` breaks the line
//! without deleting anything, and a count (`3Rab<Esc>`) types the text
//! that many times over.

use super::{Editor, Mode};
use crate::core::Position;

/// Replace mode's state, kept while in insert mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Replacing {
    /// Each char typed, where it went and what it took the place of
    /// (`None`: it was added at the end of a line), latest last.
    overwritten: Vec<(Position, Option<char>)>,
    /// How many times the text is typed (`3R`).
    count: usize,
}

impl Editor {
    /// `R`: starts replace mode at the cursor.
    pub fn start_replace(&mut self, count: usize) {
        self.set_mode(Mode::Insert);
        self.replacing = Some(Replacing {
            overwritten: Vec::new(),
            count: count.max(1),
        });
    }

    /// Whether typing replaces text (replace mode).
    pub fn replacing(&self) -> bool {
        self.replacing.is_some() && self.mode == Mode::Insert
    }

    /// Types `c` over the char under the cursor, or adds it at a line's end.
    pub(super) fn type_replacing(&mut self, c: char) {
        if c == '\n' {
            // A line break is added, not typed over anything.
            self.document.insert_str("\n");
            return;
        }
        let cursor = self.document.cursor();
        let original = self.document.buffer().char_at(cursor);
        if original.is_some() {
            self.document
                .delete(cursor, cursor.with_col(cursor.col + 1));
        }
        self.document.insert_char(c);
        if let Some(state) = &mut self.replacing {
            state.overwritten.push((cursor, original));
        }
    }

    /// `<BS>` in replace mode: puts back the char the last typed one took
    /// the place of, or just moves left over text typed before.
    pub fn backspace_replacing(&mut self) {
        let cursor = self.document.cursor();
        let Some(state) = &mut self.replacing else {
            return;
        };
        let typed_here = cursor.col > 0
            && state
                .overwritten
                .last()
                .is_some_and(|(at, _)| *at == cursor.with_col(cursor.col - 1));
        if !typed_here {
            if cursor.col > 0 {
                self.document.set_cursor(cursor.with_col(cursor.col - 1));
            }
            return;
        }
        let (at, original) = state.overwritten.pop().expect("checked");
        self.document.delete(at, cursor);
        if let Some(original) = original {
            self.document.insert(at, &original.to_string());
        }
        self.document.set_cursor(at);
        self.note_typed_backspace();
    }

    /// Leaving replace mode: types the text again for a count, then forgets
    /// the state. Called from `set_mode` before what was typed is stored.
    pub(super) fn finish_replace(&mut self) {
        let Some(state) = self.replacing.take() else {
            return;
        };
        let typed = self.typed.clone();
        if typed.is_empty() {
            return;
        }
        self.replacing = Some(Replacing {
            overwritten: Vec::new(),
            count: 1,
        });
        for _ in 1..state.count {
            for c in typed.chars() {
                self.type_replacing(c);
            }
        }
        self.replacing = None;
    }
}
