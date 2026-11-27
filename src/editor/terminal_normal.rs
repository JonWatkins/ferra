//! Terminal-normal mode, as Neovim's: the terminal panel's text as a
//! read-only buffer, moved in, searched and yanked from with the editor's
//! own keys.
//!
//! `<C-\><C-n>` in the panel (`terminal_normal`) takes the shell's text as
//! it is then (`Terminal::snapshot`: what's scrolled off, then the screen,
//! a line the terminal wrapped as one) into a `Document::scrollback`, with
//! the cursor where the shell's is, and makes it the buffer of the panel's
//! own window, `PANEL_WINDOW`. That window is never in the layout: the
//! panel draws it over the shell's screen (`ui/mod.rs`), with Neovim's
//! terminal-buffer options (no line numbers or fold column, `scroll-off`
//! 0, wrapped as the terminal wrapped it), and says how big it is
//! (`Windows::note_panel_area`). Since the active window's buffer is always
//! `Editor::document`, every motion, search, visual mode, `y` and the
//! registers work as in any window, and a click or drag in it is a click or
//! drag in a window. The text stays as it was taken; going back to the shell
//! and in again takes it again.
//!
//! Edits are refused (`Document::insert`/`delete` change nothing there), and
//! `CommandRegistry::execute` reports it. Going into insert mode (`i`, `a`,
//! `o`, after `c`...) goes back to the shell instead, at its live screen,
//! as in Neovim, and so does `<Space>t`. Any other way back to the windows
//! (`<C-w>k`, `<C-w>w`, `<C-w>p`, a click in one, opening a file, `:q`)
//! leaves the scrollback too: the window it came from gets its buffer and
//! cursor back (`leave_scrollback`), and the snapshot goes. So do the
//! panel's hiding, closing the shell, or another of its tabs.
//!
//! The mouse, with either keymap: a drag over the shell's screen (when its
//! program hasn't asked for the mouse) takes the snapshot under the pointer
//! (`scrollback_at`: the view starting where the screen does, so the text
//! stays put) and selects from where it started, as in a window. With VS
//! Code's keys, where there's no normal mode, that's the way in: `<C-c>`
//! copies the selection, and a key typed there goes to the shell, which
//! takes it back.

use super::windows::PANEL_WINDOW;
use super::{Document, Editor, Mode, WindowId};
use crate::core::Position;
use crate::editor::windows::ViewStart;

/// Terminal-normal mode's state: where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Scrollback {
    /// The window active before, which gets its buffer back.
    from: WindowId,
}

/// The panel window's own options: Neovim's for a terminal's buffer, and
/// wrapped as the terminal wraps, so its rows are the shell's.
fn panel_options() -> toml::Table {
    let mut table = toml::Table::new();
    for (name, value) in [
        ("line-numbers", toml::Value::from("none")),
        ("fold-column", toml::Value::from("off")),
        ("scroll-off", toml::Value::from(0)),
        ("wrap", toml::Value::from(true)),
        ("wrap-indent", toml::Value::from(false)),
        ("wrap-marker", toml::Value::from("")),
        ("indent-guides", toml::Value::from(false)),
        ("bracket-colors", toml::Value::from(false)),
    ] {
        table.insert(name.into(), value);
    }
    table
}

impl Editor {
    /// Whether terminal-normal mode is on: the panel's scrollback is the
    /// buffer.
    pub fn in_scrollback(&self) -> bool {
        self.scrollback.is_some()
    }

    /// `<C-\><C-n>` in the panel: its text as a read-only buffer, in
    /// normal mode, the cursor where the shell's is. With VS Code's keys
    /// (no normal mode): back to the windows, as before (a drag selects).
    pub fn terminal_normal(&mut self) -> Result<(), String> {
        if self.modeless() {
            self.leave_terminal();
            return Ok(());
        }
        let snapshot = self
            .terminal_panel()
            .ok_or("The terminal panel isn't open")?
            .snapshot();
        let (line, char) = snapshot.cursor;
        self.enter_scrollback(&snapshot.lines, Position::new(line, char), None);
        self.set_mode(Mode::Normal);
        Ok(())
    }

