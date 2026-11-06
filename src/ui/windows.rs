//! Drawing split windows: each window's editor pane, and the separators
//! between them.

use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Position, Rect},
};

use super::{EditorView, ViewState};
use crate::config::Theme;
use crate::editor::windows::Area;
use crate::editor::{DocumentId, Editor, WindowId};
use crate::syntax::Highlights;

/// Scroll state per window, and per buffer within a window, so switching a
/// window between buffers keeps each one's place.
pub type Views = HashMap<(WindowId, DocumentId), ViewState>;

/// Draws every window into `area` and returns where the terminal cursor
/// goes (in the active window).
pub fn draw_windows(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    area: Rect,
    views: &mut Views,
    syntax: &HashMap<DocumentId, &Highlights>,
) -> Option<Position> {
    let windows = editor.windows();
    let (placed, separators) =
        windows
            .layout()
            .arrange(Area::new(area.x, area.y, area.width, area.height));

    let separator_style = theme.get("ui.window.separator");
    let screen = frame.area();
    for separator in separators {
        let a = separator.area;
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
            .syntax(syntax.get(&document.id()).copied());
        frame.render_stateful_widget(view, rect, state);
        if active {
            cursor = state.cursor;
        }
    }
    cursor
}
