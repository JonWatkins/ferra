//! The mouse (`mouse = true`): clicks, drags and the wheel, found by where
//! the last frame drew things (`ui::ScreenMap`).
//!
//! - In a window, a click moves the cursor there (and to that window),
//!   staying in insert mode if that's where you were; a double click
//!   selects the word, a triple click the line; dragging selects.
//! - In the sidebar, a click gives it focus: on a tab it shows that view;
//!   on a line it selects it, and in the file tree opens the file or folder.
//! - A click in the terminal panel gives it focus.
//! - In the picker or a plugin's list popup, a click chooses the line; in
//!   the find box, the toggles flip and the fields take the cursor; in an
//!   input box, the field does. A click outside one of these closes it
//!   (the find box keeps its search, and the click goes on to the text).
//! - Dragging the sidebar's edge, the terminal panel's title row or the
//!   line between two windows resizes them.
//! - The wheel scrolls what's under the pointer: a window (the cursor kept
//!   on screen, as `<C-e>` does), the sidebar, the terminal's history, the
//!   picker or a popup.
//!
//! As the pointer moves, `pointer` says what shape it should be (a hand
//! over things to click, a text bar over text, arrows over edges), which
//! the event loop sends to terminals that take it (OSC 22).

use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::App;
use crate::commands::Invocation;
use crate::editor::find::FindField;
use crate::editor::windows::SplitDirection;
use crate::editor::{Message, Mode, WindowId};
use crate::ui::{FindView, InputBoxView};

/// Lines the wheel scrolls at a time.
const WHEEL_LINES: usize = 3;

/// Clicks this close together on one cell make a double (or triple) click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// What a drag is doing, from where it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Drag {
    /// Selecting text in a window.
    Text(WindowId),
    /// Resizing the sidebar (its edge).
    Sidebar,
    /// Resizing the terminal panel (its title row), whose bottom is here.
    Panel { bottom: u16 },
    /// Resizing a window from the line after it (to its right or below).
    Split {
        window: WindowId,
        area: Rect,
        vertical: bool,
    },
}

/// What's at a place on the screen (boxes aside).
enum Hit {
    Window(WindowId),
    /// An activity bar icon: what it shows.
    Activity(String),
    /// The activity bar between its icons.
    ActivityBar,
    /// The Search view's query field.
    SearchField,
    /// Its replace field, and the replace-all chip.
    SearchReplace,
    SearchReplaceAll,
    /// One of the Search view's toggles: match case, whole word, regex.
    SearchToggle(usize),
    SidebarTab(usize),
    /// A line of the sidebar's list, rows down from its top.
    SidebarLine(usize),
    Sidebar,
    Terminal,
    Nothing,
}

