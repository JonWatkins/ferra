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
//! under the sidebar too. The terminal panel (`TerminalView`, `<Space>t`)
//! takes the bottom of the windows' side, under their status lines.

mod command_line;
mod completion_menu;
pub mod editor_view;
pub mod find;
pub mod git_gui;
mod hover;
mod input_box;
mod picker;
mod popup;
mod sidebar;
mod signature;
mod splash;
mod statusline;
mod terminal;
mod text;
mod windows;

pub use command_line::CommandLineView;
pub use completion_menu::CompletionMenu;
pub use editor_view::{EditorView, ViewState};
pub use find::{FindKeys, FindView, key_label};
pub use input_box::{InputBoxView, InputKeys};
pub use picker::PickerView;
pub use popup::PopupView;
pub use sidebar::{GitViewWidget, PluginViewWidget, SidebarTabs, SidebarView};
pub use splash::{LOGO, Splash, SplashEntry};
pub use statusline::StatusLine;
pub use terminal::{TerminalKeys, TerminalView};
pub use windows::Views;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};

use crate::config::{StatuslineMode, Theme};

use crate::editor::{Editor, Mode};
use crate::syntax::SyntaxEngine;

/// The keys the interface shows, as bound (so a rebinding shows up).
#[derive(Debug, Clone, Default)]
pub struct KeyHints {
    /// The splash screen's menu (only while it's shown).
    pub splash: Vec<SplashEntry>,
    /// The find box's (only while it's open).
    pub find: FindKeys,
    /// The terminal panel's (only while it's open).
    pub terminal: TerminalKeys,
    /// A plugin's input box's (only while it's open).
    pub input: InputKeys,
}

/// Where things were drawn, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenMap {
    /// Each window's text area (status line not included).
    pub windows: Vec<(crate::editor::WindowId, Rect)>,
    pub sidebar: Option<SidebarMap>,
    /// The terminal panel, title row included.
    pub terminal: Option<Rect>,
    /// The lines between windows, and whether each is vertical.
    pub separators: Vec<(Rect, bool)>,
    /// A plugin's popup, the picker: where, and their lines.
    pub popup: Option<ListMap>,
    pub picker: Option<ListMap>,
    pub find: Option<find::FindHits>,
    /// An input box, and its field.
    pub input: Option<(Rect, Rect)>,
    /// The git view's panels and lists.
    pub git: Option<git_gui::GitGuiMap>,
}

/// A box with a list: its area, the list's rows, and the index of the item
/// on the first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListMap {
    pub area: Rect,
    pub list: Rect,
    pub first: usize,
}

/// Where the sidebar's parts were drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidebarMap {
    pub area: Rect,
    /// The tabs' row and each tab's columns, when there are tabs.
    pub tabs: Option<(u16, Vec<(u16, u16)>)>,
    /// The rows of the list.
    pub list: Rect,
    /// The line between it and the windows (dragged to resize it).
    pub edge: Rect,
}

