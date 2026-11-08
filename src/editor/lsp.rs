//! What the editor asks of language servers (hover, go to definition,
//! references) and shows from their answers. The editor only queues the
//! requests; the app's `Lsp` sends them and hands back the answers, so this
//! layer knows nothing of servers or JSON.

use std::path::{Path, PathBuf};

use super::picker::{Picker, PickerAction, PickerItem};
use super::{Document, DocumentId, Editor, Message, Mode};
use crate::core::Position;

/// Something a language server can be asked about the cursor's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspFeature {
    Hover,
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
    Completion,
    /// The rest of a completion item that was taken (its import).
    ResolveCompletion,
    Format,
}

impl LspFeature {
    /// What it finds, for messages (`No definition found`).
    pub fn noun(self) -> &'static str {
        match self {
            LspFeature::Hover => "hover information",
            LspFeature::Definition => "definition",
            LspFeature::Declaration => "declaration",
            LspFeature::TypeDefinition => "type definition",
            LspFeature::Implementation => "implementation",
            LspFeature::References => "references",
            LspFeature::Completion | LspFeature::ResolveCompletion => "completion",
            LspFeature::Format => "formatting",
        }
    }

    /// The picker's title when there are several.
    pub fn title(self) -> &'static str {
        match self {
            LspFeature::Hover => "Hover",
            LspFeature::Definition => "Definitions",
            LspFeature::Declaration => "Declarations",
            LspFeature::TypeDefinition => "Type definitions",
            LspFeature::Implementation => "Implementations",
            LspFeature::References => "References",
            LspFeature::Completion | LspFeature::ResolveCompletion => "Completions",
            LspFeature::Format => "Formatting",
        }
    }
}

/// A request waiting to be sent: `feature` at `position` in `document`,
/// as its text was at `edit_seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspRequest {
    pub feature: LspFeature,
    pub document: DocumentId,
    pub position: Position,
    pub buffer_id: u64,
    pub edit_seq: u64,
    /// Asked for with a key, so a server that can't answer says so (rather
    /// than a completion asked for as you type).
    pub manual: bool,
    /// For completion: the character typed that might start one (`.`),
    /// and where the word being completed starts.
    pub trigger: Option<char>,
    pub word_start: Position,
    /// For resolving: which completion item (see `ServerItem::resolve`).
    pub item: Option<(u64, usize)>,
}

/// A place a server pointed to: a file, and a row and char column in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub position: Position,
    /// The line's text, shown in the picker.
    pub line: String,
}

/// A hover popup: the server's Markdown, in a document of its own so it
/// can be highlighted like any other (code blocks in their language).
#[derive(Debug, Clone)]
pub struct Hover {
    pub document: Document,
    /// The first of its rows shown (`K` again scrolls).
    pub scroll: usize,
    /// How many rows were shown last time it was drawn, and how many there
    /// are (wrapped, without code fences), so `K` can scroll by a page.
    pub shown: std::cell::Cell<usize>,
    pub total: std::cell::Cell<usize>,
}

/// Formats a buffer by asking its language server, waiting for the answer
/// (so `:wq` saves the formatted text). The app gives the editor one.
pub trait Formatter: std::fmt::Debug {
    /// Formats the active buffer. With `quiet` (formatting on save), not
    /// having a server that formats it is fine.
    fn format(&mut self, editor: &mut Editor, quiet: bool) -> Result<(), String>;
}

/// The id hover documents have: never one of the editor's buffers.
pub const HOVER_DOCUMENT: DocumentId = DocumentId(u64::MAX);

impl Editor {
    /// Lets `:format` and `format-on-save` ask language servers.
    pub fn set_formatter(&mut self, formatter: Box<dyn Formatter>) {
        self.formatter = Some(formatter);
    }

