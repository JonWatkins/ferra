//! Multiple cursors, as VS Code's (with its keys, `keymap = "vscode"`):
//! besides the editor's own cursor and selection (the primary), more
//! cursors, each with its own selection, in the active buffer.
//!
//! A command that edits or moves runs once per cursor (`for_each_cursor`),
//! from the bottom of the file up: each cursor's turn sets the editor's
//! cursor and selection to it, runs the command, and reads them back; the
//! other cursors move along with the edit it made (from the buffer's edit
//! log). Undo steps aren't ended in the middle (one cursor's turn can't
//! split them). Cursors that end up in the same
//! place become one.
//!
//! Added with `Ctrl+D` (the next match of the selection), `Ctrl+Shift+L`
//! (every match), `Ctrl+Shift+↑`/`↓` (`Ctrl+Alt+↑`/`↓`: a cursor above or
//! below) and `Ctrl+click`; `Esc` goes back to one.

use super::{DocumentId, Editor, Mode, Selection, SelectionKind};
use crate::core::Position;

/// The commands that run once at each cursor (the rest run once, and the
/// extra cursors go if they move the cursor or change the text).
pub const PER_CURSOR: &[&str] = &[
    "cursor_left",
    "cursor_right",
    "cursor_up",
    "cursor_down",
    "cursor_word_left",
    "cursor_word_right",
    "cursor_home",
    "cursor_end",
    "select_left",
    "select_right",
    "select_up",
    "select_down",
    "select_word_left",
    "select_word_right",
    "select_home",
    "select_end",
    "select_line",
    "delete_left",
    "delete_right",
    "delete_word_left",
    "delete_word_right",
    "accept_or_newline",
    "insert_newline",
    "indent_or_tab",
    "insert_tab",
    "outdent_lines",
    "paste_clipboard",
    "toggle_comment",
    "open_line_below",
    "open_line_above",
];

/// The commands that make or keep several cursors (run once, keeping
/// them).
pub const KEEPS_CURSORS: &[&str] = &[
    "select_next_match",
    "select_all_matches",
    "add_cursor_below",
    "add_cursor_above",
    "copy",
    "cut",
];

/// A cursor besides the primary one, and where its selection starts (if it
/// has one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtraCursor {
    pub cursor: Position,
    pub anchor: Option<Position>,
}

