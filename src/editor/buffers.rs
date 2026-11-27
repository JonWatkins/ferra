//! Multiple open buffers.
//!
//! `Editor::document` is always the active buffer; the rest wait in
//! `background`, least recently used first. Switching swaps a background
//! document into place, so the rest of the editor only ever deals with
//! "the document".
//!
//! `Editor::document` is always the active buffer; the others wait in
//! `Editor::background`, least recently used first (`editor/buffers.rs`).
//! Switching swaps a background document into place, so the rest of the editor
//! only ever deals with "the document". Each `Document` has a `DocumentId`
//! (shown from 1), its own cursor and its own undo history. The app keeps each
//! buffer's scroll position (`App::other_views`).
//!
//! `Editor::open` switches to an already-open file (comparing canonical paths)
//! or opens a new buffer. A pristine scratch buffer is replaced rather than
//! kept, and its number is reused. `:q` and the write-and-quit commands check
//! every buffer for unsaved changes. `:wa` saves each modified buffer as the
//! active one, so its before-save hooks run.

use std::path::Path;

use super::{Document, DocumentId, Editor, Mode};

/// A summary of one open buffer, for listings and pickers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferInfo {
    pub id: DocumentId,
    pub name: String,
    pub modified: bool,
    pub active: bool,
}

impl Editor {
    /// All open buffers, in the order they were opened.
    pub fn buffers(&self) -> Vec<BufferInfo> {
        let mut list: Vec<BufferInfo> = std::iter::once(&self.document)
            .chain(&self.background)
            .map(|doc| BufferInfo {
                id: doc.id(),
                name: doc.name().into_owned(),
                modified: doc.is_modified(),
                active: doc.id() == self.document.id(),
            })
            .collect();
        list.sort_by_key(|b| b.id);
        list
    }

    pub fn buffer_count(&self) -> usize {
        1 + self.background.len()
    }

    /// Makes buffer `id` active. Returns `false` if there's no such buffer.
    pub fn switch_to(&mut self, id: DocumentId) -> bool {
        // From the terminal's scrollback: to the window it came from first.
        self.leave_scrollback();
        if id == self.document.id() {
            return true;
        }
        let Some(index) = self.background.iter().position(|d| d.id() == id) else {
            return false;
        };
        let document = self.background.remove(index);
        self.activate(document);
        self.check_active_disk();
        true
    }

    /// `:bn` / `:bp`: the next or previous buffer by number, wrapping.
    pub fn switch_relative(&mut self, offset: isize) {
        self.leave_scrollback();
        let ids: Vec<DocumentId> = self.buffers().iter().map(|b| b.id).collect();
        let current = ids
            .iter()
            .position(|&id| id == self.document.id())
            .expect("the active buffer is listed");
        let target = (current as isize + offset).rem_euclid(ids.len() as isize) as usize;
        self.switch_to(ids[target]);
    }

    /// The most recently used other buffer, if any.
    pub fn alternate_buffer(&self) -> Option<DocumentId> {
        self.background.last().map(Document::id)
    }

    /// `:b#`: the most recently used other buffer.
    pub fn switch_alternate(&mut self) -> bool {
        match self.background.last() {
            Some(doc) => {
                let id = doc.id();
                self.switch_to(id)
            }
            None => false,
        }
    }

    /// Resolves `:b` arguments: a buffer number, `#` for the alternate
    /// buffer, or part of a buffer's name that matches only one buffer.
    pub fn find_buffer(&self, query: &str) -> Result<DocumentId, String> {
        if query == "#" {
            return self
                .background
                .last()
                .map(Document::id)
                .ok_or_else(|| "No alternate buffer".into());
        }
        let buffers = self.buffers();
        if let Ok(number) = query.parse::<u64>() {
            return buffers
                .iter()
                .find(|b| b.id.0 + 1 == number)
                .map(|b| b.id)
                .ok_or_else(|| format!("Buffer {number} does not exist"));
        }
        let matches: Vec<&BufferInfo> = buffers.iter().filter(|b| b.name.contains(query)).collect();
        match matches.as_slice() {
            [only] => Ok(only.id),
            [] => Err(format!("No matching buffer for {query}")),
            _ => Err(format!("More than one match for {query}")),
        }
    }

