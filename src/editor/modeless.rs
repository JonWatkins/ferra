//! VS Code's keys (`keymap = "vscode"`): no modes. Insert mode is where
//! the editor rests (asking for normal mode gives insert mode), a
//! selection is visual mode with an exclusive end (`Selection::exclusive`)
//! that typing replaces, and the clipboard is the system's.
//!
//! The keys themselves are in `config/keymaps/vscode.toml`; these are the
//! editor's side of the commands they run (`commands/builtin/modeless.rs`).

use super::registers::{Register, StoreKind};
use super::selection::{Selection, SelectionKind};
use super::{Editor, Mode};
use crate::config::KeymapPreset;
use crate::core::{Position, Range, motion};

/// Where a cursor move goes (Shift with it selects on the way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// The first non-blank, or the line's start if it's already there.
    Home,
    End,
    DocStart,
    DocEnd,
}

/// The clipboard register.
const CLIPBOARD: char = '+';

impl Editor {
    /// Whether VS Code's keys are in use, rather than Vim's.
    pub fn modeless(&self) -> bool {
        self.options.keymap == KeymapPreset::Vscode
    }

    /// Whether the editor is resting in its usual mode (normal for Vim,
    /// insert for VS Code's keys): when hovers, blame and the like show.
    pub fn resting(&self) -> bool {
        match self.mode() {
            Mode::Normal => true,
            Mode::Insert => self.modeless(),
            _ => false,
        }
    }

    /// After `keymap` changed: into the new keymap's resting mode.
    pub fn keymap_switched(&mut self) {
        match self.mode() {
            Mode::Visual => self.set_mode(Mode::Normal),
            Mode::Normal if self.modeless() => self.set_mode(Mode::Normal),
            Mode::Insert if !self.modeless() => self.set_mode(Mode::Normal),
            _ => {}
        }
    }

    /// The selected text's range, if anything is selected.
    fn selected(&self) -> Option<(Position, Position)> {
        match self.selection_range()? {
            Range::Chars { start, end } if start != end => Some((start, end)),
            Range::Chars { .. } => None,
            Range::Lines { first, last } => {
                let buffer = self.document.buffer();
                let end = if last + 1 < buffer.line_count() {
                    Position::new(last + 1, 0)
                } else {
                    buffer.line_end(last)
                };
                Some((Position::new(first, 0), end))
            }
        }
    }

    /// The lines a selection (or the cursor) is on. A selection ending at
    /// a line's very start doesn't take that line in, as in VS Code.
    fn selected_rows(&self) -> (usize, usize) {
        match self.selected() {
            Some((start, end)) if end.col == 0 && end.row > start.row => (start.row, end.row - 1),
            Some((start, end)) => (start.row, end.row),
            None => {
                let row = self.document.cursor().row;
                (row, row)
            }
        }
    }

    /// Shift and a move: starts a selection at the cursor if there isn't
    /// one (the move itself is the caller's).
    pub fn begin_selection(&mut self) {
        if self.mode() == Mode::Visual && self.selection().is_some() {
            return;
        }
        let anchor = self.document.cursor();
        self.set_mode(Mode::Visual);
        self.selection = Some(Selection {
            anchor,
            kind: SelectionKind::Char,
            exclusive: true,
        });
    }

    /// Drops the selection, the cursor where it is.
    pub fn clear_selection(&mut self) {
        if self.mode() == Mode::Visual {
            self.set_mode(Mode::Normal);
        }
    }

    /// A move without Shift: with a selection, `←` and `→` go to its start
    /// and end (and no further), as in VS Code; other moves drop it and go
    /// on from the cursor. Returns whether that was the whole move.
    pub fn collapse_selection(&mut self, to: Move) -> bool {
        let Some((start, end)) = self.selected() else {
            self.clear_selection();
            return false;
        };
        self.clear_selection();
        match to {
            Move::Left => self.document.set_cursor(start),
            Move::Right => self.document.set_cursor(end),
            _ => return false,
        }
        true
    }