impl ExtraCursor {
    /// Its selection's start and end (`None`: no selection).
    pub fn range(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| {
            if anchor < self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    /// Where it is in the text: its selection's start, or the cursor.
    fn start(&self) -> Position {
        self.range().map_or(self.cursor, |(start, _)| start)
    }
}

/// The extra cursors, and the buffer they're in.
#[derive(Debug, Clone, Default)]
pub struct MultiCursors {
    document: Option<DocumentId>,
    extra: Vec<ExtraCursor>,
    /// What each cursor copied, last time several did (so a paste with as
    /// many cursors gives each its own).
    pub(super) copied: Vec<String>,
    /// The text the cursor having its turn pastes, for `paste_clipboard`.
    pub(super) paste_piece: Option<String>,
    /// The column cursors added above and below aim for (kept past short
    /// lines).
    vertical_col: Option<usize>,
}

/// Moves the byte offset `at` along with `edit`: after it, by how much it
/// grew or shrank; inside text it deleted, to where that was.
fn shift(at: usize, edit: &crate::core::TextEdit) -> usize {
    if at < edit.start_byte {
        at
    } else if at >= edit.old_end_byte {
        at + edit.new_end_byte - edit.old_end_byte
    } else {
        edit.start_byte.min(edit.new_end_byte)
    }
}

impl Editor {
    /// The extra cursors in the buffer shown (none if they were made in
    /// another).
    pub fn extra_cursors(&self) -> &[ExtraCursor] {
        if self.cursors.document == Some(self.document.id()) {
            &self.cursors.extra
        } else {
            &[]
        }
    }

    pub fn has_extra_cursors(&self) -> bool {
        !self.extra_cursors().is_empty()
    }

    /// Back to the one cursor (`Esc`, or a command that only makes sense
    /// for one). Returns whether there were others.
    pub fn clear_extra_cursors(&mut self) -> bool {
        let had = self.has_extra_cursors();
        self.cursors.extra.clear();
        self.cursors.document = None;
        had
    }

    /// The primary cursor as an `ExtraCursor`.
    fn primary(&self) -> ExtraCursor {
        let cursor = self.document.cursor();
        let anchor = match (self.mode(), self.selection) {
            (Mode::Visual, Some(selection)) if selection.anchor != cursor => Some(selection.anchor),
            _ => None,
        };
        ExtraCursor { cursor, anchor }
    }

    /// Makes `cursor` the editor's own cursor and selection.
    fn load(&mut self, cursor: ExtraCursor) {
        let buffer = self.document.buffer();
        let clamp = |p: Position| {
            let row = p.row.min(buffer.line_count() - 1);
            Position::new(row, p.col.min(buffer.line_len(row)))
        };
        let (at, anchor) = (clamp(cursor.cursor), cursor.anchor.map(clamp));
        self.document.set_cursor(at);
        match anchor {
            Some(anchor) if anchor != at => {
                self.selection = Some(Selection {
                    anchor,
                    kind: SelectionKind::Char,
                    exclusive: true,
                });
                self.mode = Mode::Visual;
            }
            _ => {
                self.selection = None;
                self.mode = Mode::Insert;
            }
        }
    }

    /// Sets the cursors: `primary` and the rest (merged where they meet).
    fn set_cursors(&mut self, primary: ExtraCursor, mut rest: Vec<ExtraCursor>) {
        rest.retain(|c| c.cursor != primary.cursor);
        rest.sort_by_key(|c| c.start());
        rest.dedup_by_key(|c| c.cursor);
        self.cursors.vertical_col = None;
        self.load(primary);
        self.cursors.document = (!rest.is_empty()).then(|| self.document.id());
        self.cursors.extra = rest;
    }

    /// Runs `run` once for each cursor (bottom first), as one undo step;
    /// the first error, if any, is returned (the rest still run).
    pub fn for_each_cursor(
        &mut self,
        mut run: impl FnMut(&mut Editor) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut all: Vec<ExtraCursor> = vec![self.primary()];
        all.extend_from_slice(self.extra_cursors());
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(all[i].start()));
        if self.completion.is_some() {
            self.cancel_completion();
        }
        let pieces = self.cursors.copied.clone();
        let paste_each = pieces.len() == all.len()
            && pieces.len() > 1
            && self
                .registers
                .read(Some(super::registers::CLIPBOARD))
                .ok()
                .flatten()
                .is_some_and(|value| value.text == pieces.join("\n"));
        // Pieces go to the cursors in the text's order.
        let mut by_place: Vec<usize> = (0..all.len()).collect();
        by_place.sort_by_key(|&i| all[i].start());

        self.document.hold_commits(true);
        let mut first_error = None;
        for &i in &order {
            let buffer = self.document.buffer();
            let seq = buffer.edit_seq();
            let bytes: Vec<(usize, Option<usize>)> = all
                .iter()
                .map(|c| {
                    (
                        buffer.byte_of(c.cursor),
                        c.anchor.map(|a| buffer.byte_of(a)),
                    )
                })
                .collect();
            self.load(all[i]);
            self.cursors.paste_piece = paste_each
                .then(|| by_place.iter().position(|&p| p == i))
                .flatten()
                .map(|place| pieces[place].clone());
            if let Err(error) = run(self) {
                first_error.get_or_insert(error);
            }
            all[i] = self.primary();
            let buffer = self.document.buffer();
            if let Some(edits) = buffer.edits_since(seq) {
                for (j, (cursor, anchor)) in bytes.into_iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    let along = |mut at: usize| {
                        for edit in edits {
                            at = shift(at, edit);
                        }
                        buffer.position_at_byte(at)
                    };
                    all[j] = ExtraCursor {
                        cursor: along(cursor),
                        anchor: anchor.map(along),
                    };
                }
            }
        }
        self.cursors.paste_piece = None;
        self.document.hold_commits(false);
        let primary = all.remove(0);
        self.set_cursors(primary, all);
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// `Ctrl+C` / `Ctrl+X` with several cursors: each one's selection (or
    /// line), in the text's order, a line each on the clipboard (and kept
    /// apart, for a paste with as many cursors).
    pub fn copy_at_cursors(&mut self, cut: bool) -> Result<(), String> {
        let mut all: Vec<ExtraCursor> = vec![self.primary()];
        all.extend_from_slice(self.extra_cursors());
        all.sort_by_key(|c| c.start());
        let buffer = self.document.buffer();
        let pieces: Vec<String> = all
            .iter()
            .map(|c| match c.range() {
                Some((start, end)) => buffer.text_range(start, end),
                None => buffer.line(c.cursor.row).into_owned(),
            })
            .collect();
        if cut {
            self.for_each_cursor(|editor| editor.copy_selection(true))?;
        }
        let text = pieces.join("\n");
        self.registers.store(
            Some(super::registers::CLIPBOARD),
            super::registers::Register::charwise(text),
            super::registers::StoreKind::Yank,
        )?;
        self.cursors.copied = pieces;
        Ok(())
    }

    /// `Ctrl+Shift+↓` / `↑`: a cursor on the line below the lowest (above
    /// the highest), at the same column (or the line's end).
    pub fn add_cursor_vertically(&mut self, below: bool) {
        let mut all: Vec<ExtraCursor> = vec![self.primary()];
        all.extend_from_slice(self.extra_cursors());
        let from = if below {
            all.iter().map(|c| c.cursor).max()
        } else {
            all.iter().map(|c| c.cursor).min()
        };
        let Some(from) = from else { return };
        let buffer = self.document.buffer();
        let row = if below {
            if from.row + 1 >= buffer.line_count() {
                return;
            }
            from.row + 1
        } else {
            let Some(row) = from.row.checked_sub(1) else {
                return;
            };
            row
        };
        let wanted = self.cursors.vertical_col.unwrap_or(from.col);
        let col = wanted.min(buffer.line_len(row));
        let primary = self.primary();
        // The new one's the primary (the view follows it).
        all.remove(0);
        all.push(ExtraCursor {
            cursor: primary.cursor,
            anchor: None,
        });
        let new = ExtraCursor {
            cursor: Position::new(row, col),
            anchor: None,
        };
        let rest = all
            .into_iter()
            .map(|c| ExtraCursor { anchor: None, ..c })
            .collect();
        self.set_cursors(new, rest);
        self.cursors.vertical_col = Some(wanted);
    }

    /// `Ctrl+click`: a cursor there, or the one there taken away.
    pub fn toggle_cursor_at(&mut self, at: Position) {
        let primary = self.primary();
        let mut rest: Vec<ExtraCursor> = self.extra_cursors().to_vec();
        if let Some(i) = rest.iter().position(|c| c.cursor == at) {
            rest.remove(i);
            self.set_cursors(primary, rest);
            return;
        }
        if primary.cursor == at {
            if rest.is_empty() {
                return;
            }
            let next = rest.remove(rest.len() - 1);
            self.set_cursors(next, rest);
            return;
        }
        rest.push(primary);
        self.set_cursors(
            ExtraCursor {
                cursor: at,
                anchor: None,
            },
            rest,
        );
    }

    /// Every place `needle` is in the text, as (start, end).
    fn matches_of(&self, needle: &str) -> Vec<(Position, Position)> {
        let buffer = self.document.buffer();
        let text = buffer.text();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(at) = text[from..].find(needle) {
            let start = from + at;
            let end = start + needle.len();
            out.push((buffer.position_at_byte(start), buffer.position_at_byte(end)));
            from = end;
        }
        out
    }

    /// `Ctrl+D`: with nothing selected, the word at the cursor; then each
    /// time, a cursor selecting the next place the selected text is (after
    /// the newest cursor, wrapping), skipping those already selected.
    pub fn add_next_match(&mut self) -> Result<(), String> {
        let primary = self.primary();
        let Some((start, end)) = primary.range() else {
            return self.select_next_match();
        };
        let needle = self.document.buffer().text_range(start, end);
        let taken: Vec<Position> = std::iter::once(primary)
            .chain(self.extra_cursors().iter().copied())
            .filter_map(|c| c.range().map(|(s, _)| s))
            .collect();
        let matches = self.matches_of(&needle);
        let next = matches
            .iter()
            .filter(|(s, _)| *s >= end)
            .chain(matches.iter().filter(|(s, _)| *s < end))
            .find(|(s, _)| !taken.contains(s))
            .copied();
        let Some((s, e)) = next else {
            return Err("No other match".into());
        };
        let mut rest = self.extra_cursors().to_vec();
        rest.push(primary);
        self.set_cursors(
            ExtraCursor {
                cursor: e,
                anchor: Some(s),
            },
            rest,
        );
        Ok(())
    }

    /// `Ctrl+Shift+L`: a cursor selecting each place the selected text (or
    /// the word at the cursor) is.
    pub fn select_all_matches(&mut self) -> Result<(), String> {
        if self.primary().range().is_none() {
            self.select_next_match()?;
        }
        let Some((start, end)) = self.primary().range() else {
            return Ok(());
        };
        let needle = self.document.buffer().text_range(start, end);
        let matches = self.matches_of(&needle);
        let cursors: Vec<ExtraCursor> = matches
            .iter()
            .map(|&(s, e)| ExtraCursor {
                cursor: e,
                anchor: Some(s),
            })
            .collect();
        // The one at the cursor stays the primary.
        let primary = cursors
            .iter()
            .copied()
            .find(|c| c.anchor == Some(start))
            .unwrap_or(ExtraCursor {
                cursor: end,
                anchor: Some(start),
            });
        let count = cursors.len();
        self.set_cursors(primary, cursors);
        self.set_message(super::Message::info(format!("{count} selected")));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn editor(text: &str) -> Editor {
        let options = crate::config::EditorOptions {
            keymap: crate::config::KeymapPreset::Vscode,
            ..Default::default()
        };
        let mut editor = Editor::with_document(Document::with_text(text));
        editor.options = options;
        editor.set_mode(Mode::Insert);
        editor
    }

    fn cursors(editor: &Editor) -> Vec<(usize, usize)> {
        let mut all: Vec<(usize, usize)> = std::iter::once(editor.document.cursor())
            .chain(editor.extra_cursors().iter().map(|c| c.cursor))
            .map(|p| (p.row, p.col))
            .collect();
        all.sort();
        all
    }

    #[test]
    fn typing_at_three_cursors_on_one_line_and_others() {
        let mut e = editor("ab ab\nab");
        e.toggle_cursor_at(Position::new(0, 3));
        e.toggle_cursor_at(Position::new(1, 0));
        e.for_each_cursor(|e| {
            e.insert_text_char('x');
            Ok(())
        })
        .unwrap();
        assert_eq!(e.document.buffer().text(), "xab xab\nxab");
        assert_eq!(cursors(&e), [(0, 1), (0, 5), (1, 1)]);
        // One undo step takes all three back.
        e.document.undo();
        assert_eq!(e.document.buffer().text(), "ab ab\nab");
    }

    #[test]
    fn cursors_that_meet_become_one() {
        let mut e = editor("abc");
        e.document.set_cursor(Position::new(0, 1));
        e.toggle_cursor_at(Position::new(0, 2));
        assert_eq!(cursors(&e).len(), 2);
        e.for_each_cursor(|e| {
            e.delete_char_or_selection(false);
            Ok(())
        })
        .unwrap();
        assert_eq!(e.document.buffer().text(), "c");
        assert_eq!(cursors(&e), [(0, 0)]);
    }

    #[test]
    fn next_match_and_all_matches() {
        let mut e = editor("cat dog cat cat");
        e.add_next_match().unwrap();
        assert_eq!(
            e.primary().range(),
            Some((Position::new(0, 0), Position::new(0, 3)))
        );
        e.add_next_match().unwrap();
        assert_eq!(cursors(&e), [(0, 3), (0, 11)]);
        e.select_all_matches().unwrap();
        assert_eq!(cursors(&e), [(0, 3), (0, 11), (0, 15)]);
        e.for_each_cursor(|e| {
            e.insert_text_char('X');
            Ok(())
        })
        .unwrap();
        assert_eq!(e.document.buffer().text(), "X dog X X");
    }

    #[test]
    fn cursors_go_below_at_the_same_column() {
        let mut e = editor("abcd\nab\nabcd");
        e.document.set_cursor(Position::new(0, 3));
        e.add_cursor_vertically(true);
        e.add_cursor_vertically(true);
        assert_eq!(cursors(&e), [(0, 3), (1, 2), (2, 3)]);
        assert!(e.clear_extra_cursors());
        assert_eq!(cursors(&e), [(2, 3)]);
    }
}