    /// `:bd`: closes the active buffer, switching to the most recently used
    /// other one (or an empty scratch buffer if it was the last).
    pub fn close_buffer(&mut self) {
        // The terminal's scrollback: back to the window it came from.
        if self.scrollback_to_windows() {
            return;
        }
        // The tab beside it in this window, else the most recently used.
        let beside = self
            .tab_to_show_after_closing()
            .and_then(|id| self.background.iter().position(|d| d.id() == id));
        let alone = beside.is_none() && self.windows.count() > 1 && !self.tab_open_elsewhere();
        let next = match beside
            .map(|i| self.background.remove(i))
            .or_else(|| self.background.pop())
        {
            Some(document) => document,
            None => {
                let mut scratch = Document::default();
                scratch.set_id(self.new_document_id());
                scratch
            }
        };
        let closed = std::mem::replace(&mut self.document, next);
        self.note_tab(closed.id(), self.document.id());
        // Other windows that showed the closed buffer show another tab.
        self.replace_window_document(closed.id(), self.document.id());
        self.set_mode(Mode::Normal);
        // Its window's last tab: the window goes too (as `:bd` in Vim).
        if alone {
            let _ = self.close_window();
        }
    }

    /// Any open buffer by id, active or not.
    pub fn document_by_id(&self, id: DocumentId) -> Option<&Document> {
        std::iter::once(&self.document)
            .chain(&self.background)
            .find(|doc| doc.id() == id)
    }

    /// The first buffer with unsaved changes, active buffer first.
    pub fn first_modified_buffer(&self) -> Option<&Document> {
        std::iter::once(&self.document)
            .chain(self.background.iter().rev())
            .find(|doc| doc.is_modified())
    }

    /// Ids of buffers with unsaved changes, in buffer order.
    pub fn modified_buffer_ids(&self) -> Vec<DocumentId> {
        self.buffers()
            .into_iter()
            .filter(|b| b.modified)
            .map(|b| b.id)
            .collect()
    }

    /// The open buffer for `path`, if any.
    pub fn find_open_path(&self, path: &Path) -> Option<DocumentId> {
        let wanted = canonical(path);
        std::iter::once(&self.document)
            .chain(&self.background)
            .find(|doc| doc.path().is_some_and(|p| canonical(p) == wanted))
            .map(Document::id)
    }

    /// Makes `document` active, keeping the current one in the background
    /// unless it's an untouched scratch buffer.
    pub(super) fn activate(&mut self, document: Document) {
        // A file opened from the terminal's scrollback goes in the window
        // it came from.
        self.leave_scrollback();
        // A file opened (from the sidebar, a picker) takes the windows'
        // place back from the settings editor.
        self.settings = None;
        self.keybindings = None;
        let previous = std::mem::replace(&mut self.document, document);
        let (from, to) = (previous.id(), self.document.id());
        self.note_tab(from, to);
        if !previous.is_pristine_scratch() {
            self.background.push(previous);
        } else {
            // An untouched scratch buffer's tab gives way to what's opened
            // over it; it stays open while another window has it.
            self.forget_tab_here(from);
            if self.in_other_window(from) {
                self.background.push(previous);
            } else {
                self.forget_tab(from);
            }
        }
        self.refresh_options();
        self.set_mode(Mode::Normal);
    }

    pub(super) fn new_document_id(&mut self) -> DocumentId {
        self.last_id += 1;
        DocumentId(self.last_id)
    }
}

