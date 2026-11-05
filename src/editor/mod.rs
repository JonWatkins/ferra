//! Editor state: the document, current mode, command line and messages.
//!
//! The editor is UI-agnostic. It's mutated by commands and read by the UI.

mod command_line;
mod document;
mod message;
mod mode;

pub use command_line::CommandLine;
pub use document::Document;
pub use message::{Message, Severity};
pub use mode::Mode;

use crate::config::EditorOptions;

#[derive(Debug, Default)]
pub struct Editor {
    pub document: Document,
    pub command_line: CommandLine,
    pub options: EditorOptions,
    mode: Mode,
    message: Option<Message>,
    should_quit: bool,
}

impl Editor {
    pub fn new(options: EditorOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    /// An editor with default options editing `document`.
    pub fn with_document(document: Document) -> Self {
        Self {
            document,
            ..Self::default()
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Switches mode, applying Vim's transition rules.
    pub fn set_mode(&mut self, mode: Mode) {
        let previous = std::mem::replace(&mut self.mode, mode);

        if previous == Mode::Insert && mode == Mode::Normal {
            // Like Vim, leaving insert mode steps back onto the last typed char.
            let cursor = self.document.cursor();
            self.document
                .set_cursor(cursor.with_col(cursor.col.saturating_sub(1)));
        }
        if previous == Mode::Command || mode == Mode::Command {
            self.command_line.clear();
        }
        self.enforce_cursor_rules();
    }

    /// Normal mode may not rest on the end-of-line slot.
    pub fn enforce_cursor_rules(&mut self) {
        if self.mode == Mode::Normal {
            self.document.clamp_to_last_char();
        }
    }

    /// Inserts typed text into whatever has focus in the current mode.
    pub fn insert_text_char(&mut self, c: char) {
        match self.mode {
            Mode::Insert => self.document.insert_char(c),
            Mode::Command => self.command_line.push(c),
            Mode::Normal => {}
        }
    }

    pub fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    pub fn set_message(&mut self, message: Message) {
        self.message = Some(message);
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position;

    #[test]
    fn leaving_insert_mode_steps_cursor_back() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.insert_text_char('a');
        editor.insert_text_char('b');
        editor.set_mode(Mode::Normal);
        assert_eq!(editor.document.cursor(), Position::new(0, 1));
    }

    #[test]
    fn entering_and_leaving_command_mode_clears_input() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Command);
        editor.insert_text_char('q');
        assert_eq!(editor.command_line.input(), "q");
        editor.set_mode(Mode::Normal);
        assert!(editor.command_line.is_empty());
    }

    #[test]
    fn typing_in_normal_mode_does_nothing() {
        let mut editor = Editor::default();
        editor.insert_text_char('x');
        assert_eq!(editor.document.buffer().text(), "");
    }
}
