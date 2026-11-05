//! The editor pane: a custom ratatui widget that renders a document with a
//! line-number gutter, cursorline and selection highlights, and a scrolling
//! viewport.
//!
//! Ratatui has no built-in widget suited to editing, so this is our own.
//! Each row is drawn as text, then highlights are layered on as styles;
//! syntax highlighting and diagnostics will layer on the same way.

mod gutter;
mod line;
mod selection;
mod viewport;

pub use viewport::{ScrollTarget, Viewport};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    widgets::StatefulWidget,
};

use crate::config::{EditorOptions, Theme};
use crate::core::Range;
use crate::core::search::SearchPattern;
use crate::editor::Document;
use crate::syntax::{HIGHLIGHT_NAMES, Highlights};

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
    selection: Option<Range>,
    search: Option<&'a SearchPattern>,
    syntax: Option<&'a Highlights>,
}

impl<'a> EditorView<'a> {
    pub fn new(document: &'a Document, options: &'a EditorOptions, theme: &'a Theme) -> Self {
        Self {
            document,
            options,
            theme,
            selection: None,
            search: None,
            syntax: None,
        }
    }

    /// Colours the text with syntax highlights, if any.
    pub fn syntax(mut self, syntax: Option<&'a Highlights>) -> Self {
        self.syntax = syntax;
        self
    }

    /// Highlights matches of `search` (the active or last search, if any).
    pub fn search(mut self, search: Option<&'a SearchPattern>) -> Self {
        self.search = search;
        self
    }

