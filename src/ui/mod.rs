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
pub mod agent;
pub mod breadcrumbs;
mod command_line;
mod completion_menu;
pub mod editor_view;
pub mod find;
pub mod git_gui;
mod hover;
pub mod icons;
mod input_box;
pub mod keybindings;
mod menu_bar;
pub mod panel;
mod picker;
mod popup;
pub mod screen_map;
mod search;
pub mod settings;
mod sidebar;
mod signature;
mod splash;
mod statusline;
pub mod tab_bar;
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
pub use screen_map::{Region, ScreenMap, Target};
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
use screen_map::SidebarPart;

use crate::editor::windows::{Area, PANEL_WINDOW};
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
        map.add(row, Target::MenuBar(None));
        for (i, (from, to)) in crate::editor::menu_bar::title_spans(row.x)
            .into_iter()
            .enumerate()
        {
            let title = Rect::new(from, row.y, to.saturating_sub(from), 1);
            map.add(title.intersection(row), Target::MenuBar(Some(i)));
        }
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
        map.add(bar, Target::Activity(None));
        // Anywhere in an icon's block.
        let blocks = rows
            .into_iter()
            .zip(items.iter().map(|i| &i.name))
            .chain(settings_row.map(|y| (y, &settings.name)));
        for (y, name) in blocks {
            let block = Rect::new(bar.x, y, bar.width, activity::BLOCK);
            map.add(
                block.intersection(bar),
                Target::Activity(Some(name.clone())),
            );
        }
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
            // Recorded once drawn, with the lists scrolled as drawn.
            let part = Target::Sidebar;
            map.add(side, part(SidebarPart::Rest));
            map.add(separator, Target::SidebarEdge);
            if tabs {
                for (i, (from, to)) in sidebar::tab_spans(&titles, shown, side.x, side.width)
                    .into_iter()
                    .enumerate()
                {
                    let tab = Rect::new(from, side.y, to.saturating_sub(from), 1);
                    map.add(tab.intersection(side), part(SidebarPart::Tab(i)));
                }
            }
            let (list, first, len) = if git_view {
                let list = GitViewWidget::list(content);
                (list, editor.git_scroll(), editor.git_rows().len())
            } else if search_view {
                let search = editor.project_search();
                let list = search::SearchViewWidget::list(content);
                (list, search.scroll.get(), search.row_count())
            } else if let Some(view) = plugin_view {
                (
                    sidebar::list_rows(content),
                    view.scroll.get(),
                    view.lines.len(),
                )
            } else {
                let list = sidebar::list_rows(content);
                (list, explorer.scroll.get(), explorer.rows().len())
            };
            map.rows(list, first, len, |i| part(SidebarPart::Line(i - first)));
            if search_view {
                search::SearchViewWidget::record(editor, content, &mut map);
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

    // The agent sidebar takes the right of what's left, down to the
    // command line (or the one status line, as the left sidebar).
    let mut agent_cursor = None;
    let (main, status) = if editor.agent_chat().shown() && main.width >= 40 {
        let per_window = editor.options.statusline == StatuslineMode::Windows;
        // At least 24 columns, given room; at most half of what there is.
        let width = editor.options.agent_width.max(24).min(main.width / 2);
        let height = main.height + u16::from(per_window);
        let side = Rect::new(main.right() - width, main.y, width, height);
        let edge = Rect::new(side.x - 1, main.y, 1, height);
        let rest = main.width - width - 1;
        agent_cursor = agent::draw(frame.buffer_mut(), side, edge, editor, theme, &mut map);
        let main = Rect::new(main.x, main.y, rest, main.height);
        let status = if per_window {
            Rect::new(status.x, status.y, rest, status.height)
        } else {
            status
        };
        (main, status)
    } else {
        (main, status)
    };

    // The bottom panel takes the bottom of what's left: a row of tabs and
    // the tab's rows (the shell's, for the terminal), under the windows
    // (and their status lines).
    let mut terminal_cursor = None;
    // Terminal-normal mode: the panel's window, drawn over the shell.
    let mut scrollback: Option<(Option<ratatui::layout::Position>, Rect)> = None;
    let (main, status) = match editor.panel_shown() {
        Some(tab) => {
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
                match (tab, editor.terminal_panel()) {
                    (crate::editor::panel::PanelTab::Terminal, Some(terminal)) => {
                        let focused = editor.mode() == Mode::Terminal;
                        let shells = editor.terminals();
                        let nerd = editor.options.icons == crate::config::Icons::Nerd;
                        let view = TerminalView::new(
                            shells.all(),
                            shells.shown_index(),
                            focused,
                            &keys.terminal,
                            theme,
                        )
                        .tabbed()
                        .nerd(nerd)
                        .list_width(editor.options.terminal_list_width);
                        frame.render_widget(view, panel);
                        let row = Rect::new(panel.x, panel.y, panel.width, 1);
                        panel::draw_tabs(editor, theme, row, frame.buffer_mut());
                        let list = terminal::shell_list(editor, panel);
                        if focused {
                            terminal_cursor = TerminalView::cursor_position(terminal, panel, list);
                        }
                        let body = terminal::body(panel, list);
                        editor.windows().note_panel_area(Area::new(
                            body.x,
                            body.y,
                            body.width,
                            body.height,
                        ));
                        if editor.in_scrollback() {
                            scrollback = Some((None, body));
                        }
                    }
                    _ => frame.render_widget(panel::PanelView::new(editor, theme), panel),
                }
                panel::PanelView::record(editor, panel, &mut map);
                // Over the shell's screen, and on top of it in the map.
                if let Some((cursor, body)) = &mut scrollback {
                    frame.render_widget(ratatui::widgets::Clear, *body);
                    let document = &editor.document;
                    *cursor = windows::draw_text(
                        frame,
                        &mut map,
                        editor,
                        theme,
                        (PANEL_WINDOW, document, document.cursor()),
                        *body,
                        views,
                        syntax.as_deref_mut(),
                    );
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
                &mut map,
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
                &mut map,
                editor,
                theme,
                main,
                views,
                syntax.as_deref_mut(),
                None,
            );
            frame.render_widget(StatusLine::new(editor, theme, pending), status);
            for (element, from, to) in StatusLine::places(editor, theme, pending, status) {
                let item = Rect::new(from, status.y, to.saturating_sub(from), 1);
                map.add(item.intersection(status), Target::Status(element));
            }
            drawn
        }
    };
    let window_cursor = drawn.cursor.or(scrollback.and_then(|(cursor, _)| cursor));
    let active_area = drawn.active.or(scrollback.map(|(_, body)| body));
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

    // A language server's hover floats by the cursor (or where the
    // pointer rested, for its hover).
    let hover_at = editor.hover().and_then(|hover| match hover.anchor {
        Some((x, y)) => Some(ratatui::layout::Position::new(x, y)),
        None if editor.resting() => window_cursor,
        None => None,
    });
    if let (Some(hover), Some(cursor)) = (editor.hover(), hover_at) {
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
        let area = view.area(cursor, main, hover.anchor.is_some());
        frame.render_widget(view, area);
        map.add(area, Target::Hover);
    }

    // The settings editor takes the windows' place (under a tab row).
    let main = if editor.options.tab_bar
        && main.height > 4
        && (editor.settings().is_some() || editor.keybindings().is_some())
    {
        let row = Rect::new(main.x, main.y, main.width, 1);
        frame.render_widget(ratatui::widgets::Clear, row);
        let active = editor.windows().active();
        frame.render_widget(tab_bar::TabBar::new(editor, theme, active), row);
        tab_bar::TabBar::record(editor, row, active, &mut map);
        Rect::new(main.x, main.y + 1, main.width, main.height - 1)
    } else {
        main
    };
    let mut settings_cursor = None;
    if editor.settings().is_some() {
        frame.render_widget(settings::SettingsWidget::new(editor, theme), main);
        settings::SettingsWidget::record(editor, main, &mut map);
        settings_cursor = settings::SettingsWidget::cursor_position(editor, main);
    }
    // So does the keybinding editor.
    if editor.keybindings().is_some() {
        frame.render_widget(keybindings::KeybindingsWidget::new(editor, theme), main);
        keybindings::KeybindingsWidget::record(editor, main, &mut map);
        settings_cursor = keybindings::KeybindingsWidget::cursor_position(editor, main);
    }

    // The find box floats at the top right of the active window.
    let mut find_cursor = None;
    if find::shown(editor)
        && let Some(window) = active_area
    {
        let view = FindView::new(editor.find_state(), editor.find_count(), find_keys, theme);
        let current = window_cursor.and_then(|at| find::current_match_cells(editor, at));
        let area = FindView::area(editor.find_state(), window, current);
        find_cursor = FindView::cursor_position(editor.find_state(), area);
        FindView::record(editor.find_state(), find_keys, area, &mut map);
        frame.render_widget(view, area);
    }

    // The git view covers everything but the command line.
    if editor.git_gui().is_some() {
        let full = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        // The blame's code (or each side's) is highlighted around the
        // selected line, as far as it could scroll either way.
        let mut highlights = [None, None];
        let mut diff_highlights = std::collections::HashMap::new();
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
            // The diff panel's files' code, each side's rows around those
            // shown, as far as a page scrolls either way.
            if gui.blame.is_none() && gui.side.is_none() {
                let shown = gui.diff_scroll.saturating_sub(reach)..gui.diff_scroll + 2 * reach;
                let mut rows: std::collections::BTreeMap<(usize, usize), std::ops::Range<usize>> =
                    Default::default();
                for &(file, side, row) in
                    gui.diff_syntax.lines[shown.start.min(gui.diff_syntax.lines.len())
                        ..shown.end.min(gui.diff_syntax.lines.len())]
                        .iter()
                        .flatten()
                {
                    let range = rows.entry((file, side)).or_insert(row..row + 1);
                    range.start = range.start.min(row);
                    range.end = range.end.max(row + 1);
                }
                for ((file, side), range) in rows {
                    let file_syntax = &gui.diff_syntax.files[file];
                    if let Some(found) = engine.highlight_as(
                        &file_syntax.documents[side],
                        Some(&file_syntax.language),
                        &editor.languages,
                        &editor.syntax_trees,
                        range,
                    ) {
                        diff_highlights.insert((file, side), found.clone());
                    }
                }
            }
        }
        frame.render_widget(
            git_gui::GitGuiView::new(editor, theme, highlights).diff_highlights(diff_highlights),
            full,
        );
        git_gui::GitGuiView::record(editor, full, &mut map);
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
        let area = PopupView::area(popup, bounds, window_cursor, active_area);
        frame.render_widget(PopupView::new(popup, theme), area);
        PopupView::record(popup, area, &mut map);
    }

    // A plugin's input box, at the top centre of the windows.
    let mut input_cursor = None;
    if let Some(input) = editor.input_box() {
        let area = InputBoxView::area(main);
        input_cursor = Some(InputBoxView::cursor_position(input, area));
        map.add(area, Target::Input { field: false });
        map.add(input_box::field_area(area), Target::Input { field: true });
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
        map.add(popup_area, Target::Picker(None));
        map.rows(list, first, picker.matches().len(), |i| {
            Target::Picker(Some(i))
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
        (Mode::Agent, _) => agent_cursor,
        (Mode::Git, _) => None,
        (Mode::Settings | Mode::Keybindings, _) => {
            settings_cursor.map(|(x, y)| ratatui::layout::Position::new(x, y))
        }
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