    /// A drag starting at row `row`, column `col` of the shell's screen
    /// (when its program hasn't asked for the mouse): its text as a
    /// read-only buffer that starts where the screen does, the cursor at
    /// the start of the drag. Returns it, in the buffer, or `None` if
    /// the panel isn't shown.
    pub fn scrollback_at(&mut self, row: u16, col: u16) -> Option<Position> {
        let snapshot = self.terminal_panel()?.snapshot();
        let (line, char) = snapshot.at(row, col);
        let at = Position::new(line, char);
        // The view starts with the screen's top row: a line's own row,
        // or part way down a wrapped one.
        let top = snapshot.screen_rows.first().map(|&(line, start)| {
            let skip = snapshot
                .screen_rows
                .iter()
                .filter(|&&(l, s)| l == line && s < start)
                .count();
            ViewStart { line, skip }
        });
        self.enter_scrollback(&snapshot.lines, at, top);
        Some(at)
    }

    fn enter_scrollback(&mut self, lines: &[String], cursor: Position, top: Option<ViewStart>) {
        if self.in_scrollback() {
            self.leave_scrollback();
        }
        self.save_active_window();
        let mut document = Document::scrollback(&lines.join("\n"));
        document.set_id(self.new_document_id());
        document.set_cursor(cursor);
        let from = self.windows.active();
        let previous = std::mem::replace(&mut self.document, document);
        self.background.push(previous);
        self.scrollback = Some(Scrollback { from });
        self.windows.set_active(PANEL_WINDOW);
        self.window_options
            .entry(PANEL_WINDOW)
            .or_insert_with(panel_options);
        if let Some(top) = top {
            self.windows.request_top(PANEL_WINDOW, top);
        }
        self.refresh_options();
    }

    /// Out of terminal-normal mode, to the window it came from (its
    /// buffer and cursor back), the snapshot gone. Leaves the mode as it
    /// is: callers set it.
    pub(super) fn leave_scrollback(&mut self) {
        let Some(Scrollback { from }) = self.scrollback.take() else {
            return;
        };
        // That window, or (closed since) the first.
        let window = self
            .windows
            .get(from)
            .or_else(|| self.windows.list().first())
            .cloned()
            .expect("there's always a window");
        let previous = self
            .background
            .iter()
            .position(|d| d.id() == window.document)
            .or_else(|| self.background.len().checked_sub(1));
        let restored = match previous {
            Some(index) => self.background.remove(index),
            None => {
                let mut scratch = Document::default();
                scratch.set_id(self.new_document_id());
                scratch
            }
        };
        let scrollback = std::mem::replace(&mut self.document, restored);
        debug_assert!(scrollback.is_read_only());
        self.windows.set_active(window.id);
        self.document.set_cursor(window.cursor);
        self.refresh_options();
        self.enforce_cursor_rules();
    }

    /// From the scrollback back to the windows, in normal mode (with VS
    /// Code's keys, insert): the window it came from active again.
    /// Returns whether it was there to leave.
    pub(super) fn scrollback_to_windows(&mut self) -> bool {
        if !self.in_scrollback() {
            return false;
        }
        self.leave_scrollback();
        self.set_mode(Mode::Normal);
        true
    }

    /// Going into insert mode in the scrollback (`i`, `a`, `o`...), or to
    /// the panel's shell: the scrollback goes, and the shell has focus.
    /// Returns whether it was there to leave.
    pub(super) fn scrollback_to_shell(&mut self) -> bool {
        if !self.in_scrollback() {
            return false;
        }
        self.leave_scrollback();
        self.panel_open = true;
        self.panel.tab = super::panel::PanelTab::Terminal;
        true
    }

    /// A key typed in the scrollback that isn't the editor's (VS Code's
    /// keys, where typing would replace the selection): back to the shell,
    /// which gets it.
    pub fn scrollback_type(&mut self, key: crate::input::KeyPress) {
        self.set_mode(Mode::Terminal);
        self.terminal_keys(&[key]);
    }

