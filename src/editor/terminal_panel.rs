//! The terminal panel at the bottom (`<Space>t`, `:terminal`), as in VS
//! Code: shells (`crate::terminal`) under the windows, one shown at a time.
//! `<Space>t` opens it and gives it focus, starting a shell the first time;
//! with focus, keys go to the shell except `[keys.terminal]`'s (`<C-\><C-n>`
//! back to the editor, leaving it open; `<C-`>` hides it, the shells still
//! running).
//!
//! More than one shell (`:terminal-new`, ``Ctrl+Shift+` ``): the editor keeps
//! them in order (`Shells`) with the one shown; `:terminal-next` and
//! `:terminal-prev` (`<C-PageDown>` `<C-PageUp>`) go through them, and with
//! two or more the panel lists them down its right side (`ui/terminal.rs`),
//! a click on one showing it. `:terminal-close` ends the one shown and shows
//! the one after it; the panel closes with the last. A shell that exits while
//! shown waits for a key to close it; one that exits while it isn't shown
//! (the panel hidden, or another shown) goes at once, with a message.
//!
//! The editor owns the shells and whether the panel's open; hiding it keeps
//! them. Ending one (closing it, or quitting) ends what it started too, or
//! only hangs up, as `terminal-close` says (`crate::terminal`). `Mode::Terminal` has focus there.
//! In it the input layer returns `Outcome::PassThrough` with the keys of any
//! sequence that isn't bound in `[keys.terminal]` (including a bound prefix
//! that goes no further, or times out), so only those bindings are kept from
//! the shell. Without the kitty protocol crossterm reads Ctrl+\ ] ^ _ as Ctrl+4
//! to 7; `KeyPress::from_event` maps them back.

use std::path::Path;

use super::{Editor, Message, Mode};
use crate::config::TerminalClose;
use crate::input::KeyPress;
use crate::terminal::{Terminal, shell_program};

/// The panel's shells, in order, and which is shown.
#[derive(Debug, Default)]
pub struct Shells {
    list: Vec<Terminal>,
    /// The one shown (an index into `list`, when there's any).
    shown: usize,
}

impl Shells {
    pub fn all(&self) -> &[Terminal] {
        &self.list
    }

    /// Which is shown.
    pub fn shown_index(&self) -> usize {
        self.shown
    }

    pub fn shown(&self) -> Option<&Terminal> {
        self.list.get(self.shown)
    }

    fn shown_mut(&mut self) -> Option<&mut Terminal> {
        self.list.get_mut(self.shown)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Adds `terminal` after the others, and shows it.
    fn push(&mut self, terminal: Terminal) {
        self.list.push(terminal);
        self.shown = self.list.len() - 1;
    }

    /// Ends shell `index`; the one after it is shown if it was (or the one
    /// before, if it was last), else the same one stays shown.
    fn remove(&mut self, index: usize) -> Option<Terminal> {
        if index >= self.list.len() {
            return None;
        }
        let terminal = self.list.remove(index);
        if index < self.shown || self.shown >= self.list.len() {
            self.shown = self.shown.saturating_sub(1);
        }
        Some(terminal)
    }
}

impl Editor {
    /// The panel's terminal shown, if the panel's open at it.
    pub fn terminal_panel(&self) -> Option<&Terminal> {
        self.terminals
            .shown()
            .filter(|_| self.panel_open && self.panel.tab == super::panel::PanelTab::Terminal)
    }

    /// The panel's shells, and which is shown.
    pub fn terminals(&self) -> &Shells {
        &self.terminals
    }

    /// Whether a shell is running (so its output is waited for).
    pub fn terminal_running(&self) -> bool {
        self.terminals.all().iter().any(Terminal::is_running)
    }

    /// `<Space>t`: opens the panel with focus (starting the shell if it
    /// isn't running); with focus already there, hides it.
    pub fn toggle_terminal(&mut self) -> Result<(), String> {
        if self.mode() == Mode::Terminal {
            self.panel_open = false;
            self.set_mode(Mode::Normal);
            return Ok(());
        }
        self.focus_terminal()
    }

