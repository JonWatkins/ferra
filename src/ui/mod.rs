//! Rendering. Each piece of the screen is its own widget:
//!
//! ```text
//! ┌──────────────────────────────┐
//! │ EditorView (gutter + text)   │
//! │                              │
//! ├──────────────────────────────┤
//! │ StatusLine                   │
//! ├──────────────────────────────┤
//! │ CommandLineView              │
//! └──────────────────────────────┘
//! ```

mod command_line;
pub mod editor_view;
mod statusline;

pub use command_line::CommandLineView;
pub use editor_view::{EditorView, ViewState};
pub use statusline::StatusLine;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
};

use crate::config::Theme;
use crate::editor::{Editor, Mode};

/// Draws the whole screen and positions the terminal cursor.
pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    view: &mut ViewState,
    pending: &str,
) {
    let area = frame.area();
    let base = theme.get("ui.background").patch(theme.get("ui.text"));
    frame.buffer_mut().set_style(area, base);

    let [main, status, command] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let editor_view = EditorView::new(&editor.document, &editor.options, theme);
    frame.render_stateful_widget(editor_view, main, view);
    frame.render_widget(StatusLine::new(editor, theme, pending), status);
    frame.render_widget(CommandLineView::new(editor, theme), command);

    let cursor = match editor.mode() {
        Mode::Command => Some(CommandLineView::cursor_position(editor, command)),
        _ => view.cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}
