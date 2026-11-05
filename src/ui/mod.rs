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
mod picker;
mod statusline;
mod text;

pub use command_line::CommandLineView;
pub use editor_view::{EditorView, ViewState};
pub use picker::PickerView;
pub use statusline::StatusLine;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
};

use crate::config::Theme;
use crate::editor::{Editor, Mode};
use crate::syntax::Highlights;

/// Draws the whole screen and positions the terminal cursor.
pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    view: &mut ViewState,
    pending: &str,
    syntax: Option<&Highlights>,
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

    let editor_view = EditorView::new(&editor.document, &editor.options, theme)
        .selection(editor.selection_range())
        .search(editor.search_highlight())
        .syntax(syntax);
    frame.render_stateful_widget(editor_view, main, view);
    frame.render_widget(StatusLine::new(editor, theme, pending), status);
    frame.render_widget(CommandLineView::new(editor, theme), command);

    // The picker popup floats over the editor pane.
    let popup = editor.picker().filter(|_| editor.mode() == Mode::Picker);
    if let Some(picker) = popup {
        let popup_area = PickerView::area(main);
        frame.render_widget(PickerView::new(picker, theme), popup_area);
    }

    let cursor = match (editor.mode(), popup) {
        (Mode::Picker, Some(picker)) => {
            Some(PickerView::cursor_position(picker, PickerView::area(main)))
        }
        (Mode::Command, _) => Some(CommandLineView::cursor_position(editor, command)),
        _ => view.cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}