    /// Opens the panel at the shell shown and gives it focus, starting a
    /// shell if there's none (one that's exited is closed first).
    pub fn focus_terminal(&mut self) -> Result<(), String> {
        if self.terminals.shown().is_some_and(|t| !t.is_running()) {
            self.terminals.remove(self.terminals.shown);
        }
        if self.terminals.is_empty() {
            let terminal = self.spawn_shell()?;
            self.terminals.push(terminal);
        }
        self.show_terminal_panel();
        Ok(())
    }

    /// `:terminal-new`: another shell, shown and with focus.
    pub fn new_terminal(&mut self) -> Result<(), String> {
        let terminal = self.spawn_shell()?;
        self.terminals.push(terminal);
        self.show_terminal_panel();
        Ok(())
    }

    /// Shows shell `index` (from 0), with focus (a click in the list).
    pub fn select_terminal(&mut self, index: usize) -> Result<(), String> {
        if index >= self.terminals.len() {
            return Err(format!("There's no terminal {}", index + 1));
        }
        self.terminals.shown = index;
        self.focus_terminal()
    }

    /// `:terminal-next` / `:terminal-prev`: the next or previous shell,
    /// round from the last to the first.
    pub fn cycle_terminal(&mut self, delta: isize) -> Result<(), String> {
        let count = self.terminals.len();
        if count == 0 {
            return self.focus_terminal();
        }
        let at = self.terminals.shown as isize + delta;
        self.terminals.shown = at.rem_euclid(count as isize) as usize;
        self.focus_terminal()
    }

    /// The shell to start, at the project's root, the panel's height.
    fn spawn_shell(&self) -> Result<Terminal, String> {
        let program = shell_program(&self.options.shell);
        let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new("/").to_path_buf());
        let rows = self.options.terminal_height.max(3);
        let mut terminal =
            Terminal::spawn(&program, &[], &cwd, rows, 80).map_err(|e| e.to_string())?;
        terminal.set_end_session(self.options.terminal_close == TerminalClose::Kill);
        Ok(terminal)
    }

    /// `terminal-close` changed: the shells running take it too.
    pub(super) fn apply_terminal_close(&mut self, close: TerminalClose) {
        for terminal in &mut self.terminals.list {
            terminal.set_end_session(close == TerminalClose::Kill);
        }
    }

    fn show_terminal_panel(&mut self) {
        self.panel_open = true;
        self.panel.tab = super::panel::PanelTab::Terminal;
        self.set_mode(Mode::Terminal);
    }

    /// `<C-\><C-n>`: back to the editor, leaving the panel open.
    pub fn leave_terminal(&mut self) {
        if self.mode() == Mode::Terminal {
            self.set_mode(Mode::Normal);
        }
    }

    /// `:terminal-close`: ends the shell shown, showing the one after it;
    /// with the last, closes the panel.
    pub fn close_terminal(&mut self) {
        self.terminals.remove(self.terminals.shown);
        if self.terminals.is_empty() {
            self.panel_open = false;
            if self.mode() == Mode::Terminal {
                self.set_mode(Mode::Normal);
            }
        }
    }

    /// Keys typed into the panel. Once the shell has exited, a key closes
    /// it.
    pub fn terminal_keys(&mut self, keys: &[KeyPress]) {
        let Some(terminal) = self.terminals.shown_mut() else {
            return;
        };
        if !terminal.is_running() {
            self.close_terminal();
            return;
        }
        for &key in keys {
            terminal.send_key(key);
        }
    }

    /// Text pasted into the panel.
    pub fn terminal_paste(&mut self, text: &str) {
        if let Some(terminal) = self.terminals.shown_mut() {
            terminal.paste(text);
        }
    }

