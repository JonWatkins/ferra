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
mod completion_menu;
pub mod editor_view;
mod picker;
mod splash;
mod statusline;
mod text;
mod windows;

pub use command_line::CommandLineView;
pub use completion_menu::CompletionMenu;
pub use editor_view::{EditorView, ViewState};
pub use picker::PickerView;
pub use splash::{LOGO, Splash, SplashEntry};
pub use statusline::StatusLine;
pub use windows::Views;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
};

use crate::config::{StatuslineMode, Theme};

use crate::editor::{Editor, Mode};
use crate::syntax::SyntaxEngine;

/// Draws the whole screen and positions the terminal cursor.
pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    views: &mut Views,
    pending: &str,
    syntax: Option<&mut SyntaxEngine>,
    splash: &[SplashEntry],
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

    let window_cursor = match editor.options.statusline {
        StatuslineMode::Windows => {
            let per_window = windows::WindowStatus {
                pending,
                bottom_row: status,
            };
            windows::draw_windows(frame, editor, theme, main, views, syntax, Some(per_window))
        }
        StatuslineMode::Global => {
            let cursor = windows::draw_windows(frame, editor, theme, main, views, syntax, None);
            frame.render_widget(StatusLine::new(editor, theme, pending), status);
            cursor
        }
    };
    frame.render_widget(CommandLineView::new(editor, theme), command);
    if editor.splash_visible() && !splash.is_empty() {
        frame.render_widget(Splash::new(splash, theme), main);
    }

    // The completion menu floats by the cursor.
    if editor.mode() == Mode::Insert
        && let (Some(completion), Some(cursor)) = (editor.completion(), window_cursor)
    {
        let menu = CompletionMenu::new(completion, theme);
        let area = menu.area(cursor, main);
        frame.render_widget(menu, area);
    }

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
        _ => window_cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}