    /// Highlights `selection` (the visual-mode selection, if any).
    pub fn selection(mut self, selection: Option<Range>) -> Self {
        self.selection = selection;
        self
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
        let tab_width = self.options.tab_width;
        let line_numbers = self.options.line_numbers;

        let gutter_width = gutter::width(text.line_count(), line_numbers).min(area.width);
        let [gutter_area, text_area] =
            Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(0)]).areas(area);

        let cursor_col = line::display_col(&text.line(cursor.row), cursor.col, tab_width);
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
        let syntax_styles: Vec<Style> = match self.syntax {
            Some(_) => HIGHLIGHT_NAMES
                .iter()
                .map(|name| self.theme.get(name))
                .collect(),
            None => Vec::new(),
        };
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

            let line_text = &*text.line(row);
            let visible = line::visible_slice(line_text, left, text_area.width as usize, tab_width);
            buf.set_stringn(
                text_area.x,
                y,
                visible,
                text_area.width as usize,
                text_style,
            );

            let mut highlight = |from: usize, to: usize, style: Style| {
                // Char columns to display cells; a column past the last char
                // (a selected line break) is drawn as one extra cell.
                let line_len = text.line_len(row);
                let from = line::display_col(line_text, from, tab_width);
                let to = line::display_col(line_text, to, tab_width) + usize::from(to > line_len);
                let start = from.max(left) - left;
                let end = to.saturating_sub(left).min(text_area.width as usize);
                if start < end {
                    let cells = Rect::new(text_area.x + start as u16, y, (end - start) as u16, 1);
                    buf.set_style(cells, style);
                }
            };

            if let Some(syntax) = self.syntax {
                for span in syntax.line(row) {
                    highlight(span.start, span.end, syntax_styles[span.highlight]);
                }
            }
            if let Some(pattern) = self.search {
                let style = self.theme.get("ui.search.match");
                for (from, to) in pattern.matches_in_line(line_text) {
                    highlight(from, to, style);
                }
            }
            // The selection is drawn last, so it wins over search matches.
            if let Some(range) = self.selection
                && let Some((from, to)) = selection::selected_cols(range, row, text.line_len(row))
            {
                highlight(from, to, self.theme.get("ui.selection"));
            }
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
    fn highlights_selection_including_line_break() {
        let doc = Document::with_text("abc\nde");
        let mut theme = Theme::default();
        let selected = Style::new().bg(ratatui::style::Color::Blue);
        theme.set("ui.selection", selected);
        let range = Range::Chars {
            start: TextPos::new(0, 1),
            end: TextPos::new(1, 1),
        };

        let area = Rect::new(0, 0, 6, 2);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .selection(Some(range))
            .render(area, &mut buf, &mut ViewState::default());

        let highlighted = |y: u16| -> Vec<u16> {
            (0..6)
                .filter(|&x| buf[(x, y)].bg == ratatui::style::Color::Blue)
                .collect()
        };
        assert_eq!(highlighted(0), vec![1, 2, 3], "b, c and the line break");
        assert_eq!(highlighted(1), vec![0], "d");
    }

    #[test]
    fn selection_highlight_respects_horizontal_scroll() {
        let mut doc = Document::with_text("0123456789");
        doc.set_cursor(TextPos::new(0, 9));
        let mut theme = Theme::default();
        theme.set("ui.selection", Style::new().bg(ratatui::style::Color::Blue));
        let range = Range::Chars {
            start: TextPos::new(0, 2),
            end: TextPos::new(0, 8),
        };

        let area = Rect::new(0, 0, 4, 1);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .selection(Some(range))
            .render(area, &mut buf, &mut state);

        // Scrolled so columns 6..=9 are visible; 6 and 7 are selected.
        assert_eq!(state.viewport.left, 6);
        let highlighted: Vec<u16> = (0..4)
            .filter(|&x| buf[(x, 0)].bg == ratatui::style::Color::Blue)
            .collect();
        assert_eq!(highlighted, vec![0, 1]);
    }

    #[test]
    fn highlights_search_matches() {
        let doc = Document::with_text("ab ab");
        let mut theme = Theme::default();
        theme.set(
            "ui.search.match",
            Style::new().bg(ratatui::style::Color::Yellow),
        );
        let pattern = SearchPattern::new("ab", crate::core::search::CaseMode::Smart).unwrap();

        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .search(Some(&pattern))
            .render(area, &mut buf, &mut ViewState::default());

        let highlighted: Vec<u16> = (0..6)
            .filter(|&x| buf[(x, 0)].bg == ratatui::style::Color::Yellow)
            .collect();
        assert_eq!(highlighted, vec![0, 1, 3, 4]);
    }

    #[test]
    fn colours_text_with_syntax_highlights() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "fn main() {}").unwrap();
        let (doc, _) = Document::open(&path).unwrap();
        let mut engine = crate::syntax::SyntaxEngine::default();
        let highlights = engine.highlights(&doc);

        let mut theme = Theme::default();
        theme.set("keyword", Style::new().fg(ratatui::style::Color::Magenta));
        let area = Rect::new(0, 0, 12, 1);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .syntax(highlights)
            .render(area, &mut buf, &mut ViewState::default());

        let magenta: Vec<u16> = (0..12)
            .filter(|&x| buf[(x, 0)].fg == ratatui::style::Color::Magenta)
            .collect();
        assert_eq!(magenta, vec![0, 1], "`fn` is a keyword");
    }

    #[test]
    fn tabs_render_as_spaces_and_the_cursor_follows() {
        let mut doc = Document::with_text("\tx");
        doc.set_cursor(TextPos::new(0, 1));
        let options = EditorOptions {
            tab_width: 4,
            ..options(LineNumbers::None)
        };
        let area = Rect::new(0, 0, 8, 1);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(&doc, &options, &Theme::default()).render(area, &mut buf, &mut state);
        assert_eq!(row_text(&buf, 0), "    x   ");
        assert_eq!(state.cursor, Some(Position::new(4, 0)));
    }

    #[test]
    fn zero_sized_area_does_not_panic() {
        let doc = Document::with_text("x");
        let (_, state) = render(&doc, &options(LineNumbers::Absolute), 0, 0);
        assert_eq!(state.cursor, None);
    }
}
