//! The bottom line: the `:` prompt in command mode, otherwise the last message.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    text::Line,
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use crate::config::Theme;
use crate::editor::{Editor, Mode, Severity};

const PROMPT: &str = ":";

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
        let offset = PROMPT.width() + editor.command_line.input().width();
        let x = (area.x as usize + offset).min(area.right().saturating_sub(1) as usize);
        Position::new(x as u16, area.y)
    }
}

impl Widget for CommandLineView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = if self.editor.mode() == Mode::Command {
            let text = format!("{PROMPT}{}", self.editor.command_line.input());
            Line::styled(text, self.theme.get("ui.command-line"))
        } else if let Some(message) = self.editor.message() {
            let scope = match message.severity {
                Severity::Info => "ui.message.info",
                Severity::Error => "ui.message.error",
            };
            Line::styled(message.text.as_str(), self.theme.get(scope))
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
    fn shows_message_otherwise() {
        let mut editor = Editor::default();
        editor.set_message(Message::error("boom"));
        assert_eq!(render(&editor).trim_end(), "boom");
    }

    #[test]
    fn empty_without_message() {
        assert_eq!(render(&Editor::default()).trim(), "");
    }
}