    /// Moves the cursor (selecting on the way with `select`).
    pub fn modeless_move(&mut self, to: Move, select: bool) {
        if select {
            self.begin_selection();
        } else if self.collapse_selection(to) {
            return;
        }
        let buffer = self.document.buffer();
        let at = self.document.cursor();
        let last = buffer.line_count() - 1;
        let target = match to {
            Move::Left if at.col > 0 => at.with_col(at.col - 1),
            Move::Left if at.row > 0 => buffer.line_end(at.row - 1),
            Move::Left => at,
            Move::Right if at.col < buffer.line_len(at.row) => at.with_col(at.col + 1),
            Move::Right if at.row < last => Position::new(at.row + 1, 0),
            Move::Right => at,
            Move::WordLeft => motion::word_backward(buffer, at),
            Move::WordRight => {
                // Past the end of this word, or the next.
                let end = motion::word_end(buffer, at);
                if end <= at {
                    buffer.line_end(at.row)
                } else if end.col < buffer.line_len(end.row) {
                    end.with_col(end.col + 1)
                } else {
                    end
                }
            }
            Move::Home => {
                let first = motion::first_non_blank(buffer, at.row);
                at.with_col(if at.col == first { 0 } else { first })
            }
            Move::End => buffer.line_end(at.row),
            Move::DocStart => Position::new(0, 0),
            Move::DocEnd => buffer.line_end(last),
        };
        if matches!(to, Move::DocStart | Move::DocEnd) {
            self.push_jump();
        }
        self.document.set_cursor(target);
    }

    /// `Ctrl+A`: everything.
    pub fn select_all(&mut self) {
        self.clear_selection();
        self.document.set_cursor(Position::new(0, 0));
        self.begin_selection();
        let buffer = self.document.buffer();
        let end = buffer.line_end(buffer.line_count() - 1);
        self.document.set_cursor(end);
    }

    /// `Ctrl+L`: the cursor's line (and the next one each time again).
    pub fn select_line(&mut self) {
        let buffer = self.document.buffer();
        let last = buffer.line_count() - 1;
        let (first, row) = match self.selected() {
            Some((start, end)) => (start.row, if end.col == 0 { end.row } else { end.row + 1 }),
            None => {
                let row = self.document.cursor().row;
                (row, row + 1)
            }
        };
        let end = if row > last {
            buffer.line_end(last)
        } else {
            Position::new(row, 0)
        };
        self.clear_selection();
        self.document.set_cursor(Position::new(first, 0));
        self.begin_selection();
        self.document.set_cursor(end);
    }

    /// `Ctrl+D`: the word at the cursor; with a selection, the next place
    /// its text is found (ferra has one cursor, so the selection moves).
    pub fn select_next_match(&mut self) -> Result<(), String> {
        let buffer = self.document.buffer();
        let Some((start, end)) = self.selected() else {
            let at = self.document.cursor();
            let line: Vec<char> = buffer.line(at.row).chars().collect();
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            let mut from = at.col.min(line.len());
            let mut to = from;
            while from > 0 && line.get(from - 1).is_some_and(|c| is_word(*c)) {
                from -= 1;
            }
            while line.get(to).is_some_and(|c| is_word(*c)) {
                to += 1;
            }
            if from == to {
                return Ok(());
            }
            self.document.set_cursor(at.with_col(from));
            self.begin_selection();
            self.document.set_cursor(at.with_col(to));
            return Ok(());
        };
        let needle = buffer.text_range(start, end);
        let text = buffer.text();
        let offset = |p: Position| -> usize {
            text.split_inclusive('\n')
                .take(p.row)
                .map(|l| l.chars().count())
                .sum::<usize>()
                + p.col
        };
        let chars: Vec<char> = text.chars().collect();
        let needle: Vec<char> = needle.chars().collect();
        let from = offset(end);
        let found = (from..chars.len())
            .chain(0..from)
            .find(|&i| chars[i..].starts_with(&needle))
            .ok_or("No other match")?;
        let to_position = |index: usize| {
            let before = &chars[..index];
            let row = before.iter().filter(|&&c| c == '\n').count();
            let col = before.iter().rev().take_while(|&&c| c != '\n').count();
            Position::new(row, col)
        };
        let (a, b) = (to_position(found), to_position(found + needle.len()));
        self.clear_selection();
        self.document.set_cursor(a);
        self.begin_selection();
        self.document.set_cursor(b);
        Ok(())
    }

