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

pub mod activity;
mod command_line;
mod completion_menu;
pub mod editor_view;
pub mod find;
pub mod git_gui;
mod hover;
pub mod icons;
mod input_box;
mod menu_bar;
mod picker;
mod popup;
mod search;
pub mod settings;
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
    /// The activity bar: its area, and each icon's row and what it shows.
    pub activity: Option<(Rect, Vec<(u16, String)>)>,
    /// The menu bar: its row, and each title's columns.
    pub menu_bar: Option<(u16, Vec<(u16, u16)>)>,
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
    /// The settings editor's parts.
    pub settings: Option<settings::SettingsMap>,
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
    /// The Search view's field and toggles, when it's shown.
    pub search: Option<search::SearchHits>,
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

    // The menu bar takes the top row, with a line under it; everything
    // else is laid out below. (A menu drops down over the line.)
    let area = if editor.options.menu_bar && area.height > 5 {
        let row = Rect::new(area.x, area.y, area.width, 1);
        frame.render_widget(menu_bar::MenuBar::new(editor.open_menu(), theme), row);
        map.menu_bar = Some((row.y, crate::editor::menu_bar::title_spans(row.x)));
        horizontal_line(
            frame,
            Rect::new(area.x, area.y + 1, area.width, 1),
            theme.get("ui.menubar.border"),
        );
        Rect::new(area.x, area.y + 2, area.width, area.height - 2)
    } else {
        area
    };

    let [main, status, command] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let screen_main = main;
    // Where the Search view's query cursor goes, while it's typed in.
    let mut search_cursor: Option<(u16, u16)> = None;
    // The activity bar takes the screen's left edge, as the sidebar does
    // (below), from the top down to the status line or the command line.
    let activity_bar = editor.options.activity_bar && main.width >= 30;
    let (main, status) = if activity_bar {
        let per_window = editor.options.statusline == StatuslineMode::Windows;
        let height = main.height + u16::from(per_window);
        let bar = Rect::new(main.x, main.y, activity::WIDTH, height);
        let (items, settings) = editor.activity_items();
        frame.render_widget(activity::ActivityBar::new(&items, &settings, theme), bar);
        let (rows, settings_row) = activity::layout(bar, items.len());
        let mut hits: Vec<(u16, String)> = rows
            .into_iter()
            .zip(items.iter().map(|i| i.name.clone()))
            .collect();
        hits.extend(settings_row.map(|y| (y, settings.name.clone())));
        map.activity = Some((bar, hits));
        let main = Rect::new(
            main.x + bar.width,
            main.y,
            main.width - bar.width,
            main.height,
        );
        let status = if per_window {
            Rect::new(main.x, status.y, main.width, status.height)
        } else {
            status
        };
        (main, status)
    } else {
        (main, status)
    };
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
            // With plugins' views, a row of tabs first (the activity bar
            // does that when it's there).
            let (titles, shown) = editor.sidebar_tabs();
            let tabs = titles.len() > 1 && !activity_bar;
            let content = if tabs {
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
            let search_view = editor.search_view_shown();
            map.sidebar = Some(SidebarMap {
                area: side,
                tabs: tabs.then(|| {
                    (
                        side.y,
                        sidebar::tab_spans(&titles, shown, side.x, side.width),
                    )
                }),
                list: if git_view {
                    GitViewWidget::list(content)
                } else if search_view {
                    search::SearchViewWidget::list(content)
                } else {
                    sidebar::list_rows(content)
                },
                edge: separator,
                search: search_view.then(|| search::SearchViewWidget::hits(editor, content)),
            });
            match plugin_view {
                Some(view) => {
                    frame.render_widget(PluginViewWidget::new(view, focused, theme), content);
                }
                None if git_view => {
                    frame.render_widget(GitViewWidget::new(editor, focused, theme), content);
                }
                None if search_view => {
                    frame.render_widget(
                        search::SearchViewWidget::new(editor, focused, theme),
                        content,
                    );
                    if editor.search_typing() {
                        search_cursor = search::SearchViewWidget::cursor_position(editor, content);
                    }
                }
                None => {
                    let mut view = SidebarView::new(explorer, theme)
                        .focused(focused)
                        .current(editor.document.path())
                        .modified(editor.modified_paths())
                        .git(editor.git_tree_marks());
                    if editor.options.icons == crate::config::Icons::Nerd {
                        view = view.icons(&editor.languages);
                    }
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
                // A line above it, unless a window's status line is there.
                let border = u16::from(!per_window);
                if border == 1 {
                    let line = Rect::new(main.x, panel.y - 1, main.width, 1);
                    horizontal_line(frame, line, theme.get("ui.terminal.border"));
                }
                let windows_bottom = panel.y - border - u16::from(per_window);
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
    if editor.resting()
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

    // The settings editor takes the windows' place.
    let mut settings_cursor = None;
    if editor.settings().is_some() {
        frame.render_widget(settings::SettingsWidget::new(editor, theme), main);
        map.settings = Some(settings::SettingsWidget::map(editor, main));
        settings_cursor = settings::SettingsWidget::cursor_position(editor, main);
    }

    // The find box floats at the top right of the active window.
    let mut find_cursor = None;
    if editor.mode() == Mode::Find
        && !editor.search_typing()
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
        // The blame's code (or each side's) is highlighted around the
        // selected line, as far as it could scroll either way.
        let mut highlights = [None, None];
        if let (Some(gui), Some(engine)) = (editor.git_gui(), syntax) {
            let reach = full.height as usize;
            let around = |line: usize| line.saturating_sub(reach)..line + reach;
            let mut wanted: Vec<(
                usize,
                &crate::editor::Document,
                Option<&str>,
                std::ops::Range<usize>,
            )> = Vec::new();
            if let Some(blame) = &gui.blame {
                wanted.push((
                    0,
                    &blame.document,
                    blame.language.as_deref(),
                    around(blame.row),
                ));
            }
            if let Some(side) = &gui.side {
                for i in 0..2 {
                    // The nearest line this side has to the selected row.
                    let line = side.rows[..=side.row.min(side.rows.len().saturating_sub(1))]
                        .iter()
                        .rev()
                        .find_map(|r| if i == 0 { r.left } else { r.right })
                        .unwrap_or(0);
                    wanted.push((
                        i,
                        &side.documents[i],
                        side.language.as_deref(),
                        around(line),
                    ));
                }
            }
            for (i, document, language, rows) in wanted {
                highlights[i] = engine
                    .highlight_as(
                        document,
                        language,
                        &editor.languages,
                        &editor.syntax_trees,
                        rows,
                    )
                    .cloned();
            }
        }
        frame.render_widget(git_gui::GitGuiView::new(editor, theme, highlights), full);
        map.git = Some(git_gui::GitGuiView::map(editor, full));
    }

    // A plugin's popup floats over the windows.
    if let Some(popup) = editor.popup() {
        // A menu goes under its title, anywhere on the screen.
        let bounds = if matches!(
            popup.position,
            crate::editor::popup::PopupPosition::At { .. }
        ) {
            screen_main
        } else {
            main
        };
        let area = PopupView::area(popup, bounds, window_cursor, drawn.active);
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
        let mut view = PickerView::new(picker, theme);
        if editor.options.icons == crate::config::Icons::Nerd {
            view = view.icons(&editor.languages);
        }
        frame.render_widget(view, popup_area);
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
        (Mode::Find, _) if editor.search_typing() => {
            search_cursor.map(|(x, y)| ratatui::layout::Position::new(x, y))
        }
        (Mode::Find, _) => find_cursor,
        (Mode::Terminal, _) => terminal_cursor,
        // The selection shows where it is.
        (Mode::Popup, _) => None,
        (Mode::Input, _) => input_cursor,
        (Mode::Git, _) => None,
        (Mode::Settings, _) => settings_cursor.map(|(x, y)| ratatui::layout::Position::new(x, y)),
        _ => window_cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
    map
}

/// A row of `─` across `area`.
fn horizontal_line(frame: &mut Frame, area: Rect, style: ratatui::style::Style) {
    let buf = frame.buffer_mut();
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_symbol("─").set_style(style);
    }
}
