//! The editor pane: a custom ratatui widget that renders a document with a
//! line-number gutter, cursorline highlight and scrolling viewport.
//!
//! Ratatui has no built-in widget suited to editing, so this is our own.
//! Per-line rendering is deliberately simple for now; syntax highlighting,
//! selections and diagnostics will layer styled spans onto it later.

mod gutter;
mod line;
mod viewport;

pub use viewport::{ScrollTarget, Viewport};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    widgets::StatefulWidget,
};

use crate::config::{EditorOptions, Theme};
use crate::editor::Document;

/// State kept between frames for an editor pane.
#[derive(Debug, Default, Clone)]
pub struct ViewState {
    pub viewport: Viewport,
    /// Where the terminal cursor should be drawn, set during rendering.
    pub cursor: Option<Position>,
}

pub struct EditorView<'a> {
    document: &'a Document,
    options: &'a EditorOptions,
    theme: &'a Theme,
}

impl<'a> EditorView<'a> {
    pub fn new(document: &'a Document, options: &'a EditorOptions, theme: &'a Theme) -> Self {
        Self {
            document,
            options,
            theme,
        }
    }
}

impl StatefulWidget for EditorView<'_> {
    type State = ViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut ViewState) {
        state.cursor = None;
        if area.is_empty() {
            return;
        }

        let text = self.document.buffer();
        let cursor = self.document.cursor();
        let line_numbers = self.options.line_numbers;

        let gutter_width = gutter::width(text.line_count(), line_numbers).min(area.width);
        let [gutter_area, text_area] =
            Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(0)]).areas(area);

        let cursor_col = line::display_col(text.line(cursor.row), cursor.col);
        let target = ScrollTarget {
            row: cursor.row,
            col: cursor_col,
            line_count: text.line_count(),
        };
        state.viewport.scroll_to(
            target,
            text_area.width as usize,
            text_area.height as usize,
            self.options.scroll_off,
        );
        let Viewport { top, left } = state.viewport;

        let text_style = self.theme.get("ui.text");
        let rows = (top..text.line_count()).take(area.height as usize);
        for (i, row) in rows.enumerate() {
            let y = area.y + i as u16;
            let is_cursor_row = row == cursor.row;

            let row_style = if is_cursor_row && self.options.cursorline {
                self.theme.get("ui.cursorline")
            } else {
                Style::default()
            };
            buf.set_style(Rect::new(area.x, y, area.width, 1), row_style);

            let gutter_scope = if is_cursor_row {
                "ui.linenr.selected"
            } else {
                "ui.linenr"
            };
            gutter::render(
                buf,
                Rect::new(gutter_area.x, y, gutter_area.width, 1),
                &gutter::label(row, cursor.row, line_numbers),
                self.theme.get(gutter_scope),
            );

            let visible = line::visible_slice(text.line(row), left, text_area.width as usize);
            buf.set_stringn(
                text_area.x,
                y,
                visible,
                text_area.width as usize,
                text_style,
            );
        }

        if !text_area.is_empty() {
            state.cursor = Some(Position::new(
                text_area.x + (cursor_col - left) as u16,
                text_area.y + (cursor.row - top) as u16,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LineNumbers;
    use crate::core::Position as TextPos;

    fn render(doc: &Document, options: &EditorOptions, w: u16, h: u16) -> (Buffer, ViewState) {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(doc, options, &Theme::default()).render(area, &mut buf, &mut state);
        (buf, state)
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn options(line_numbers: LineNumbers) -> EditorOptions {
        EditorOptions {
            line_numbers,
            scroll_off: 0,
            ..EditorOptions::default()
        }
    }

    #[test]
    fn renders_gutter_and_text() {
        let doc = Document::with_text("one\ntwo");
        let (buf, _) = render(&doc, &options(LineNumbers::Absolute), 12, 3);
        assert_eq!(row_text(&buf, 0), "   1 one    ");
        assert_eq!(row_text(&buf, 1), "   2 two    ");
        assert_eq!(row_text(&buf, 2), "            ");
    }

    #[test]
    fn renders_without_gutter() {
        let doc = Document::with_text("one");
        let (buf, state) = render(&doc, &options(LineNumbers::None), 5, 1);
        assert_eq!(row_text(&buf, 0), "one  ");
        assert_eq!(state.cursor, Some(Position::new(0, 0)));
    }

    #[test]
    fn cursor_position_accounts_for_gutter() {
        let mut doc = Document::with_text("abc\ndef");
        doc.set_cursor(TextPos::new(1, 2));
        let (_, state) = render(&doc, &options(LineNumbers::Absolute), 12, 3);
        assert_eq!(state.cursor, Some(Position::new(7, 1)));
    }

    #[test]
    fn scrolls_to_keep_cursor_visible() {
        let text = (1..=10)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut doc = Document::with_text(&text);
        doc.set_cursor(TextPos::new(9, 0));
        let (buf, state) = render(&doc, &options(LineNumbers::None), 4, 3);
        assert_eq!(row_text(&buf, 2), "10  ");
        assert_eq!(state.viewport.top, 7);
        assert_eq!(state.cursor, Some(Position::new(0, 2)));
    }

    #[test]
    fn cursor_accounts_for_wide_chars() {
        let mut doc = Document::with_text("日本");
        doc.set_cursor(TextPos::new(0, 1));
        let (_, state) = render(&doc, &options(LineNumbers::None), 10, 1);
        assert_eq!(state.cursor, Some(Position::new(2, 0)));
    }

    #[test]
    fn zero_sized_area_does_not_panic() {
        let doc = Document::with_text("x");
        let (_, state) = render(&doc, &options(LineNumbers::Absolute), 0, 0);
        assert_eq!(state.cursor, None);
    }
}
