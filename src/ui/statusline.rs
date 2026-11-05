//! The status line: mode indicator, document name, pending keys and position.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

use crate::config::Theme;
use crate::editor::Editor;

pub struct StatusLine<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
    /// Partially typed count/key sequence, e.g. `"2g"`.
    pending: &'a str,
}

impl<'a> StatusLine<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme, pending: &'a str) -> Self {
        Self {
            editor,
            theme,
            pending,
        }
    }
}

impl Widget for StatusLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let base = self.theme.get("ui.statusline");
        buf.set_style(area, base);

        let mode = self.editor.mode();
        let mode_style = self.theme.get(&format!("ui.statusline.{}", mode.id()));
        let left = Line::from(vec![
            Span::styled(format!(" {mode} "), mode_style),
            Span::raw(format!(" {} ", self.editor.document.name())),
        ]);

        let cursor = self.editor.document.cursor();
        let right = Line::from(vec![
            Span::styled(
                format!("{} ", self.pending),
                self.theme.get("ui.statusline.pending"),
            ),
            Span::raw(format!(" {}:{} ", cursor.row + 1, cursor.col + 1)),
        ])
        .right_aligned();

        left.render(area, buf);
        right.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position;
    use crate::editor::{Document, Mode};

    fn render(editor: &Editor, pending: &str) -> String {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        StatusLine::new(editor, &Theme::default(), pending).render(area, &mut buf);
        (0..area.width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    #[test]
    fn shows_mode_name_and_position() {
        let mut editor = Editor::with_document(Document::with_text("abc\ndef"));
        editor.document.set_cursor(Position::new(1, 2));
        let text = render(&editor, "");
        assert!(text.starts_with(" NORMAL  [scratch]"), "{text:?}");
        assert!(text.ends_with(" 2:3 "), "{text:?}");
    }

    #[test]
    fn shows_current_mode() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        assert!(render(&editor, "").starts_with(" INSERT "));
    }

    #[test]
    fn shows_pending_keys() {
        let text = render(&Editor::default(), "3g");
        assert!(text.contains("3g"), "{text:?}");
    }
}