    /// Deletes the selection, if there is one. Returns whether it did.
    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selected() else {
            return false;
        };
        self.clear_selection();
        self.document.delete(start, end);
        self.document.set_cursor(start);
        true
    }

    /// A character typed over a selection: it replaces it.
    pub(super) fn type_over_selection(&mut self, c: char) {
        self.delete_selection();
        self.set_mode(Mode::Insert);
        self.insert_text_char(c);
    }

    /// `Backspace` and `Delete` (`forward`): the selection, or the
    /// character before or after the cursor (joining lines at their ends).
    pub fn delete_char_or_selection(&mut self, forward: bool) {
        if self.delete_selection() {
            return;
        }
        let at = self.document.cursor();
        let buffer = self.document.buffer();
        let last = buffer.line_count() - 1;
        let (start, end) = if forward {
            if at.col < buffer.line_len(at.row) {
                (at, at.with_col(at.col + 1))
            } else if at.row < last {
                (at, Position::new(at.row + 1, 0))
            } else {
                return;
            }
        } else if at.col > 0 {
            (at.with_col(at.col - 1), at)
        } else if at.row > 0 {
            (buffer.line_end(at.row - 1), at)
        } else {
            return;
        };
        self.document.delete(start, end);
        self.document.set_cursor(start);
    }

    /// `Ctrl+Backspace`, `Ctrl+Delete`: to the start of the word before,
    /// or the end of the one after.
    pub fn delete_word_or_selection(&mut self, forward: bool) {
        if self.delete_selection() {
            return;
        }
        let at = self.document.cursor();
        self.modeless_move(
            if forward {
                Move::WordRight
            } else {
                Move::WordLeft
            },
            false,
        );
        let other = self.document.cursor();
        let (start, end) = if other < at { (other, at) } else { (at, other) };
        self.document.delete(start, end);
        self.document.set_cursor(start);
    }

    /// `Ctrl+C`, `Ctrl+X` (`cut`): the selection, or with none the
    /// cursor's whole line, onto the clipboard.
    pub fn copy_selection(&mut self, cut: bool) -> Result<(), String> {
        let buffer = self.document.buffer();
        let (value, range) = match self.selected() {
            Some((start, end)) => (
                Register::charwise(buffer.text_range(start, end)),
                Some((start, end)),
            ),
            None => {
                let row = self.document.cursor().row;
                (Register::linewise(buffer.lines_text(row, row)), None)
            }
        };
        let kind = if cut {
            StoreKind::Delete
        } else {
            StoreKind::Yank
        };
        self.registers.store(Some(CLIPBOARD), value, kind)?;
        if cut {
            match range {
                Some(_) => {
                    self.delete_selection();
                }
                None => {
                    let row = self.document.cursor().row;
                    self.document.delete_lines(row, row);
                    let row = row.min(self.document.buffer().line_count() - 1);
                    self.document.set_cursor(Position::new(row, 0));
                }
            }
        }
        Ok(())
    }

    /// `Ctrl+V`: the clipboard over the selection, or at the cursor; a
    /// whole line copied (with nothing selected) goes in above the line.
    pub fn paste_clipboard(&mut self) -> Result<(), String> {
        let Some(value) = self.registers.read(Some(CLIPBOARD))? else {
            return Ok(());
        };
        let replaced = self.delete_selection();
        let at = self.document.cursor();
        if value.linewise && !replaced {
            self.document
                .insert(at.with_col(0), &format!("{}\n", value.text));
            let lines = value.text.split('\n').count();
            self.document
                .set_cursor(Position::new(at.row + lines, at.col));
        } else {
            let text = if value.linewise {
                format!("{}\n", value.text)
            } else {
                value.text
            };
            let end = self.document.insert(at, &text);
            self.document.set_cursor(end);
        }
        Ok(())
    }

    /// Pasted text (the terminal's own paste): over the selection.
    pub fn paste_over_selection(&mut self, text: &str) {
        self.delete_selection();
        self.set_mode(Mode::Insert);
        let at = self.document.cursor();
        let end = self.document.insert(at, text);
        self.document.set_cursor(end);
    }

    /// `Ctrl+Shift+K`: deletes the selected lines (or the cursor's).
    pub fn delete_selected_lines(&mut self) {
        let (first, last) = self.selected_rows();
        self.clear_selection();
        let col = self.document.cursor().col;
        self.document.delete_lines(first, last);
        let row = first.min(self.document.buffer().line_count() - 1);
        let col = col.min(self.document.buffer().line_len(row));
        self.document.set_cursor(Position::new(row, col));
    }

    /// `Alt+↑`/`Alt+↓`: moves the selected lines (or the cursor's) up or
    /// down one, keeping them selected.
    pub fn move_selected_lines(&mut self, down: bool) {
        let (first, last) = self.selected_rows();
        let count = self.document.buffer().line_count();
        if (down && last + 1 >= count) || (!down && first == 0) {
            return;
        }
        let selection = self.selected();
        let cursor = self.document.cursor();
        let (take, put) = if down {
            (last + 1, first)
        } else {
            (first - 1, last)
        };
        let line = self.document.buffer().line(take).into_owned();
        self.document.delete_lines(take, take);
        self.document
            .insert(Position::new(put, 0), &format!("{line}\n"));
        let shift = |p: Position| Position::new(if down { p.row + 1 } else { p.row - 1 }, p.col);
        match selection {
            Some((start, end)) => {
                let backwards = cursor == start;
                self.clear_selection();
                let (anchor, to) = if backwards {
                    (end, start)
                } else {
                    (start, end)
                };
                self.document.set_cursor(shift(anchor));
                self.begin_selection();
                self.document.set_cursor(shift(to));
            }
            None => self.document.set_cursor(shift(cursor)),
        }
    }

    /// `Shift+Alt+↑`/`↓`: copies the selected lines (or the cursor's) above
    /// or below themselves.
    pub fn copy_selected_lines(&mut self, down: bool) {
        let (first, last) = self.selected_rows();
        self.clear_selection();
        let text = self.document.buffer().lines_text(first, last);
        let cursor = self.document.cursor();
        self.document
            .insert(Position::new(first, 0), &format!("{text}\n"));
        if down {
            let lines = last - first + 1;
            self.document
                .set_cursor(Position::new(cursor.row + lines, cursor.col));
        } else {
            self.document.set_cursor(cursor);
        }
    }

    /// `Ctrl+/`: comments the selected lines (or the cursor's) in or out.
    pub fn toggle_comment_selected(&mut self) -> Result<(), String> {
        let (first, last) = self.selected_rows();
        self.keeping_selection(|editor| editor.toggle_comment(first, last))
    }

    /// `Tab` / `Shift+Tab` with a selection: indents or outdents its lines.
    pub fn shift_selected_lines(&mut self, indent: bool) {
        let (first, last) = self.selected_rows();
        let _ = self.keeping_selection(|editor| {
            editor.shift_lines(Range::Lines { first, last }, indent);
            Ok(())
        });
    }

    /// Runs `change` (which adds or takes text at the start of lines), the
    /// selection's ends and the cursor staying on the same text, as in VS
    /// Code: each moves with its line's change in length.
    fn keeping_selection(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        let selection = self.selection;
        let cursor = self.document.cursor();
        let len = |editor: &Self, row: usize| editor.document.buffer().line_len(row);
        let rows = [selection.map(|s| s.anchor.row), Some(cursor.row)];
        let before: Vec<Option<usize>> = rows.iter().map(|r| r.map(|r| len(self, r))).collect();
        change(self)?;
        let moved = |editor: &Self, at: Position, before: usize| {
            let after = len(editor, at.row);
            let col = if at.col == 0 {
                0
            } else {
                (at.col + after).saturating_sub(before).min(after)
            };
            at.with_col(col)
        };
        if let (Some(mut selection), Some(anchor_len)) = (selection, before[0]) {
            selection.anchor = moved(self, selection.anchor, anchor_len);
            self.selection = Some(selection);
            self.mode = Mode::Visual;
        }
        let cursor = moved(self, cursor, before[1].unwrap_or(0));
        self.document.set_cursor(cursor);
        Ok(())
    }

    /// `Esc`: drops the selection, completion, a hover, search highlights.
    pub fn modeless_escape(&mut self) {
        self.clear_selection();
        if self.completion.is_some() {
            self.cancel_completion();
        }
        self.clear_search_highlight();
    }
}
