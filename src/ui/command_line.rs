//! The bottom line: the `:`, `/` or `?` prompt in command mode, otherwise
//! the last message.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    text::{Line, Span},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use super::text::{truncate_end, truncate_start};
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
        let offset = 1 + editor.command_line.before_cursor().width();
        let x = (area.x as usize + offset).min(area.right().saturating_sub(1) as usize);
        Position::new(x as u16, area.y)
    }
}

impl Widget for CommandLineView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = if self.editor.mode() == Mode::Command {
            let line = &self.editor.command_line;
            let text = format!("{}{}", line.prompt().symbol(), line.input());
            let mut spans = vec![Span::styled(text, self.theme.get("ui.command-line"))];
            // While cycling completions, show which one this is.
            if let Some(completion) = line.completion() {
                spans.push(Span::styled(
                    format!(
                        "  ({}/{})",
                        completion.index + 1,
                        completion.candidates.len()
                    ),
                    self.theme.get("ui.message.info"),
                ));
            }
            Line::from(spans)
        } else if let Some(message) = self.editor.message() {
            let scope = match message.severity {
                Severity::Info => "ui.message.info",
                Severity::Error => "ui.message.error",
            };
            let text = message_line(
                &message.text,
                message.severity == Severity::Error,
                area.width as usize,
            );
            Line::styled(text, self.theme.get(scope))
        } else {
            return;
        };
        line.render(area, buf);
    }
}

/// What of a message fits the line, `width` wide. An info line keeps its
/// end: for `"path" 12L, 340B written`, the part after a long path is what
/// matters. An error, or a message of several lines, keeps its start (its
/// first line), with where to read all of it: the Output tab has every
/// message whole.
fn message_line(text: &str, error: bool, width: usize) -> String {
    let first = text.lines().next().unwrap_or_default().trim_end();
    let more = text.trim_end().contains('\n');
    if !error && !more {
        return truncate_start(text, width);
    }
    if !more && first.width() <= width {
        return first.to_string();
    }
    const HINT: &str = " … (all of it: :output)";
    let room = width.saturating_sub(HINT.width());
    if room < 10 {
        return truncate_end(first, width);
    }
    let mut kept = String::new();
    let mut used = 0;
    for c in first.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > room {
            break;
        }
        used += w;
        kept.push(c);
    }
    format!("{}{HINT}", kept.trim_end())
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
    fn cursor_follows_the_prompt_cursor_and_completion_count_shows() {
        let mut editor = Editor::default();
        editor.open_command_line();
        "wri".chars().for_each(|c| editor.insert_text_char(c));
        editor.command_line.move_left();
        let pos = CommandLineView::cursor_position(&editor, Rect::new(0, 0, 20, 1));
        assert_eq!(pos, Position::new(3, 0), "between `wr` and `i`");

        editor.command_line.move_end();
        editor
            .command_line
            .start_completion(0, vec!["write".into(), "write_all".into()]);
        assert_eq!(render(&editor).trim_end(), ":write  (1/2)");
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
    fn long_errors_keep_their_start_and_say_where_the_rest_is() {
        let long = "rust-analyzer: cargo check failed to start: Cargo watcher failed";
        assert_eq!(
            message_line(long, true, 50),
            "rust-analyzer: cargo check … (all of it: :output)"
        );
        // Several lines: the first, and where the rest is, even if it fits.
        assert_eq!(
            message_line(
                "Failed to load.\nerror: duplicate key\n --> Cargo.toml:8:2",
                false,
                60
            ),
            "Failed to load. … (all of it: :output)"
        );
        // Short ones as they are; long info lines keep their end.
        assert_eq!(message_line("boom", true, 50), "boom");
        assert_eq!(message_line("abcdefghij", false, 5), "…ghij");
        // Very narrow: just the start.
        assert_eq!(message_line(long, true, 12), "rust-analyz…");
    }

    #[test]
    fn empty_without_message() {
        assert_eq!(render(&Editor::default()).trim(), "");
    }
}
