//! The bottom line: the `:`, `/` or `?` prompt in command mode, otherwise
//! the last message.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    text::Line,
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use super::text::truncate_start;
use crate::config::Theme;
use crate::editor::{Editor, Mode, Severity};

pub struct CommandLineView<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
}

impl<'a> CommandLineView<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme) -> Self {
        Self { editor, theme }
    }

    /// Where the terminal cursor goes while typing a command.
    pub fn cursor_position(editor: &Editor, area: Rect) -> Position {
        // The prompt symbol is one cell wide.
        let offset = 1 + editor.command_line.input().width();
        let x = (area.x as usize + offset).min(area.right().saturating_sub(1) as usize);
        Position::new(x as u16, area.y)
    }
}

impl Widget for CommandLineView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = if self.editor.mode() == Mode::Command {
            let line = &self.editor.command_line;
            let text = format!("{}{}", line.prompt().symbol(), line.input());
            Line::styled(text, self.theme.get("ui.command-line"))
        } else if let Some(message) = self.editor.message() {
            let scope = match message.severity {
                Severity::Info => "ui.message.info",
                Severity::Error => "ui.message.error",
            };
            // Keep the end visible: for "path" 12L, 340B written, the part
            // after a long path is what matters.
            let text = truncate_start(&message.text, area.width as usize);
            Line::styled(text, self.theme.get(scope))
        } else {
            return;
        };
        line.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Message;

    fn render(editor: &Editor) -> String {
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        CommandLineView::new(editor, &Theme::default()).render(area, &mut buf);
        (0..area.width)
            .map(|x| buf[(x, 0)].symbol())
            .collect::<String>()
    }

    #[test]
    fn shows_prompt_and_input_in_command_mode() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Command);
        editor.insert_text_char('q');
        assert_eq!(render(&editor).trim_end(), ":q");
        let pos = CommandLineView::cursor_position(&editor, Rect::new(0, 5, 20, 1));
        assert_eq!(pos, Position::new(2, 5));
    }

    #[test]
    fn shows_search_prompt_symbol() {
        let mut editor = Editor::default();
        editor.open_search(false);
        editor.insert_text_char('x');
        assert_eq!(render(&editor).trim_end(), "?x");
    }

    #[test]
    fn shows_message_otherwise() {
        let mut editor = Editor::default();
        editor.set_message(Message::error("boom"));
        assert_eq!(render(&editor).trim_end(), "boom");
    }

    #[test]
    fn long_messages_keep_their_end() {
        let mut editor = Editor::default();
        editor.set_message(Message::info(
            "\"/a/very/long/path/to/file.txt\" 3L written",
        ));
        // The test area is 20 cells wide.
        assert_eq!(render(&editor), "…ile.txt\" 3L written");
    }

    #[test]
    fn empty_without_message() {
        assert_eq!(render(&Editor::default()).trim(), "");
    }
}