/// The path with symlinks and `..` resolved where possible, so the same file
/// opened by two different paths is one buffer.
fn canonical(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An editor with buffers named `a`, `b`, `c` (ids 1, 2, 3 for users),
    /// the last one active.
    fn editor_with(names: &[&str]) -> Editor {
        let mut editor = Editor::default();
        for (i, name) in names.iter().enumerate() {
            let mut doc = Document::with_text(name);
            if i == 0 {
                doc.set_id(editor.document.id());
                editor.document = doc;
            } else {
                doc.set_id(editor.new_document_id());
                editor.activate(doc);
            }
        }
        editor
    }

    fn active_text(editor: &Editor) -> String {
        editor.document.buffer().text()
    }

    #[test]
    fn lists_buffers_in_id_order() {
        let editor = editor_with(&["a", "b", "c"]);
        let names: Vec<String> = editor.buffers().iter().map(|b| b.id.to_string()).collect();
        assert_eq!(names, ["1", "2", "3"]);
        assert!(editor.buffers()[2].active);
        assert_eq!(editor.buffer_count(), 3);
    }

    #[test]
    fn switch_keeps_each_buffers_state() {
        let mut editor = editor_with(&["a", "b"]);
        editor.document.insert_str("edited ");
        assert!(editor.switch_to(DocumentId(0)));
        assert_eq!(active_text(&editor), "a");
        assert!(editor.switch_to(DocumentId(1)));
        assert_eq!(active_text(&editor), "edited b");
        assert!(!editor.switch_to(DocumentId(99)));
    }

    #[test]
    fn next_and_previous_wrap() {
        let mut editor = editor_with(&["a", "b", "c"]);
        editor.switch_relative(1);
        assert_eq!(active_text(&editor), "a");
        editor.switch_relative(-1);
        assert_eq!(active_text(&editor), "c");
        editor.switch_relative(-1);
        assert_eq!(active_text(&editor), "b");
    }

    #[test]
    fn alternate_is_most_recently_used() {
        let mut editor = editor_with(&["a", "b", "c"]);
        editor.switch_to(DocumentId(0));
        assert!(editor.switch_alternate());
        assert_eq!(active_text(&editor), "c");
        assert!(editor.switch_alternate());
        assert_eq!(active_text(&editor), "a");
    }

    #[test]
    fn find_buffer_by_number_alternate_or_name() {
        let editor = editor_with(&["a", "b"]);
        assert_eq!(editor.find_buffer("1"), Ok(DocumentId(0)));
        assert_eq!(editor.find_buffer("#"), Ok(DocumentId(0)));
        assert!(
            editor
                .find_buffer("7")
                .unwrap_err()
                .contains("does not exist")
        );
        assert!(
            editor
                .find_buffer("scratch")
                .unwrap_err()
                .contains("More than one")
        );
        assert!(
            editor
                .find_buffer("zzz")
                .unwrap_err()
                .contains("No matching")
        );
    }

    #[test]
    fn close_switches_to_most_recent_then_scratch() {
        let mut editor = editor_with(&["a", "b"]);
        editor.close_buffer();
        assert_eq!(active_text(&editor), "a");
        assert_eq!(editor.buffer_count(), 1);
        editor.close_buffer();
        assert_eq!(active_text(&editor), "");
        assert_eq!(editor.buffer_count(), 1);
        assert_ne!(
            editor.document.id(),
            DocumentId(0),
            "a fresh scratch buffer"
        );
    }

    #[test]
    fn pristine_scratch_is_replaced_not_kept() {
        let mut editor = Editor::default();
        let mut doc = Document::with_text("file");
        doc.set_id(editor.new_document_id());
        editor.activate(doc);
        assert_eq!(editor.buffer_count(), 1);
    }

    #[test]
    fn modified_buffers_are_found() {
        let mut editor = editor_with(&["a", "b"]);
        assert!(editor.first_modified_buffer().is_none());
        editor.switch_to(DocumentId(0));
        editor.document.insert_str("x");
        editor.switch_to(DocumentId(1));
        assert_eq!(editor.first_modified_buffer().unwrap().id(), DocumentId(0));
        assert_eq!(editor.modified_buffer_ids(), [DocumentId(0)]);
    }
}