/// Draws the whole screen and positions the terminal cursor. Returns where
/// things went, for the mouse.
pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    views: &mut Views,
    pending: &str,
    mut syntax: Option<&mut SyntaxEngine>,
    keys: &KeyHints,
) -> ScreenMap {
    let mut map = ScreenMap::default();
    let splash = keys.splash.as_slice();
    let find_keys = &keys.find;
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
    let (main, status) = match editor.sidebar() {
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
            // With plugins' views, a row of tabs first.
            let (titles, shown) = editor.sidebar_tabs();
            let content = if titles.len() > 1 {
                frame.render_widget(SidebarTabs::new(&titles, shown, focused, theme), side);
                Rect::new(
                    side.x,
                    side.y + 1,
                    side.width,
                    side.height.saturating_sub(1),
                )
            } else {
                side
            };
            let plugin_view = editor.plugin_view();
            let git_view = editor.git_view_shown();
            map.sidebar = Some(SidebarMap {
                area: side,
                tabs: (titles.len() > 1).then(|| (side.y, sidebar::tab_spans(&titles, side.x))),
                list: if git_view {
                    GitViewWidget::list(content)
                } else {
                    sidebar::list_rows(content)
                },
                edge: separator,
            });
            match plugin_view {
                Some(view) => {
                    frame.render_widget(PluginViewWidget::new(view, focused, theme), content);
                }
                None if git_view => {
                    frame.render_widget(GitViewWidget::new(editor, focused, theme), content);
                }
                None => {
                    let view = SidebarView::new(explorer, theme)
                        .focused(focused)
                        .current(editor.document.path())
                        .modified(editor.modified_paths())
                        .git(editor.git_tree_marks());
                    frame.render_widget(view, content);
                }
            }
            let separator_style = theme.get("ui.window.separator");
            for y in separator.y..separator.bottom() {
                frame.buffer_mut()[(separator.x, y)]
                    .set_symbol("│")
                    .set_style(separator_style);
            }
            (main, status)
        }
        _ => (main, status),
    };

    // The terminal panel takes the bottom of what's left: a title row and
    // the shell's rows, under the windows (and their status lines).
    let mut terminal_cursor = None;
    let (main, status) = match editor.terminal_panel() {
        Some(terminal) => {
            let per_window = editor.options.statusline == StatuslineMode::Windows;
            let region = main.height + u16::from(per_window);
            // Room for a few rows of windows (and their status line).
            let room = region.saturating_sub(6);
            if room >= 2 {
                let height = (editor.options.terminal_height.max(2) + 1).min(room);
                let panel = Rect::new(main.x, main.y + region - height, main.width, height);
                let windows_bottom = panel.y - u16::from(per_window);
                let main = Rect::new(main.x, main.y, main.width, windows_bottom - main.y);
                let status = if per_window {
                    Rect::new(status.x, windows_bottom, status.width, 1)
                } else {
                    status
                };
                let focused = editor.mode() == Mode::Terminal;
                map.terminal = Some(panel);
                let view = TerminalView::new(terminal, focused, &keys.terminal, theme);
                frame.render_widget(view, panel);
                if focused {
                    terminal_cursor = TerminalView::cursor_position(terminal, panel);
                }
                (main, status)
            } else {
                (main, status)
            }
        }
        None => (main, status),
    };

    let drawn = match editor.options.statusline {
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
            let drawn = windows::draw_windows(
                frame,
                editor,
                theme,
                main,
                views,
                syntax.as_deref_mut(),
                None,
            );
            frame.render_widget(StatusLine::new(editor, theme, pending), status);
            drawn
        }
    };
    let window_cursor = drawn.cursor;
    map.windows = drawn.windows.clone();
    map.separators = drawn.separators.clone();
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
        let highlights = syntax.as_deref_mut().and_then(|engine| {
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

    // The find box floats at the top right of the active window.
    let mut find_cursor = None;
    if editor.mode() == Mode::Find
        && let Some(window) = drawn.active
    {
        let view = FindView::new(editor.find_state(), editor.find_count(), find_keys, theme);
        let area = FindView::area(editor.find_state(), window);
        find_cursor = FindView::cursor_position(editor.find_state(), area);
        map.find = Some(FindView::hits(editor.find_state(), find_keys, area));
        frame.render_widget(view, area);
    }

    // The git view covers everything but the command line.
    if editor.git_gui().is_some() {
        let full = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        // The blame's code is highlighted around the selected line (as
        // far as it could scroll either way).
        let highlights = editor
            .git_gui()
            .and_then(|g| g.blame.as_ref())
            .zip(syntax)
            .and_then(|(blame, engine)| {
                let reach = full.height as usize;
                let rows = blame.row.saturating_sub(reach)..blame.row + reach;
                engine.highlight_as(
                    &blame.document,
                    blame.language.as_deref(),
                    &editor.languages,
                    &editor.syntax_trees,
                    rows,
                )
            });
        frame.render_widget(git_gui::GitGuiView::new(editor, theme, highlights), full);
        map.git = Some(git_gui::GitGuiView::map(editor, full));
    }

    // A plugin's popup floats over the windows.
    if let Some(popup) = editor.popup() {
        let area = PopupView::area(popup, main, window_cursor, drawn.active);
        frame.render_widget(PopupView::new(popup, theme), area);
        let inner = ratatui::widgets::Block::bordered().inner(area);
        map.popup = Some(ListMap {
            area,
            list: inner,
            // Set as it drew.
            first: popup.scroll.get(),
        });
    }

    // A plugin's input box, at the top centre of the windows.
    let mut input_cursor = None;
    if let Some(input) = editor.input_box() {
        let area = InputBoxView::area(main);
        input_cursor = Some(InputBoxView::cursor_position(input, area));
        map.input = Some((area, input_box::field_area(area)));
        frame.render_widget(InputBoxView::new(input, &keys.input, theme), area);
    }

    // The picker popup floats over the editor pane.
    let popup = editor.picker().filter(|_| editor.mode() == Mode::Picker);
    if let Some(picker) = popup {
        let popup_area = PickerView::area(screen_main);
        frame.render_widget(PickerView::new(picker, theme), popup_area);
        let (list, first) = PickerView::list(picker, popup_area);
        map.picker = Some(ListMap {
            area: popup_area,
            list,
            first,
        });
    }

    let cursor = match (editor.mode(), popup) {
        (Mode::Picker, Some(picker)) => Some(PickerView::cursor_position(
            picker,
            PickerView::area(screen_main),
        )),
        (Mode::Command, _) => Some(CommandLineView::cursor_position(editor, command)),
        // The selected row's highlight shows where it is (as in VS Code).
        (Mode::Sidebar, _) => None,
        (Mode::Find, _) => find_cursor,
        (Mode::Terminal, _) => terminal_cursor,
        // The selection shows where it is.
        (Mode::Popup, _) => None,
        (Mode::Input, _) => input_cursor,
        (Mode::Git, _) => None,
        _ => window_cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
    map
}
