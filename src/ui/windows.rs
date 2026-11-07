//! Drawing split windows: each window's editor pane, the separators
//! between them, and (with `statusline = "windows"`) each one's status line.
//!
//! A window's status line goes in the row right below it. That row is
//! always either the separator below a stacked window or the screen's
//! status row, so per-window status lines take no extra space: they replace
//! the horizontal separators.

use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Position, Rect},
};

use super::{EditorView, StatusLine, ViewState};
use crate::config::Theme;
use crate::editor::windows::Area;
use crate::editor::{DocumentId, Editor, WindowId};
use crate::syntax::SyntaxEngine;

/// Scroll state per window, and per buffer within a window, so switching a
/// window between buffers keeps each one's place.
pub type Views = HashMap<(WindowId, DocumentId), ViewState>;

/// Per-window status lines: the keys typed so far (for the active window's
/// line) and the screen's status row, below the windows.
pub struct WindowStatus<'a> {
    pub pending: &'a str,
    pub bottom_row: Rect,
}

/// Draws every window into `area` and returns where the terminal cursor
/// goes (in the active window).
pub fn draw_windows(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    area: Rect,
    views: &mut Views,
    mut syntax: Option<&mut SyntaxEngine>,
    status: Option<WindowStatus>,
) -> Option<Position> {
    let windows = editor.windows();
    let area_cells = Area::new(area.x, area.y, area.width, area.height);
    // Resizing works in cells, so the editor needs to know the real size.
    windows.note_area(area_cells);
    let (placed, separators) = windows.layout().arrange(area_cells);

    let separator_style = theme.get("ui.window.separator");
    let screen = frame.area();
    if let Some(status) = &status {
        // The status rows' background, including the cells between
        // side-by-side windows' status lines.
        frame.buffer_mut().set_style(
            status.bottom_row.intersection(screen),
            theme.get("ui.statusline"),
        );
    }
    for separator in separators {
        let a = separator.area;
        if status.is_some() && !separator.vertical {
            let rect = Rect::new(a.x, a.y, a.width, a.height).intersection(screen);
            frame
                .buffer_mut()
                .set_style(rect, theme.get("ui.statusline"));
            continue;
        }
        let symbol = if separator.vertical { "│" } else { "─" };
        for y in a.y..a.y + a.height {
            for x in a.x..a.x + a.width {
                if !screen.contains(Position::new(x, y)) {
                    continue;
                }
                frame.buffer_mut()[(x, y)]
                    .set_symbol(symbol)
                    .set_style(separator_style);
            }
        }
    }

    let mut cursor = None;
    for (id, window_area) in placed {
        let Some((document, window_cursor)) = editor.window_view(id) else {
            continue;
        };
        let active = id == windows.active();
        let rect = Rect::new(
            window_area.x,
            window_area.y,
            window_area.width,
            window_area.height,
        );
        let state = views.entry((id, document.id())).or_default();
        if let Some(top) = windows.take_requested_top(id) {
            let last = document.buffer().line_count().saturating_sub(1);
            state.viewport.top = top.min(last);
        }
        let highlights = syntax.as_deref_mut().and_then(|engine| {
            let rows = state.visible_rows(document, window_cursor, rect.height, &editor.options);
            engine.highlight(document, &editor.languages, &editor.syntax_trees, rows)
        });

        let view = EditorView::new(document, &editor.options, theme)
            .cursor(window_cursor)
            .active(active)
            .selection(if active {
                editor.selection_range()
            } else {
                None
            })
            .block(editor.selection_block().filter(|_| active))
            .search(editor.search_highlight())
            .syntax(highlights);
        frame.render_stateful_widget(view, rect, state);
        windows.note_top(id, state.viewport.top);
        if active {
            cursor = state.cursor;
        }

        if let Some(status) = &status {
            let line = Rect::new(rect.x, rect.y + rect.height, rect.width, 1).intersection(screen);
            if line.height > 0 && line.width > 0 {
                let widget = if active {
                    StatusLine::new(editor, theme, status.pending)
                } else {
                    StatusLine::inactive(document, window_cursor, theme)
                };
                frame.render_widget(widget, line);
            }
        }
    }
    cursor
}