    /// `<S-PageUp>` / `<S-PageDown>`: through what's scrolled off the top.
    pub fn terminal_scroll(&mut self, pages: isize) {
        let rows = self.options.terminal_height.max(3) as isize;
        if let Some(terminal) = self.terminals.shown_mut() {
            terminal.scroll(pages * (rows - 1));
        }
    }

    /// The mouse wheel over the panel: `rows` back (or forward if negative).
    pub fn terminal_scroll_rows(&mut self, rows: isize) {
        if let Some(terminal) = self.terminals.shown_mut() {
            terminal.scroll(rows);
        }
    }

    /// Takes the shells' output. Returns whether there's anything new to
    /// draw. A shell that exits while it isn't shown closes.
    pub fn update_terminal(&mut self) -> bool {
        let shown = self
            .terminal_panel()
            .is_some()
            .then_some(self.terminals.shown);
        let mut changed = false;
        let mut gone = Vec::new();
        for (i, terminal) in self.terminals.list.iter_mut().enumerate() {
            let was_running = terminal.is_running();
            changed |= terminal.update();
            self.terminal_bell |= terminal.take_bell() && self.options.terminal_bell;
            if was_running && !terminal.is_running() && shown != Some(i) {
                gone.push(i);
            }
        }
        for &i in gone.iter().rev() {
            let Some(terminal) = self.terminals.remove(i) else {
                continue;
            };
            let why = terminal.exited().unwrap_or("exited").to_string();
            let which = match self.terminals.len() {
                0 => "The terminal's shell".to_string(),
                _ => format!("A terminal's shell ({})", terminal.title()),
            };
            self.set_message(Message::info(format!("{which} {why}")));
            changed = true;
        }
        changed
    }

    /// Whether a shell rang the bell since last asked, with `terminal-bell`
    /// on (any shell, shown or not: a long job's bell says it's done).
    pub fn take_terminal_bell(&mut self) -> bool {
        std::mem::take(&mut self.terminal_bell)
    }

    /// The cursor shape the shown shell's program asked for, and whether
    /// it blinks, while the panel has focus (else `cursor-shape`'s).
    pub fn terminal_cursor_shape(&self) -> Option<(crate::config::CursorShape, bool)> {
        if self.mode() != Mode::Terminal {
            return None;
        }
        self.terminal_panel()?.cursor_shape()
    }

