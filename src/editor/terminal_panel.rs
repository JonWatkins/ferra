//! The terminal panel at the bottom (`<Space>t`, `:terminal`), as in VS
//! Code: a shell (`crate::terminal`) under the windows. `<Space>t` opens
//! it and gives it focus, starting the shell the first time; with focus,
//! keys go to the shell except `[keys.terminal]`'s (`<C-\><C-n>` back to
//! the editor, leaving it open; `<C-`>` hides it, the shell still running).

use std::path::Path;

use super::{Editor, Message, Mode};
use crate::input::KeyPress;
use crate::terminal::{Terminal, shell_program};

impl Editor {
    /// The panel's terminal, if the panel's open.
    pub fn terminal_panel(&self) -> Option<&Terminal> {
        self.terminal.as_ref().filter(|_| self.terminal_open)
    }

    /// Whether a shell is running (so its output is waited for).
    pub fn terminal_running(&self) -> bool {
        self.terminal.as_ref().is_some_and(Terminal::is_running)
    }

    /// `<Space>t`: opens the panel with focus (starting the shell if it
    /// isn't running); with focus already there, hides it.
    pub fn toggle_terminal(&mut self) -> Result<(), String> {
        if self.mode() == Mode::Terminal {
            self.terminal_open = false;
            self.set_mode(Mode::Normal);
            return Ok(());
        }
        self.focus_terminal()
    }

    /// Opens the panel and gives it focus, starting a shell if there's
    /// none (or the last one exited).
    pub fn focus_terminal(&mut self) -> Result<(), String> {
        if !self.terminal_running() {
            self.terminal = None;
            let program = shell_program(&self.options.shell);
            let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new("/").to_path_buf());
            let rows = self.options.terminal_height.max(3);
            let terminal =
                Terminal::spawn(&program, &[], &cwd, rows, 80).map_err(|e| e.to_string())?;
            self.terminal = Some(terminal);
        }
        self.terminal_open = true;
        self.set_mode(Mode::Terminal);
        Ok(())
    }

    /// `<C-\><C-n>`: back to the editor, leaving the panel open.
    pub fn leave_terminal(&mut self) {
        if self.mode() == Mode::Terminal {
            self.set_mode(Mode::Normal);
        }
    }

    /// `:terminal-close`: ends the shell and closes the panel.
    pub fn close_terminal(&mut self) {
        self.terminal = None;
        self.terminal_open = false;
        if self.mode() == Mode::Terminal {
            self.set_mode(Mode::Normal);
        }
    }

    /// Keys typed into the panel. Once the shell has exited, a key closes
    /// the panel.
    pub fn terminal_keys(&mut self, keys: &[KeyPress]) {
        let Some(terminal) = self.terminal.as_mut() else {
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
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.paste(text);
        }
    }

    /// `<S-PageUp>` / `<S-PageDown>`: through what's scrolled off the top.
    pub fn terminal_scroll(&mut self, pages: isize) {
        let rows = self.options.terminal_height.max(3) as isize;
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.scroll(pages * (rows - 1));
        }
    }

    /// Takes the shell's output. Returns whether there's anything new to
    /// draw. A shell that exits while the panel is hidden closes it.
    pub fn pump_terminal(&mut self) -> bool {
        let Some(terminal) = self.terminal.as_mut() else {
            return false;
        };
        let was_running = terminal.is_running();
        let changed = terminal.pump();
        if was_running && !terminal.is_running() && !self.terminal_open {
            let why = terminal.exited().unwrap_or("exited").to_string();
            self.terminal = None;
            self.set_message(Message::info(format!("The terminal's shell {why}")));
        }
        changed
    }

    /// Resizes the shell's terminal to the size the panel was drawn at.
    pub fn apply_terminal_size(&mut self) {
        if let Some(terminal) = self.terminal.as_mut() {
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

    fn pump_until(editor: &mut Editor, done: impl Fn(&Editor) -> bool) {
        let start = Instant::now();
        while !done(editor) && start.elapsed() < Duration::from_secs(3) {
            editor.pump_terminal();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn opens_hides_and_refocuses_the_same_shell() {
        let mut editor = editor_with_shell("echo ready");
        assert_eq!(editor.mode(), Mode::Terminal);
        pump_until(&mut editor, |e| {
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
        pump_until(&mut editor, |e| !e.terminal_running());
        assert!(editor.terminal_panel().unwrap().exited().is_some());
        editor.terminal_keys(&crate::input::parse_keys("x").unwrap());
        assert!(editor.terminal_panel().is_none());
        assert_eq!(editor.mode(), Mode::Normal);
    }
}
