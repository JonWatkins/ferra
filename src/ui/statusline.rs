//! The status line: mode indicator, document name, pending keys and position.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

use unicode_width::UnicodeWidthStr;

use super::text::truncate_start;
use crate::config::Theme;
use crate::editor::{Editor, Mode, SelectionKind};

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

        let cursor = self.editor.document.cursor();
        let language = self
            .editor
            .document
            .path()
            .and_then(crate::syntax::detect)
            .map(|l| format!(" {} ", l.name))
            .unwrap_or_default();
        let recording = self
            .editor
            .recording_macro()
            .map(|r| format!("recording @{r}  "))
            .unwrap_or_default();
        let right = Line::from(vec![
            Span::styled(recording, self.theme.get("ui.statusline.recording")),
            Span::styled(
                format!("{} ", self.pending),
                self.theme.get("ui.statusline.pending"),
            ),
            Span::raw(language),
            Span::raw(format!(" {}:{} ", cursor.row + 1, cursor.col + 1)),
        ])
        .right_aligned();

        let mode = self.editor.mode();
        let mode_label = match self.editor.selection() {
            _ if mode == Mode::Command && self.editor.command_line.prompt().is_search() => {
                " SEARCH ".to_string()
            }
            Some(selection) if selection.kind == SelectionKind::Line => " V-LINE ".to_string(),
            Some(selection) if selection.kind == SelectionKind::Block => " V-BLOCK ".to_string(),
            _ => format!(" {mode} "),
        };
        let modified = if self.editor.document.is_modified() {
            "[+] "
        } else {
            ""
        };
        // The name gets whatever space is left, so a long path is shortened
        // from the left and the file name and `[+]` stay visible.
        let fixed = mode_label.width() + 2 + modified.width() + right.width();
        let name_budget = (area.width as usize).saturating_sub(fixed);
        let name = truncate_start(&self.editor.document.name(), name_budget);

        let mode_style = self.theme.get(&format!("ui.statusline.{}", mode.id()));
        let left = Line::from(vec![
            Span::styled(mode_label, mode_style),
            Span::raw(format!(" {name} ")),
            Span::styled(modified, self.theme.get("ui.statusline.modified")),
        ]);

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
    fn shows_modified_marker() {
        let mut editor = Editor::default();
        assert!(!render(&editor, "").contains("[+]"));
        editor.document.insert_str("x");
        assert!(render(&editor, "").contains(" [scratch] [+] "));
    }

    #[test]
    fn long_names_are_shortened_from_the_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deeply/nested/directory/structure/file.rs");
        let (document, _) = Document::open(&path).unwrap();
        let mut editor = Editor::with_document(document);
        editor.document.insert_str("x");

        let text = render(&editor, "");
        assert!(text.contains("…"), "{text:?}");
        assert!(text.contains("/file.rs [+]"), "{text:?}");
        assert!(text.contains(" rust "), "language shown: {text:?}");
        assert!(text.ends_with(" 1:2 "), "{text:?}");
    }

    #[test]
    fn shows_visual_and_visual_line() {
        let mut editor = Editor::default();
        editor.toggle_visual(SelectionKind::Char);
        assert!(render(&editor, "").starts_with(" VISUAL "));
        editor.toggle_visual(SelectionKind::Line);
        assert!(render(&editor, "").starts_with(" V-LINE "));
    }

    #[test]
    fn shows_search_label() {
        let mut editor = Editor::default();
        editor.open_search(true);
        assert!(render(&editor, "").starts_with(" SEARCH "));
    }

    #[test]
    fn shows_macro_recording() {
        let mut editor = Editor::default();
        editor.set_recording_macro(Some('q'));
        assert!(render(&editor, "").contains("recording @q"));
    }

    #[test]
    fn shows_pending_keys() {
        let text = render(&Editor::default(), "3g");
        assert!(text.contains("3g"), "{text:?}");
    }
}
