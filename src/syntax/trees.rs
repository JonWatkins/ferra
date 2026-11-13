//! Syntax trees kept per document and updated as the text changes.
//!
//! The editor holds one [`SyntaxTrees`], so indentation and highlighting
//! share a document's tree. `Buffer` logs every edit (insert and delete,
//! so undo, redo and reloads too) as byte offsets and points; when a tree
//! is asked for, the edits since it was built are fed to `Tree::edit` and
//! the text re-parsed reusing the old tree, so only what changed is parsed
//! again. A different buffer (`Buffer::id`; clones get new ids), a
//! different language or a log that's moved past means a full parse.
//!
//! Asking for a tree only needs `&self` (the trees sit in a `RefCell`), so
//! drawing, which borrows the editor immutably, can bring them up to date.

use std::cell::RefCell;
use std::collections::HashMap;

use tree_sitter::{InputEdit, Parser, Point, Tree};

use super::{LanguageDef, MAX_HIGHLIGHT_CHARS};
use crate::core::{Buffer, TextEdit};
use crate::editor::{Document, DocumentId};

#[derive(Default)]
pub struct SyntaxTrees {
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    parser: Parser,
    kept: HashMap<DocumentId, Kept>,
}

struct Kept {
    /// Compared by address: a plugin's updated grammar has the same name.
    language: &'static LanguageDef,
    buffer_id: u64,
    /// The buffer's `edit_seq` the tree was last brought up to.
    edit_seq: u64,
    /// `None` if the document is too large, or parsing failed.
    tree: Option<Tree>,
}

impl std::fmt::Debug for SyntaxTrees {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.inner.try_borrow().map_or(0, |inner| inner.kept.len());
        f.debug_struct("SyntaxTrees")
            .field("documents", &count)
            .finish()
    }
}

impl SyntaxTrees {
    /// `document`'s syntax tree in the built-in language `language`, up to
    /// date with its text. `None` for an unknown language, a document over
    /// [`MAX_HIGHLIGHT_CHARS`], or a failed parse. Cloning a tree is cheap
    /// (it shares its nodes).
    pub fn tree(&self, document: &Document, language: &str) -> Option<Tree> {
        let definition = super::language(language)?;
        let mut inner = self.inner.borrow_mut();
        let Inner { parser, kept } = &mut *inner;
        let buffer = document.buffer();
        let id = document.id();

        let edits = kept
            .get(&id)
            .filter(|k| std::ptr::eq(k.language, definition) && k.buffer_id == buffer.id())
            .and_then(|k| buffer.edits_since(k.edit_seq))
            .map(<[TextEdit]>::to_vec);
        match edits {
            Some(edits) => {
                let entry = kept.get_mut(&id).expect("checked above");
                if !edits.is_empty() {
                    entry.tree = entry.tree.take().and_then(|mut tree| {
                        for edit in &edits {
                            tree.edit(&input_edit(edit));
                        }
                        parse(parser, definition, buffer, Some(&tree))
                    });
                    entry.edit_seq = buffer.edit_seq();
                }
                entry.tree.clone()
            }
            None => {
                let tree = if buffer.len_chars() > MAX_HIGHLIGHT_CHARS {
                    None
                } else {
                    parse(parser, definition, buffer, None)
                };
                kept.insert(
                    id,
                    Kept {
                        language: definition,
                        buffer_id: buffer.id(),
                        edit_seq: buffer.edit_seq(),
                        tree: tree.clone(),
                    },
                );
                tree
            }
        }
    }

    /// Drops the trees of documents that are no longer open.
    pub fn retain(&self, open: &[DocumentId]) {
        self.inner
            .borrow_mut()
            .kept
            .retain(|id, _| open.contains(id));
    }

    /// How many documents have a tree kept.
    pub fn count(&self) -> usize {
        self.inner.borrow().kept.len()
    }
}

/// Parses `buffer`, reading it chunk by chunk from the rope, reusing the
/// unchanged parts of `old` (already told about the edits) if given.
fn parse(
    parser: &mut Parser,
    language: &LanguageDef,
    buffer: &Buffer,
    old: Option<&Tree>,
) -> Option<Tree> {
    parser.set_language(&language.grammar()).ok()?;
    parser.parse_with_options(&mut |byte, _| buffer.chunk_from_byte(byte), old, None)
}

fn input_edit(edit: &TextEdit) -> InputEdit {
    let point = |(row, column): (usize, usize)| Point::new(row, column);
    InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: point(edit.start),
        old_end_position: point(edit.old_end),
        new_end_position: point(edit.new_end),
    }
}

/// A fresh parse of `buffer` in `language`, for checking kept trees.
#[cfg(test)]
pub(super) fn parse_fresh(language: &str, buffer: &Buffer) -> Option<Tree> {
    let definition = super::language(language)?;
    parse(&mut Parser::new(), definition, buffer, None)
}