    /// `:format`, or before saving with `format-on-save` (`quiet`): has
    /// the buffer's language server format it.
    pub fn format(&mut self, quiet: bool) -> Result<(), String> {
        if !self.options.lsp {
            return if quiet {
                Ok(())
            } else {
                Err("Language servers are off (lsp = false)".into())
            };
        }
        let Some(mut formatter) = self.formatter.take() else {
            return if quiet {
                Ok(())
            } else {
                Err("No language server for this buffer (see :lsp)".into())
            };
        };
        let result = formatter.format(self, quiet);
        self.formatter = Some(formatter);
        result
    }

    /// Replaces each `start..end` with its text (a server's edits: they
    /// don't overlap, and are all about the text as it is now), keeping the
    /// cursor on the text it was on: moved along by edits before it, and
    /// kept at its line and column (as far as it can be) in one that
    /// replaces the text it was in.
    pub fn apply_text_edits(&mut self, mut edits: Vec<(Position, Position, String)>) {
        // Last first, so each edit's place is still where it was.
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let mut cursor = self.document.cursor();
        for (start, end, text) in edits {
            self.document.delete(start, end);
            let new_end = self.document.insert(start, &text);
            if end <= cursor {
                cursor = moved(cursor, end, new_end);
            } else if start < cursor {
                cursor = cursor.min(new_end);
            }
        }
        self.document.set_cursor(cursor);
        // In insert mode the cursor can be after the last char (after a
        // word just completed).
        if self.mode() != Mode::Insert {
            self.document.clamp_to_last_char();
        }
    }

    /// Asks the language server for `feature` at the cursor (`K`, `gd`...).
    pub fn request_lsp(&mut self, feature: LspFeature) -> Result<(), String> {
        if !self.options.lsp {
            return Err("Language servers are off (lsp = false)".into());
        }
        if self.document.path().is_none() {
            return Err("No language server: the buffer has no file".into());
        }
        let buffer = self.document.buffer();
        self.lsp_requests.push(LspRequest {
            feature,
            document: self.document.id(),
            position: self.document.cursor(),
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            manual: true,
            trigger: None,
            word_start: self.document.cursor(),
            item: None,
        });
        Ok(())
    }

    /// The requests made since this was last called, oldest first.
    pub fn take_lsp_requests(&mut self) -> Vec<LspRequest> {
        std::mem::take(&mut self.lsp_requests)
    }

    /// Whether the editor is still where `request` was made (same buffer,
    /// text and cursor), so its answer still applies.
    pub fn still_at(&self, request: &LspRequest) -> bool {
        let buffer = self.document.buffer();
        self.document.id() == request.document
            && buffer.id() == request.buffer_id
            && buffer.edit_seq() == request.edit_seq
            && self.document.cursor() == request.position
            && self.mode() == Mode::Normal
    }

    /// Shows `markdown` in a popup by the cursor, until the next key.
    pub fn show_hover(&mut self, markdown: &str) {
        let mut document = Document::with_text(markdown.trim());
        document.set_id(HOVER_DOCUMENT);
        let lines = document.buffer().line_count();
        self.hover = Some(Hover {
            document,
            scroll: 0,
            shown: std::cell::Cell::new(lines),
            total: std::cell::Cell::new(lines),
        });
        // Shown as it arrives, between keys: the next key is the first that
        // could close it.
        self.hover_used = true;
    }

    pub fn hover(&self) -> Option<&Hover> {
        self.hover.as_ref()
    }

    /// `K` with a hover open: scrolls it a page, or closes it at the end.
    /// Returns whether there was one.
    pub fn scroll_hover(&mut self) -> bool {
        let Some(hover) = &mut self.hover else {
            return false;
        };
        let page = hover.shown.get().max(1);
        let total = hover.total.get();
        if hover.scroll + page >= total {
            self.hover = None;
        } else {
            // The last page ends at the bottom, rather than half empty.
            hover.scroll = (hover.scroll + page).min(total - page);
            self.hover_used = true;
        }
        true
    }