    /// Resizes the shells' terminals to the size the panel was drawn at.
    pub fn apply_terminal_size(&mut self) {
        for terminal in &mut self.terminals.list {
            terminal.apply_size();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn editor_with_shell(script: &str) -> Editor {
        let mut editor = Editor::default();
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

    #[test]
    fn the_agent_reads_the_scrollback_and_sees_whats_running() {
        let mut editor = editor_with_shell(
            "i=1; while [ $i -le 50 ]; do echo line$i; i=$((i+1)); done; sleep 3",
        );
        update_until(&mut editor, |e| {
            e.terminals
                .shown()
                .and_then(Terminal::running_command)
                .is_some_and(|c| c.contains("sleep"))
                && e.terminals
                    .shown()
                    .is_some_and(|t| t.text_lines().iter().any(|l| l == "line50"))
        });
        let all = editor.agent_read_terminal(None).unwrap();
        assert!(all.contains("running `sleep 3`"), "{all}");
        // Scrolled well off a 12-row panel, and in order.
        let first = all
            .find("line1\n")
            .unwrap_or_else(|| panic!("the first line: {all}"));
        assert!(first < all.find("line50\n").unwrap(), "{all}");
        assert!(!all.contains("line50\nline50"), "no row twice: {all}");

        let last = editor.agent_read_terminal(Some(3)).unwrap();
        assert!(last.contains("(ask for more with lines)"), "{last}");
        assert!(!last.contains("line47"), "only the last 3: {last}");
        assert!(last.contains("line50"), "{last}");

        let state = editor.agent_editor_state();
        assert!(
            state.contains("Terminal panel (shown, titled") && state.contains("read_terminal"),
            "{state}"
        );
    }

    #[test]
    fn a_status_taller_than_the_panel_leaves_no_copies_for_the_agent() {
        // As log-update does: draw the status, then up over it, clear, and
        // draw it again. 15 rows don't fit in the 12-row panel, so the top
        // of each copy has scrolled off before it can be cleared.
        let mut editor = editor_with_shell(
            "for i in 1 2 3 4 5 6; do [ $i -gt 1 ] && printf '\\033[15A\\033[J'; \
             k=1; while [ $k -le 15 ]; do echo \"w$k $i/6\"; k=$((k+1)); done; done; echo finished",
        );
        update_until(&mut editor, |e| {
            e.terminals
                .shown()
                .is_some_and(|t| t.text_lines().iter().any(|l| l == "finished"))
        });
        let lines = editor.terminals.shown().unwrap().text_lines();
        let copies = lines.iter().filter(|l| l.starts_with("w1 ")).count();
        assert!(
            copies > 1,
            "the panel itself keeps the leftovers: {lines:?}"
        );

        let read = editor.agent_read_terminal(None).unwrap();
        assert!(read.contains("progress redraws left out"), "{read}");
        assert_eq!(read.matches("w1 ").count(), 1, "{read}");
        assert!(
            read.contains("w1 6/6") && read.contains("w15 6/6"),
            "{read}"
        );
    }

    fn shown_text_has(editor: &Editor, text: &str) -> bool {
        editor
            .terminal_panel()
            .is_some_and(|t| t.screen().contents().contains(text))
    }

    #[test]
    fn the_programs_cursor_shape_is_used_while_the_panel_has_focus() {
        let mut editor = editor_with_shell("echo none");
        update_until(&mut editor, |e| shown_text_has(e, "none\n"));
        assert_eq!(editor.terminal_cursor_shape(), None, "cursor-shape's");

        // As Vim does going into insert mode: a blinking bar.
        let keys = crate::input::parse_keys("printf '\\033[5 q'; echo bar<CR>").unwrap();
        editor.terminal_keys(&keys);
        update_until(&mut editor, |e| shown_text_has(e, "bar\n"));
        assert_eq!(
            editor.terminal_cursor_shape(),
            Some((crate::config::CursorShape::Bar, true))
        );
        editor.leave_terminal();
        assert_eq!(editor.terminal_cursor_shape(), None, "the editor's own");
        editor.focus_terminal().unwrap();

        // And back to the default.
        let keys = crate::input::parse_keys("printf '\\033[0 q'; echo back<CR>").unwrap();
        editor.terminal_keys(&keys);
        update_until(&mut editor, |e| shown_text_has(e, "back\n"));
        assert_eq!(editor.terminal_cursor_shape(), None);
    }

    #[test]
    fn the_bell_is_passed_on_only_with_terminal_bell() {
        let mut editor = editor_with_shell("printf '\\a'; echo once");
        update_until(&mut editor, |e| shown_text_has(e, "once\n"));
        assert!(!editor.take_terminal_bell(), "off by default");

        editor.options.terminal_bell = true;
        let keys = crate::input::parse_keys("printf '\\a'; echo twice<CR>").unwrap();
        editor.terminal_keys(&keys);
        update_until(&mut editor, |e| shown_text_has(e, "twice\n"));
        assert!(editor.take_terminal_bell());
        assert!(!editor.take_terminal_bell(), "once per ring");
    }

    #[test]
    fn opens_hides_and_refocuses_the_same_shell() {
        let mut editor = editor_with_shell("echo ready");
        assert_eq!(editor.mode(), Mode::Terminal);
        update_until(&mut editor, |e| {
            e.terminal_panel()
                .is_some_and(|t| t.screen().contents().contains("ready\n"))
        });
        editor.leave_terminal();
        assert_eq!(editor.mode(), Mode::Normal);
        assert!(editor.terminal_panel().is_some(), "still open");

        editor.toggle_terminal().unwrap();
        assert_eq!(editor.mode(), Mode::Terminal, "focused again");
        editor.toggle_terminal().unwrap();
        assert!(editor.terminal_panel().is_none(), "hidden");
        assert!(editor.terminal_running(), "the shell's still there");
        editor.toggle_terminal().unwrap();
        let text = editor.terminal_panel().unwrap().screen().contents();
        assert!(text.contains("ready"), "same shell: {text:?}");
    }

    #[test]
    fn a_key_after_the_shell_exits_closes_the_panel() {
        let mut editor = editor_with_shell("exit");
        update_until(&mut editor, |e| !e.terminal_running());
        assert!(editor.terminal_panel().unwrap().exited().is_some());
        editor.terminal_keys(&crate::input::parse_keys("x").unwrap());
        assert!(editor.terminal_panel().is_none());
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn new_shells_are_shown_in_turn_and_closing_shows_the_next() {
        let mut editor = editor_with_shell("echo one");
        editor.new_terminal().unwrap();
        editor.new_terminal().unwrap();
        assert_eq!(editor.terminals().len(), 3);
        assert_eq!(editor.terminals().shown_index(), 2, "the new one's shown");
        assert_eq!(editor.mode(), Mode::Terminal);
        let state = editor.agent_editor_state();
        assert!(
            state.contains("shell 3 of 3; the others aren't read"),
            "{state}"
        );

        editor.cycle_terminal(1).unwrap();
        assert_eq!(editor.terminals().shown_index(), 0, "round to the first");
        editor.cycle_terminal(-1).unwrap();
        assert_eq!(editor.terminals().shown_index(), 2, "and back");
        editor.select_terminal(1).unwrap();
        assert_eq!(editor.terminals().shown_index(), 1);
        assert!(editor.select_terminal(5).is_err());

        // Closing the middle one shows the one after it; the last, the one
        // before.
        editor.close_terminal();
        assert_eq!(editor.terminals().len(), 2);
        assert_eq!(editor.terminals().shown_index(), 1);
        editor.close_terminal();
        assert_eq!(editor.terminals().shown_index(), 0);
        assert!(
            editor.terminal_panel().is_some(),
            "still open, at the first"
        );
        update_until(&mut editor, |e| {
            e.terminal_panel()
                .is_some_and(|t| t.screen().contents().contains("one"))
        });
        editor.close_terminal();
        assert!(editor.terminal_panel().is_none(), "closed with the last");
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn a_shell_that_exits_while_another_is_shown_goes() {
        let mut editor = editor_with_shell("sleep 0.3; exit");
        editor.new_terminal().unwrap();
        update_until(&mut editor, |e| e.terminals().len() == 1);
        assert_eq!(editor.terminals().len(), 1, "the first one's gone");
        assert_eq!(
            editor.terminals().shown_index(),
            0,
            "the second's still shown"
        );
        assert!(editor.terminal_panel().unwrap().is_running());
        let message = editor.message().map(|m| m.text.clone()).unwrap_or_default();
        assert!(message.contains("exited"), "{message}");
    }

    #[test]
    fn a_key_after_the_shell_shown_exits_closes_only_it() {
        let mut editor = editor_with_shell("echo first");
        editor.new_terminal().unwrap();
        let keys = crate::input::parse_keys("exit<CR>").unwrap();
        editor.terminal_keys(&keys);
        update_until(&mut editor, |e| {
            e.terminal_panel().is_some_and(|t| !t.is_running())
        });
        assert_eq!(editor.terminals().len(), 2, "waits for a key");
        editor.terminal_keys(&crate::input::parse_keys("x").unwrap());
        assert_eq!(editor.terminals().len(), 1);
        assert!(
            editor.terminal_panel().unwrap().is_running(),
            "the first, shown"
        );
        assert_eq!(editor.mode(), Mode::Terminal);
    }
}
