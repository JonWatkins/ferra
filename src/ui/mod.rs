//! Rendering. Each piece of the screen is its own widget:
//!
//! ```text
//! ┌─────────────┬────────────────┐
//! │ SidebarView │ EditorView     │
//! │ (<Space>e)  │ (gutter + text)│
//! │             ├────────────────┤
//! │             │ StatusLine     │
//! ├─────────────┴────────────────┤
//! │ CommandLineView              │
//! └──────────────────────────────┘
//! ```
//!
//! With `statusline = "global"` the one status line spans the screen,
//! under the sidebar too.

mod command_line;
mod completion_menu;
pub mod editor_view;
mod hover;
mod picker;
mod sidebar;
mod signature;
mod splash;
mod statusline;
mod text;
mod windows;

pub use command_line::CommandLineView;
pub use completion_menu::CompletionMenu;
pub use editor_view::{EditorView, ViewState};
pub use picker::PickerView;
pub use sidebar::SidebarView;
pub use splash::{LOGO, Splash, SplashEntry};
pub use statusline::StatusLine;
pub use windows::Views;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
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
    mut syntax: Option<&mut SyntaxEngine>,
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

    let screen_main = main;
    // The sidebar takes the left of the screen, down to the command line
    // with per-window status lines (they're the windows'), or down to the
    // one status line, which keeps the full width.
    let (main, status, sidebar_cursor) = match editor.sidebar() {
        Some(explorer) if main.width >= 20 => {
            let per_window = editor.options.statusline == StatuslineMode::Windows;
            let width = editor.options.sidebar_width.clamp(10, main.width / 2);
            let height = main.height + u16::from(per_window);
            let side = Rect::new(main.x, main.y, width, height);
            let separator = Rect::new(main.x + width, main.y, 1, height);
            let rest = main.width - width - 1;
            let main = Rect::new(main.x + width + 1, main.y, rest, main.height);
            let status = if per_window {
                Rect::new(main.x, status.y, rest, status.height)
            } else {
                status
            };
            let focused = editor.mode() == Mode::Sidebar;
            let view = SidebarView::new(explorer, theme)
                .focused(focused)
                .current(editor.document.path())
                .modified(editor.modified_paths());
            frame.render_widget(view, side);
            let separator_style = theme.get("ui.window.separator");
            for y in separator.y..separator.bottom() {
                frame.buffer_mut()[(separator.x, y)]
                    .set_symbol("│")
                    .set_style(separator_style);
            }
            let cursor = focused
                .then(|| SidebarView::cursor_position(explorer, side))
                .flatten();
            (main, status, cursor)
        }
        _ => (main, status, None),
    };

    let window_cursor = match editor.options.statusline {
        StatuslineMode::Windows => {
            let per_window = windows::WindowStatus {
                pending,
                bottom_row: status,
            };
            windows::draw_windows(
                frame,
                editor,
                theme,
                main,
                views,
                syntax.as_deref_mut(),
                Some(per_window),
            )
        }
        StatuslineMode::Global => {
            let cursor = windows::draw_windows(
                frame,
                editor,
                theme,
                main,
                views,
                syntax.as_deref_mut(),
                None,
            );
            frame.render_widget(StatusLine::new(editor, theme, pending), status);
            cursor
        }
    };
    frame.render_widget(CommandLineView::new(editor, theme), command);
    if editor.splash_visible() && !splash.is_empty() {
        frame.render_widget(Splash::new(splash, theme), main);
    }

    // A call's signature floats above the line being typed.
    if editor.mode() == Mode::Insert
        && let (Some(help), Some(cursor)) = (editor.signature_help(), window_cursor)
    {
        let view = signature::SignatureView::new(help, theme);
        let area = view.area(cursor, main);
        frame.render_widget(view, area);
    }

    // The completion menu floats by the cursor.
    if editor.mode() == Mode::Insert
        && let (Some(completion), Some(cursor)) = (editor.completion(), window_cursor)
    {
        let menu = CompletionMenu::new(completion, theme);
        let area = menu.area(cursor, main);
        frame.render_widget(menu, area);
    }

    // A language server's hover floats by the cursor.
    if editor.mode() == Mode::Normal
        && let (Some(hover), Some(cursor)) = (editor.hover(), window_cursor)
    {
        let rows = 0..hover.document.buffer().line_count();
        let highlights = syntax.and_then(|engine| {
            engine.highlight_as(
                &hover.document,
                Some("markdown"),
                &editor.languages,
                &editor.syntax_trees,
                rows,
            )
        });
        let view = hover::HoverView::new(hover, highlights, theme);
        let area = view.area(cursor, main);
        frame.render_widget(view, area);
    }

    // The picker popup floats over the editor pane.
    let popup = editor.picker().filter(|_| editor.mode() == Mode::Picker);
    if let Some(picker) = popup {
        let popup_area = PickerView::area(screen_main);
        frame.render_widget(PickerView::new(picker, theme), popup_area);
    }

    let cursor = match (editor.mode(), popup) {
        (Mode::Picker, Some(picker)) => Some(PickerView::cursor_position(
            picker,
            PickerView::area(screen_main),
        )),
        (Mode::Command, _) => Some(CommandLineView::cursor_position(editor, command)),
        (Mode::Sidebar, _) => sidebar_cursor,
        _ => window_cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}