    /// Called before each key.
    pub fn note_key_for_hover(&mut self) {
        self.hover_used = false;
    }

    /// Called after each key: one that didn't scroll the hover closes it.
    pub fn end_hover_unless_used(&mut self) {
        if !self.hover_used {
            self.hover = None;
        }
    }

    /// Goes to `location` (opening its file if need be), as a jump.
    pub fn goto_location(&mut self, location: &Location) -> Result<(), String> {
        let here = self
            .document
            .path()
            .is_some_and(|p| same_file(p, &location.path));
        if here {
            self.push_jump();
        } else {
            // Named as the user would (`src/shapes.rs`) when it's under
            // the current folder, as servers give full paths.
            let path = std::env::current_dir()
                .ok()
                .and_then(|dir| std::fs::canonicalize(dir).ok())
                .and_then(|dir| location.path.strip_prefix(dir).ok().map(Path::to_path_buf))
                .unwrap_or_else(|| location.path.clone());
            self.open(&path).map_err(|e| e.to_string())?;
        }
        self.document.set_cursor(location.position);
        self.document.clamp_to_last_char();
        self.reveal_cursor();
        Ok(())
    }

    /// Shows what a server found for `feature`: goes straight there if it's
    /// one place, else lists them in a picker.
    pub fn show_locations(
        &mut self,
        feature: LspFeature,
        locations: Vec<Location>,
    ) -> Result<(), String> {
        match locations.as_slice() {
            [] => Err(format!("No {} found", feature.noun())),
            [one] => self.goto_location(one),
            _ => {
                let here = std::env::current_dir().ok();
                let items = locations
                    .into_iter()
                    .map(|location| {
                        let shown = here
                            .as_deref()
                            .and_then(|dir| location.path.strip_prefix(dir).ok())
                            .unwrap_or(&location.path);
                        PickerItem {
                            label: format!(
                                "{}:{}:{}",
                                shown.display(),
                                location.position.row + 1,
                                location.position.col + 1
                            ),
                            detail: location.line.trim().to_string(),
                            action: PickerAction::GoTo(location.path, location.position),
                        }
                    })
                    .collect();
                self.open_picker(Picker::new(feature.title(), items));
                Ok(())
            }
        }
    }

    /// Reports a failed or empty answer.
    pub fn lsp_failed(&mut self, text: String) {
        self.set_message(Message::error(text));
    }
}

/// Where `cursor` (after `end`) is once the text up to `end` is replaced
/// by text ending at `new_end`.
fn moved(cursor: Position, end: Position, new_end: Position) -> Position {
    if end.row == cursor.row {
        Position::new(new_end.row, new_end.col + (cursor.col - end.col))
    } else {
        let rows = new_end.row as isize - end.row as isize;
        Position::new((cursor.row as isize + rows) as usize, cursor.col)
    }
}

