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

use super::breadcrumbs::Breadcrumbs;
use super::screen_map::{ScreenMap, Target};
use super::tab_bar::TabBar;
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

/// Where drawing the windows put things.
pub struct Drawn {
    /// Where the terminal cursor goes (in the active window).
    pub cursor: Option<Position>,
    /// The active window's area (its text, not its status line).
    pub active: Option<Rect>,
}

/// Draws every window into `area`, recording in `map` each one's text,
/// tabs and breadcrumbs, and the lines between them.
#[allow(clippy::too_many_arguments)]
pub fn draw_windows(
    frame: &mut Frame,
    map: &mut ScreenMap,
    editor: &Editor,
    theme: &Theme,
    area: Rect,
    views: &mut Views,
    mut syntax: Option<&mut SyntaxEngine>,
    status: Option<WindowStatus>,
) -> Drawn {
    let windows = editor.windows();
    let area_cells = Area::new(area.x, area.y, area.width, area.height);
    // Resizing works in cells, so the editor needs to know the real size.
    windows.note_area(area_cells);
    // Each window's own tab row and breadcrumbs, at its top.
    let (tab_bar, crumbs) = (editor.options.tab_bar, editor.options.breadcrumbs);
    windows.note_chrome(u16::from(tab_bar) + u16::from(crumbs));
    let (placed, separators) = windows.layout().arrange(area_cells);

    let separator_areas: Vec<(Rect, bool)> = separators
        .iter()
        .map(|s| {
            (
                Rect::new(s.area.x, s.area.y, s.area.width, s.area.height),
                s.vertical,
            )
        })
        .collect();
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
    let mut active_area = None;
    let mut areas = Vec::new();
    for (id, window_area) in placed {
        let Some((document, window_cursor)) = editor.window_view(id) else {
            continue;
        };
        let active = id == windows.active();
        let mut rect = Rect::new(
            window_area.x,
            window_area.y,
            window_area.width,
            window_area.height,
        );
        if windows.chrome_for(rect.height) > 0 {
            if tab_bar {
                let row = Rect::new(rect.x, rect.y, rect.width, 1).intersection(screen);
                frame.render_widget(TabBar::new(editor, theme, id), row);
                TabBar::record(editor, row, id, map);
                rect = Rect::new(rect.x, rect.y + 1, rect.width, rect.height - 1);
            }
            if crumbs {
                let row = Rect::new(rect.x, rect.y, rect.width, 1).intersection(screen);
                frame.render_widget(Breadcrumbs::new(editor, theme, id), row);
                Breadcrumbs::record(editor, row, id, map);
                rect = Rect::new(rect.x, rect.y + 1, rect.width, rect.height - 1);
            }
        }
        areas.push((id, rect));
        let drawn = draw_text(
            frame,
            map,
            editor,
            theme,
            (id, document, window_cursor),
            rect,
            views,
            syntax.as_deref_mut(),
        );
        if active {
            cursor = drawn;
            active_area = Some(rect);
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
    // A line between windows resizes the window before it (to its left,
    // or above it): each window's stretch of it.
    for (line, vertical) in separator_areas {
        for &(window, area) in &areas {
            let stretch = if vertical && area.right() == line.x {
                Rect::new(line.x, area.y, line.width, area.height)
            } else if !vertical && area.bottom() == line.y {
                Rect::new(area.x, line.y, area.width, line.height)
            } else {
                continue;
            };
            map.add(
                stretch.intersection(line),
                Target::Split { window, vertical },
            );
        }
    }
    Drawn {
        cursor,
        active: active_area,
    }
}

/// Draws window `id`'s text (`document`, the cursor at `window_cursor`)
/// in `rect`, recording it in `map`: a window's, or the terminal panel's
/// in terminal-normal mode. Returns where the terminal cursor goes, if
/// it's the active window.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    frame: &mut Frame,
    map: &mut ScreenMap,
    editor: &Editor,
    theme: &Theme,
    (id, document, window_cursor): (WindowId, &crate::editor::Document, crate::core::Position),
    rect: Rect,
    views: &mut Views,
    syntax: Option<&mut SyntaxEngine>,
) -> Option<Position> {
    let windows = editor.windows();
    let active = id == windows.active();
    map.add(rect, Target::Window(id));
    // Each window draws with its own options (`:setlocal wrap`...).
    let options = editor.options_for(id, document.id());
    let state = views.entry((id, document.id())).or_default();
    if let Some(top) = windows.take_requested_top(id) {
        let last = document.buffer().line_count().saturating_sub(1);
        state.viewport.top = if top.line > last { last.into() } else { top };
    }
    // The find box covers the active window's top rows.
    let covered = if active && super::find::shown(editor) {
        super::FindView::height(editor.find_state()) as usize
    } else {
        0
    };
    let highlights = syntax.and_then(|engine| {
        let rows = state.visible_rows(document, window_cursor, rect, &options, covered);
        engine.highlight(document, &editor.languages, &editor.syntax_trees, rows)
    });

    let view = EditorView::new(document, &options, theme)
        .cursor(window_cursor)
        .active(active)
        .covered(covered)
        .selection(if active {
            editor.selection_range()
        } else {
            None
        })
        .block(editor.selection_block().filter(|_| active))
        .extra_cursors(if active { editor.extra_cursors() } else { &[] })
        .search(editor.search_highlight())
        .find_scope(if active { editor.find_scope() } else { None })
        .git_changes(editor.git_hunks(document.id()))
        .conflicts(editor.git_conflicts(document.id()))
        .caret(active && editor.mode() == crate::editor::Mode::Insert)
        .current_match(if active {
            editor.find_current_match()
        } else {
            None
        })
        .references(if active {
            editor.references_shown()
        } else {
            &[]
        })
        .code_action_sign(editor.code_action_sign().filter(|_| active))
        .syntax(highlights)
        .diagnostics(editor.diagnostics(document.id()));
    let view = if options.bracket_colors {
        view.brackets(
            editor.bracket_states(document),
            editor.comments_of(document),
        )
    } else {
        view
    };
    let view = if options.indent_guides {
        view.indent_guides(editor.indent_width_of(document, &options))
    } else {
        view
    };
    frame.render_stateful_widget(view, rect, state);
    for (at, row) in state.fold_markers() {
        map.add(
            Rect::new(at.x, at.y, 1, 1),
            Target::Fold { window: id, row },
        );
    }
    windows.note_top(id, state.viewport.top);
    state.cursor
}