    /// Edits were refused in the scrollback: the command's error, or with
    /// VS Code's keys (a key that edits a selection), the shell again.
    pub(crate) fn refused_edit(&mut self) -> Option<String> {
        if !self.document.take_refused_edit() {
            return None;
        }
        if self.modeless() {
            self.set_mode(Mode::Terminal);
            return None;
        }
        Some("The terminal's text can't be changed (i goes back to the shell)".into())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn editor_with_shell(script: &str) -> Editor {
        let mut editor = Editor::with_document(Document::with_text("file text\nsecond"));
        editor.options.shell = "/bin/sh".into();
        editor.focus_terminal().unwrap();
        let keys = crate::input::parse_keys(&format!("{script}<CR>")).unwrap();
        editor.terminal_keys(&keys);
        editor
    }

    fn update_until(editor: &mut Editor, done: impl Fn(&Editor) -> bool) {
        let start = Instant::now();
        while !done(editor) && start.elapsed() < Duration::from_secs(3) {
            editor.update_terminal();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn shows(editor: &Editor, text: &str) -> bool {
        editor
            .terminal_panel()
            .is_some_and(|t| t.text_lines().iter().any(|l| l == text))
    }

    #[test]
    fn the_shells_text_becomes_a_read_only_buffer_and_i_goes_back() {
        let mut editor = editor_with_shell("echo one; echo two");
        update_until(&mut editor, |e| shows(e, "two"));
        let file = editor.document.id();
        editor.terminal_normal().unwrap();
        assert!(editor.in_scrollback());
        assert_eq!(editor.mode(), Mode::Normal);
        assert_eq!(editor.windows().active(), PANEL_WINDOW);
        assert!(editor.document.is_read_only());
        assert_eq!(editor.document.name(), "[terminal]");
        let text = editor.document.buffer().text();
        assert!(text.contains("one\ntwo\n"), "{text:?}");
        // The cursor's on the prompt after the output: the last line.
        let last = editor.document.buffer().line_count() - 1;
        assert_eq!(editor.document.cursor().row, last);
        assert_eq!(
            editor.options.line_numbers,
            crate::config::LineNumbers::None,
            "Neovim's terminal options"
        );

        // Edits change nothing.
        editor.document.insert(Position::new(0, 0), "x");
        assert!(editor.refused_edit().is_some());
        assert_eq!(editor.document.buffer().text(), text);

        // Insert mode is the shell again; the file's back in its window.
        editor.set_mode(Mode::Insert);
        assert_eq!(editor.mode(), Mode::Terminal);
        assert!(!editor.in_scrollback());
        assert_eq!(editor.document.id(), file);
        assert_eq!(editor.windows().active(), WindowId(0));
        assert!(
            editor.buffers().iter().all(|b| b.name != "[terminal]"),
            "the snapshot's gone"
        );
    }

    #[test]
    fn going_to_a_window_or_opening_a_file_leaves_the_scrollback() {
        let mut editor = editor_with_shell("echo hi");
        update_until(&mut editor, |e| shows(e, "hi"));
        editor.document.set_cursor(Position::new(1, 2));
        editor.save_active_window();
        editor.terminal_normal().unwrap();
        assert!(editor.focus_window_in(super::super::windows::Direction::Up));
        assert!(!editor.in_scrollback());
        assert_eq!(
            editor.document.cursor(),
            Position::new(1, 2),
            "its cursor back"
        );
        assert!(editor.terminal_panel().is_some(), "the panel stays open");

        editor.focus_terminal().unwrap();
        editor.terminal_normal().unwrap();
        editor.switch_alternate();
        assert!(!editor.in_scrollback());
        assert!(!editor.document.is_read_only());

        editor.focus_terminal().unwrap();
        editor.terminal_normal().unwrap();
        editor.close_terminal();
        assert!(!editor.in_scrollback(), "nothing left to show it");
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn a_drag_starts_where_the_screen_shows_it() {
        let mut editor = editor_with_shell("echo first; echo second");
        update_until(&mut editor, |e| shows(e, "second"));
        let rows = editor.terminal_panel().unwrap().screen().contents();
        let row = rows.lines().position(|l| l == "second").expect("on screen") as u16;
        let at = editor.scrollback_at(row, 3).unwrap();
        let line = editor.document.buffer().line(at.row).to_string();
        assert_eq!(line.trim_end(), "second");
        assert_eq!(at.col, 3);
        assert_eq!(editor.document.cursor(), at);
    }
}