/// Whether `a` and `b` are the same file, however they're written.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || canonical(a) == canonical(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(path: &Path, row: usize, col: usize) -> Location {
        Location {
            path: path.to_path_buf(),
            position: Position::new(row, col),
            line: "  some line ".into(),
        }
    }

    #[test]
    fn one_location_is_gone_to_and_several_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.rs");
        let b = dir.path().join("b.rs");
        std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
        std::fs::write(&b, "x\ny\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&a).unwrap();
        editor
            .show_locations(LspFeature::Definition, vec![location(&a, 2, 1)])
            .unwrap();
        assert_eq!(editor.document.cursor(), Position::new(2, 1));
        editor.jump_back(1);
        assert_eq!(editor.document.cursor(), Position::new(0, 0), "a jump");

        editor
            .show_locations(LspFeature::Definition, vec![location(&b, 1, 0)])
            .unwrap();
        assert!(same_file(editor.document.path().unwrap(), &b));
        assert_eq!(editor.document.cursor(), Position::new(1, 0));

        editor
            .show_locations(
                LspFeature::References,
                vec![location(&a, 0, 0), location(&b, 1, 0)],
            )
            .unwrap();
        let picker = editor.picker().unwrap();
        assert_eq!(picker.title(), "References");
        assert_eq!(picker.total(), 2);

        let none = editor.show_locations(LspFeature::Definition, vec![]);
        assert_eq!(none, Err("No definition found".into()));
    }

    #[test]
    fn answers_apply_only_while_the_editor_is_still_where_they_were_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "one two\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&path).unwrap();
        editor.request_lsp(LspFeature::Hover).unwrap();
        let request = editor.take_lsp_requests()[0];
        assert!(editor.still_at(&request));
        editor.document.set_cursor(Position::new(0, 4));
        assert!(!editor.still_at(&request), "the cursor moved");
        editor.document.set_cursor(Position::new(0, 0));
        assert!(editor.still_at(&request));
        editor.document.insert(Position::new(0, 0), "x");
        assert!(!editor.still_at(&request), "the text changed");
        assert!(editor.take_lsp_requests().is_empty(), "taken");

        let mut scratch = Editor::default();
        assert!(
            scratch.request_lsp(LspFeature::Definition).is_err(),
            "no file"
        );
    }

    #[test]
    fn the_cursor_moves_with_edits_before_it() {
        let at = Position::new;
        // `use a;\n` put in above: one row down.
        assert_eq!(moved(at(3, 4), at(0, 0), at(1, 0)), at(4, 4));
        // On its row, before it: along by the difference (`2..5` to `2..3`).
        assert_eq!(moved(at(0, 10), at(0, 5), at(0, 3)), at(0, 8));
        // A line joined into the one above (`0,3..1,0` deleted).
        assert_eq!(moved(at(1, 4), at(1, 0), at(0, 3)), at(0, 7));
    }

    #[test]
    fn edits_are_made_last_first_and_the_cursor_stays_on_its_text() {
        let mut editor = Editor::default();
        editor
            .document
            .insert(Position::new(0, 0), "fn  main(){\nlet x=1;\n}");
        editor.document.set_cursor(Position::new(1, 6));
        let at = Position::new;
        editor.apply_text_edits(vec![
            (at(0, 2), at(0, 4), " ".into()),
            (at(0, 10), at(0, 10), " ".into()),
            (at(1, 0), at(1, 0), "    ".into()),
            (at(1, 5), at(1, 6), " = ".into()),
        ]);
        assert_eq!(
            editor.document.buffer().text(),
            "fn main() {\n    let x = 1;\n}"
        );
        // Still on the `1`, moved along by the spaces before it.
        assert_eq!(editor.document.cursor(), at(1, 12));
        // In text an edit replaced: kept where it was, within the new text.
        editor.document.set_cursor(at(1, 6));
        editor.apply_text_edits(vec![(at(1, 4), at(1, 9), "letting".into())]);
        assert_eq!(editor.document.cursor(), at(1, 6));
    }

    #[test]
    fn k_scrolls_the_hover_then_closes_it() {
        let mut editor = Editor::default();
        editor.show_hover("1\n2\n3\n4\n5");
        editor.hover().unwrap().shown.set(2);
        editor.note_key_for_hover();
        assert!(editor.scroll_hover());
        editor.end_hover_unless_used();
        assert_eq!(editor.hover().unwrap().scroll, 2);
        editor.note_key_for_hover();
        editor.scroll_hover();
        editor.end_hover_unless_used();
        assert_eq!(editor.hover().unwrap().scroll, 3, "the last page, full");
        editor.note_key_for_hover();
        editor.scroll_hover();
        editor.end_hover_unless_used();
        assert!(editor.hover().is_none(), "past the end");

        editor.show_hover("x");
        editor.note_key_for_hover();
        editor.end_hover_unless_used();
        assert!(editor.hover().is_none(), "any other key closes it");
    }
}