impl App {
    /// Handles a mouse event. Public so tests can drive the app directly.
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        if !self.editor.options.mouse {
            return;
        }
        let at = Position::new(event.column, event.row);
        match event.kind {
            MouseEventKind::Moved => {
                self.pointer = self.pointer_for(at);
                return;
            }
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(at),
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(at),
            MouseEventKind::Up(_) => self.dragging = None,
            MouseEventKind::ScrollDown => self.mouse_wheel(at, true),
            MouseEventKind::ScrollUp => self.mouse_wheel(at, false),
            _ => {}
        }
        self.after_popup_key();
        self.dispatch_events();
    }

    /// The pointer's shape over `at`: a CSS cursor name.
    pub fn pointer_for(&self, at: Position) -> &'static str {
        let map = &self.screen_map;
        if let Some(settings) = &map.settings
            && settings.area.contains(at)
        {
            let on = settings.categories.iter().any(|(r, _)| r.contains(at))
                || settings
                    .items
                    .iter()
                    .any(|(_, value, _)| value.contains(at));
            return if on {
                "pointer"
            } else if settings.search.contains(at) {
                "text"
            } else {
                "default"
            };
        }
        if let Some(git) = &map.git {
            let on_list = git.lists.iter().any(|(list, panel, _)| {
                *panel != crate::editor::git_gui::GitPanel::Diff && list.contains(at)
            });
            return if on_list { "pointer" } else { "default" };
        }
        if let Some(picker) = &map.picker {
            return if picker.list.contains(at) {
                "pointer"
            } else {
                "default"
            };
        }
        if let Some(popup) = &map.popup
            && popup.area.contains(at)
        {
            let list = self.editor.popup().is_some_and(|p| p.select);
            return if list && popup.list.contains(at) {
                "pointer"
            } else {
                "default"
            };
        }
        if let Some(find) = &map.find
            && find.area.contains(at)
        {
            if find.toggles.iter().any(|(area, _)| area.contains(at)) {
                return "pointer";
            }
            let field = find.find.contains(at) || find.replace.is_some_and(|r| r.contains(at));
            return if field { "text" } else { "default" };
        }
        if let Some((area, field)) = map.input
            && area.contains(at)
        {
            return if field.contains(at) {
                "text"
            } else {
                "default"
            };
        }
        if let Some(edge) = self.edge_at(at) {
            return match edge {
                Drag::Sidebar | Drag::Split { vertical: true, .. } => "ew-resize",
                _ => "ns-resize",
            };
        }
        match self.hit(at) {
            Hit::SidebarTab(_) | Hit::Activity(_) | Hit::SearchToggle(_) => "pointer",
            Hit::SearchField | Hit::SearchReplace => "text",
            Hit::SearchReplaceAll => "pointer",
            Hit::SidebarLine(row) if self.sidebar_line_exists(row) => "pointer",
            Hit::Window(_) | Hit::Terminal => "text",
            _ => "default",
        }
    }

    fn sidebar_line_exists(&self, row: usize) -> bool {
        if self.editor.git_view_shown() {
            let index = self.editor.git_scroll() + row;
            return matches!(
                self.editor.git_rows().get(index),
                Some(crate::editor::GitRow::File { .. })
            );
        }
        match self.editor.plugin_view() {
            Some(view) => view.scroll.get() + row < view.lines.len(),
            None => self
                .editor
                .sidebar()
                .is_some_and(|tree| tree.scroll.get() + row < tree.rows().len()),
        }
    }

    fn hit(&self, at: Position) -> Hit {
        let map = &self.screen_map;
        if let Some((bar, items)) = &map.activity
            && bar.contains(at)
        {
            // Anywhere in an icon's block.
            return items
                .iter()
                .find(|(y, _)| (*y..y + crate::ui::activity::BLOCK).contains(&at.y))
                .map_or(Hit::ActivityBar, |(_, name)| Hit::Activity(name.clone()));
        }
        if let Some(sidebar) = &map.sidebar
            && sidebar.area.contains(at)
        {
            if let Some((y, spans)) = &sidebar.tabs
                && at.y == *y
            {
                return spans
                    .iter()
                    .position(|&(from, to)| (from..to).contains(&at.x))
                    .map_or(Hit::Sidebar, Hit::SidebarTab);
            }
            if let Some(search) = &sidebar.search {
                if search.field.contains(at) {
                    return Hit::SearchField;
                }
                if search.replace_all.contains(at) {
                    return Hit::SearchReplaceAll;
                }
                if search.replace.contains(at) {
                    return Hit::SearchReplace;
                }
                if let Some(i) = search.toggles.iter().position(|t| t.contains(at)) {
                    return Hit::SearchToggle(i);
                }
            }
            if sidebar.list.contains(at) {
                return Hit::SidebarLine((at.y - sidebar.list.y) as usize);
            }
            return Hit::Sidebar;
        }
        if map.terminal.is_some_and(|area| area.contains(at)) {
            return Hit::Terminal;
        }
        map.windows
            .iter()
            .find(|(_, area)| area.contains(at))
            .map_or(Hit::Nothing, |&(id, _)| Hit::Window(id))
    }

    /// An edge to drag at `at`: the sidebar's, the panel's title row, or the
    /// line after a window.
    fn edge_at(&self, at: Position) -> Option<Drag> {
        let map = &self.screen_map;
        if map.sidebar.as_ref().is_some_and(|s| s.edge.contains(at)) {
            return Some(Drag::Sidebar);
        }
        if let Some(panel) = map.terminal
            && at.y == panel.y
            && panel.contains(at)
        {
            return Some(Drag::Panel {
                bottom: panel.bottom(),
            });
        }
        let &(line, vertical) = map.separators.iter().find(|(area, _)| area.contains(at))?;
        // The window it follows: to its left, or above it.
        map.windows
            .iter()
            .find(|(_, area)| {
                if vertical {
                    area.right() == line.x && (area.y..area.bottom()).contains(&at.y)
                } else {
                    area.bottom() == line.y && (area.x..area.right()).contains(&at.x)
                }
            })
            .map(|&(window, area)| Drag::Split {
                window,
                area,
                vertical,
            })
    }

    /// Whether clicks go to the screen, rather than a box that has focus.
    fn clicks_allowed(&self) -> bool {
        matches!(
            self.editor.mode(),
            Mode::Normal
                | Mode::Insert
                | Mode::Visual
                | Mode::Sidebar
                | Mode::Terminal
                | Mode::Settings
        )
    }

    /// A click in the settings editor: a category shows its settings, a
    /// setting is selected, and one on its value changes it (as `<CR>`).
    fn click_in_settings(&mut self, at: Position) -> bool {
        let Some(map) = self.screen_map.settings.clone() else {
            return false;
        };
        if !map.area.contains(at)
            || !matches!(
                self.editor.mode(),
                Mode::Settings | Mode::Sidebar | Mode::Terminal | Mode::Normal
            )
        {
            return false;
        }
        self.input.reset();
        self.editor.set_mode(Mode::Settings);
        if map.search.contains(at) {
            self.editor.settings_search();
        } else if let Some(&(_, i)) = map.categories.iter().find(|(row, _)| row.contains(at)) {
            self.editor.settings_category(i);
        } else if let Some(&(_, value, i)) = map.items.iter().find(|(block, ..)| block.contains(at))
        {
            self.editor.settings_select(i);
            if value.contains(at) {
                self.execute("settings_change", Invocation::default());
            }
        }
        true
    }

    fn mouse_down(&mut self, at: Position) {
        // The menu bar first: a title opens its menu (another one's open
        // menu switching to it), the open one's closes it.
        if let Some((y, titles)) = &self.screen_map.menu_bar
            && at.y == *y
        {
            let index = titles
                .iter()
                .position(|&(from, to)| (from..to).contains(&at.x));
            match index {
                Some(index) => {
                    let name = crate::editor::menu_bar::MENUS[index].title.to_lowercase();
                    self.execute(&format!("menu_{name}"), Invocation::default());
                }
                None if self.editor.open_menu().is_some() => {
                    self.execute("popup_close", Invocation::default());
                }
                None => {}
            }
            return;
        }
        // A hover-like popup goes with any click.
        if self.editor.mode() != Mode::Popup && self.editor.popup().is_some() {
            self.editor.close_popup();
        }
        if self.click_in_settings(at) || self.click_in_box(at) || !self.clicks_allowed() {
            return;
        }
        self.input.reset();
        if let Some(edge) = self.edge_at(at) {
            if let Drag::Panel { .. } = edge
                && let Err(error) = self.editor.focus_terminal()
            {
                self.editor.set_message(Message::error(error));
            }
            self.dragging = Some(edge);
            return;
        }
        let clicks = self.count_click(at);
        match self.hit(at) {
            Hit::Window(id) => {
                self.leave_panes();
                if self.editor.mode() == Mode::Visual {
                    self.editor.set_mode(Mode::Normal);
                }
                self.editor.focus_window(id);
                self.put_cursor_at(id, at);
                self.dragging = Some(Drag::Text(id));
                match clicks {
                    2 => self.select_after_click(&["visual_mode", "inner_word"]),
                    3 => self.select_after_click(&["visual_line_mode"]),
                    _ => {}
                }
            }
            Hit::Activity(name) => {
                self.leave_panes();
                if let Err(error) = self.editor.activity(&name) {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::ActivityBar => {}
            Hit::SearchField => self.editor.search_focus_query(),
            Hit::SearchReplace => self.editor.search_focus_replace(),
            Hit::SearchReplaceAll => {
                if let Err(error) = self.editor.search_ask_replace_all() {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::SearchToggle(i) => {
                // Toggled as the keys do, from the query field.
                self.editor.search_focus_query();
                let command = ["find_toggle_case", "find_toggle_word", "find_toggle_regex"][i];
                self.execute(command, Invocation::default());
            }
            Hit::SidebarTab(index) => {
                let names: Vec<String> = self
                    .editor
                    .sidebar_view_names()
                    .iter()
                    .map(|n| n.to_string())
                    .collect();
                if let Some(name) = names.get(index)
                    && let Err(error) = self.editor.open_sidebar_view(name)
                {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::SidebarLine(row) => {
                if let Err(error) = self.editor.sidebar_click_times(row, clicks) {
                    self.editor.set_message(Message::error(error));
                }
                self.editor.finish_command();
            }
            Hit::Sidebar => self.editor.set_mode(Mode::Sidebar),
            Hit::Terminal => {
                if let Err(error) = self.editor.focus_terminal() {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::Nothing => {}
        }
    }

    /// How many clicks this is in a row on the same cell (1 to 3).
    fn count_click(&mut self, at: Position) -> u8 {
        let now = Instant::now();
        let count = match self.last_click {
            Some((when, place, count)) if place == at && now - when <= MULTI_CLICK => count % 3 + 1,
            _ => 1,
        };
        self.last_click = Some((now, at, count));
        count
    }

    /// A double or triple click: from normal mode, the selection commands.
    fn select_after_click(&mut self, commands: &[&str]) {
        if self.editor.mode() != Mode::Normal {
            self.editor.set_mode(Mode::Normal);
        }
        for command in commands {
            self.execute(command, Invocation::default());
        }
        self.dragging = None;
    }

    /// A click while a box has focus (the picker, a popup, the find box,
    /// an input box). Returns whether the click is used up.
    fn click_in_box(&mut self, at: Position) -> bool {
        let map = self.screen_map.clone();
        match self.editor.mode() {
            Mode::Picker => {
                if let Some(picker) = map.picker {
                    if picker.list.contains(at) {
                        let index = picker.first + (at.y - picker.list.y) as usize;
                        let exists = self
                            .editor
                            .picker()
                            .is_some_and(|p| index < p.matches().len());
                        if exists {
                            if let Some(p) = self.editor.picker_mut() {
                                p.select(index);
                            }
                            self.execute("picker_accept", Invocation::default());
                        }
                    } else if !picker.area.contains(at) {
                        self.execute("picker_cancel", Invocation::default());
                    }
                }
                true
            }
            Mode::Popup => {
                if let Some(area) = map.popup {
                    if area.list.contains(at) {
                        let index = area.first + (at.y - area.list.y) as usize;
                        let choose = self.editor.popup_mut().and_then(|popup| {
                            if !popup.select || index >= popup.lines.len() {
                                return None;
                            }
                            popup.select(index);
                            popup
                                .keys
                                .iter()
                                .find(|(keys, _)| keys == "<CR>")
                                .map(|(_, command)| command.clone())
                        });
                        if let Some(command) = choose {
                            self.execute(&command, Invocation::default());
                        }
                    } else if !area.area.contains(at) {
                        self.execute("popup_close", Invocation::default());
                    }
                }
                true
            }
            Mode::Input => {
                if let Some((area, field)) = map.input {
                    if field.contains(at) {
                        if let Some(input) = self.editor.input_box() {
                            let col = InputBoxView::field_col(input, area, at.x);
                            self.editor.input_box_edit(|f| f.set_cursor(col));
                        }
                    } else if !area.contains(at) {
                        self.execute("input_cancel", Invocation::default());
                    }
                }
                true
            }
            Mode::Find if self.editor.search_typing() => {
                // Typing in the Search view's field: a click anywhere
                // leaves it and goes on (back into it, if it's on it).
                self.editor.search_stop_typing();
                false
            }
            Mode::Find => {
                let Some(find) = map.find else {
                    return true;
                };
                if !find.area.contains(at) {
                    // Out of the box: it closes, and the click goes on.
                    self.editor.close_find();
                    return false;
                }
                if let Some(&(_, toggle)) = find.toggles.iter().find(|(area, _)| area.contains(at))
                {
                    self.editor.find_toggle(|f| match toggle {
                        0 => f.match_case = !f.match_case,
                        1 => f.whole_word = !f.whole_word,
                        _ => f.regex = !f.regex,
                    });
                } else if find.find.contains(at) {
                    let field = &self.editor.find_state().find;
                    let col = FindView::field_col(field, find.find, at.x);
                    self.editor.find_click_field(FindField::Find, col);
                } else if let Some(replace) = find.replace.filter(|r| r.contains(at)) {
                    let field = &self.editor.find_state().replace;
                    let col = FindView::field_col(field, replace, at.x);
                    self.editor.find_click_field(FindField::Replace, col);
                }
                true
            }
            Mode::Git => {
                let Some(git) = map.git else {
                    return true;
                };
                if let Some((rows, first)) = git.side {
                    if rows.contains(at) {
                        self.editor
                            .git_gui_side_select(first + (at.y - rows.y) as usize);
                    }
                    return true;
                }
                if let Some((rows, first)) = git.blame {
                    if rows.contains(at) {
                        self.editor
                            .git_gui_blame_select(first + (at.y - rows.y) as usize);
                    }
                    return true;
                }
                if let Some(&(list, panel, first)) =
                    git.lists.iter().find(|(list, ..)| list.contains(at))
                {
                    self.editor
                        .git_gui_select(panel, first + (at.y - list.y) as usize);
                } else if let Some(&(_, panel)) =
                    git.panels.iter().find(|(area, _)| area.contains(at))
                {
                    self.editor.git_gui_focus(panel);
                }
                true
            }
            // The `:` line, or an operator waiting: clicks wait too.
            Mode::Command | Mode::OperatorPending => true,
            _ => false,
        }
    }

    /// A click back into the text leaves the sidebar or the terminal.
    fn leave_panes(&mut self) {
        if matches!(self.editor.mode(), Mode::Sidebar | Mode::Terminal) {
            self.editor.set_mode(Mode::Normal);
        }
    }

    /// Puts the cursor where `at` is in window `id` (the active one), as
    /// it was last drawn.
    fn put_cursor_at(&mut self, id: WindowId, at: Position) {
        let document = self.editor.document.id();
        let Some(state) = self.views.get(&(id, document)) else {
            return;
        };
        let tab_width = self.editor.options_for(id, document).tab_width;
        if let Some(position) = state.position_at(&self.editor.document, at.x, at.y, tab_width) {
            self.editor.document.set_cursor(position);
        }
        self.editor.finish_command();
    }

    fn mouse_drag(&mut self, at: Position) {
        match self.dragging {
            Some(Drag::Text(id)) => self.drag_select(id, at),
            Some(Drag::Sidebar) => {
                if let Some(sidebar) = &self.screen_map.sidebar {
                    self.editor.options.sidebar_width = at.x.saturating_sub(sidebar.area.x).max(10);
                }
            }
            Some(Drag::Panel { bottom }) => {
                self.editor.options.terminal_height = bottom.saturating_sub(at.y + 1).max(2);
            }
            Some(Drag::Split {
                window,
                area,
                vertical,
            }) => {
                let active = self.editor.windows().active();
                self.editor.focus_window(window);
                let (axis, want) = if vertical {
                    (SplitDirection::Vertical, at.x.saturating_sub(area.x))
                } else {
                    (SplitDirection::Horizontal, at.y.saturating_sub(area.y))
                };
                self.editor
                    .resize_window(axis, |_, most| want.clamp(1, most.max(1)));
                self.editor.focus_window(active);
            }
            None => {}
        }
    }

    /// Dragging from a click in a window selects to where the pointer is
    /// (kept inside the window).
    fn drag_select(&mut self, id: WindowId, at: Position) {
        if self.editor.windows().active() != id {
            return;
        }
        let Some(&(_, area)) = self.screen_map.windows.iter().find(|(w, _)| *w == id) else {
            return;
        };
        if area.is_empty() {
            return;
        }
        let inside = Position::new(
            at.x.clamp(area.x, area.right() - 1),
            at.y.clamp(area.y, area.bottom() - 1),
        );
        if self.editor.mode() != Mode::Visual {
            if self.editor.mode() == Mode::Insert {
                self.editor.set_mode(Mode::Normal);
            }
            self.execute("visual_mode", Invocation::default());
        }
        let document = self.editor.document.id();
        let tab_width = self.editor.options_for(id, document).tab_width;
        if let Some(position) = self.views.get(&(id, document)).and_then(|state| {
            state.position_at(&self.editor.document, inside.x, inside.y, tab_width)
        }) {
            self.editor.document.set_cursor(position);
        }
    }

    fn mouse_wheel(&mut self, at: Position, down: bool) {
        let sign: isize = if down { 1 } else { -1 };
        if self
            .screen_map
            .settings
            .as_ref()
            .is_some_and(|s| s.list.contains(at))
        {
            self.editor.settings_move(sign);
            return;
        }
        match self.editor.mode() {
            Mode::Picker => {
                if let Some(picker) = self.editor.picker_mut() {
                    picker.move_selection(sign);
                }
                return;
            }
            Mode::Popup => {
                if let Some(popup) = self.editor.popup_mut() {
                    let step = if popup.select {
                        1
                    } else {
                        WHEEL_LINES as isize
                    };
                    popup.move_by(sign * step);
                }
                return;
            }
            Mode::Git => {
                if self
                    .screen_map
                    .git
                    .as_ref()
                    .is_some_and(|g| g.blame.is_some() || g.side.is_some())
                {
                    self.editor.git_gui_scroll(WHEEL_LINES as isize * sign);
                    return;
                }
                let panel = self.screen_map.git.as_ref().and_then(|git| {
                    git.panels
                        .iter()
                        .find(|(area, _)| area.contains(at))
                        .map(|&(_, panel)| panel)
                });
                match panel {
                    Some(crate::editor::git_gui::GitPanel::Diff) => {
                        self.editor.git_gui_scroll(WHEEL_LINES as isize * sign);
                    }
                    Some(panel) => {
                        self.editor.git_gui_focus(panel);
                        self.editor.git_gui_move(sign);
                    }
                    None => {}
                }
                return;
            }
            _ => {}
        }
        let lines = WHEEL_LINES as isize * sign;
        match self.hit(at) {
            Hit::Window(id) => {
                // Another window only where moving between them makes sense.
                if self.editor.windows().active() != id && !self.clicks_allowed() {
                    return;
                }
                self.editor.focus_window(id);
                let command = if down {
                    "scroll_line_down"
                } else {
                    "scroll_line_up"
                };
                let invocation = Invocation {
                    count: Some(WHEEL_LINES),
                    ..Invocation::default()
                };
                self.execute(command, invocation);
            }
            Hit::SidebarLine(_) | Hit::Sidebar | Hit::SidebarTab(_)
                if self.editor.search_view_shown() =>
            {
                self.editor.search_scroll(lines);
            }
            Hit::SidebarLine(_) | Hit::Sidebar | Hit::SidebarTab(_) => {
                self.editor.sidebar_move(lines);
            }
            Hit::Terminal => self.editor.terminal_scroll_rows(-lines),
            Hit::Activity(_)
            | Hit::ActivityBar
            | Hit::SearchField
            | Hit::SearchReplace
            | Hit::SearchReplaceAll
            | Hit::SearchToggle(_)
            | Hit::Nothing => {}
        }
    }
}
