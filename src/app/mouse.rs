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
//!
//! `ui::draw` returns a `ScreenMap` of where things went: each window's text
//! area and the lines between windows, the sidebar (its tabs, list rows and
//! edge), the terminal panel, and whichever box is open (the picker's and a
//! popup's list rows with the index of the first item shown, the find box's
//! fields and toggles from `FindView::hits`, an input box's field). The app
//! keeps the last one, and `app/mouse.rs` works out what an event is over from
//! it. Text positions come from the window's `ViewState`, which records each
//! screen row as it's drawn (`DrawnRow`: the buffer row, the chars on it, the
//! display column it starts at and the wrap indent), so
//! `ViewState::position_at` maps a cell back through wrapping, folds, tabs,
//! wide chars and sideways scrolling without laying anything out again.
//!
//! A click goes first to a box with focus (`click_in_box`): the picker chooses
//! or closes, a list popup runs its `<CR>` command, the find box flips toggles
//! and moves between fields (a click outside it closes it and goes on to the
//! text), an input box places its cursor or cancels. Otherwise an edge starts a
//! resize drag (`Drag::Sidebar` sets `sidebar-width`, `Drag::Panel`
//! `terminal-height`, `Drag::Split` resizes the window before the line through
//! `resize_window`), and anything else is a click on a window, the sidebar or
//! the panel. A window click focuses it and sets the cursor; double and triple
//! clicks (same cell, within 400 ms) run `visual_mode` + `inner_word` or
//! `visual_line_mode`; a drag from it starts visual mode and moves the cursor,
//! kept inside the window.
//!
//! As the pointer moves, `App::pointer_for` names its shape (CSS names:
//! `pointer`, `text`, `ew-resize`, `ns-resize`, `default`); the event loop
//! sends it with OSC 22 when it changes, and pointer moves don't redraw. iTerm2
//! only knows X11 names (`hand2`, `xterm`, `sb_h_double_arrow`), so when
//! `TERM_PROGRAM` or `LC_TERMINAL` says it's iTerm2, `terminal::x11_pointer`
//! translates; it honours them only from 3.7.0 (before that, it ignores them
//! while mouse reporting is on). Mouse reporting is turned on and off with the
//! `mouse` option, each time round the event loop.

use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::App;
use crate::commands::Invocation;
use crate::editor::find::FindField;
use crate::editor::windows::SplitDirection;
use crate::editor::{Message, Mode, WindowId};
use crate::input::Lookup;
use crate::ui::{FindView, InputBoxView};

/// Lines the wheel scrolls at a time.
const WHEEL_LINES: usize = 3;

/// Clicks this close together on one cell make a double (or triple) click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// What a drag is doing, from where it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Drag {
    /// Selecting text in a window.
    Text(WindowId),
    /// Resizing the sidebar (its edge).
    Sidebar,
    /// Resizing the agent sidebar (its edge).
    Agent,
    /// Resizing the terminal panel (its title row), whose bottom is here.
    Panel { bottom: u16 },
    /// Resizing a window from the line after it (to its right or below).
    Split {
        window: WindowId,
        area: Rect,
        vertical: bool,
    },
    /// A tab dragged: along its row (reordering it), or to another window.
    Tab {
        window: WindowId,
        document: crate::editor::DocumentId,
    },
    /// A file or folder dragged from the tree's row `row`: onto a folder
    /// (moving it there), or into a window (opening it there).
    Tree {
        path: std::path::PathBuf,
        row: usize,
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
    /// The agent sidebar's name (lists the agents), its mode (lists the
    /// modes), a tool call's row (opens it), one of its question's
    /// options, its box, a setting's chip under the box (lists its
    /// values), a row of the slash menu, and the rest of it.
    AgentTitle,
    AgentMode,
    AgentChip(String),
    AgentSlash(usize),
    AgentTool(String),
    AgentOption(usize),
    AgentInput,
    Agent,
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
        self.pointer_at = Some(at);
        match event.kind {
            MouseEventKind::Moved => {
                self.pointer = self.pointer_for(at);
                // Ctrl (or ⌥, as for Ctrl+click) held: peek at the
                // definition.
                let peek = event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                self.pointer_moved(at, peek);
                return;
            }
            // With VS Code's keys, Ctrl+click adds a cursor there (or
            // takes the one there away).
            MouseEventKind::Down(MouseButton::Left)
                if self.editor.modeless()
                    && event.modifiers == KeyModifiers::CONTROL
                    && matches!(self.hit(at), Hit::Window(id) if id == self.editor.windows().active()) =>
            {
                self.add_cursor_at(at);
            }
            // Ctrl+click (Ctrl+Shift+click with VS Code's keys, or
            // ⌥+click: a Mac's terminal can take Ctrl+click for its
            // right-click) on a word: its definition.
            MouseEventKind::Down(MouseButton::Left)
                if (event.modifiers.contains(KeyModifiers::CONTROL)
                    || event.modifiers.contains(KeyModifiers::ALT))
                    && matches!(self.hit(at), Hit::Window(_)) =>
            {
                self.mouse_down(at);
                self.dragging = None;
                self.execute("goto_definition", Invocation::default());
            }
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(at),
            MouseEventKind::Down(MouseButton::Right) => self.right_click(at),
            MouseEventKind::Down(MouseButton::Middle) => {
                if let Some((window, id, _)) = self.tab_at(at) {
                    self.close_tab(window, id);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(at),
            MouseEventKind::Up(_) => self.mouse_up(at),
            MouseEventKind::ScrollDown => self.mouse_wheel(at, true),
            MouseEventKind::ScrollUp => self.mouse_wheel(at, false),
            _ => {}
        }
        self.after_popup_key();
        self.dispatch_events();
    }

    /// The pointer's shape over `at`: a CSS cursor name.
    pub fn pointer_for(&self, at: Position) -> &'static str {
        use crate::config::StatusElement as E;
        if let Some(element) = self.status_item_at(at) {
            let clickable = !matches!(
                element,
                E::Mode | E::Modified | E::PendingKeys | E::Recording | E::Separator | E::Spacer
            );
            return if clickable { "pointer" } else { "default" };
        }
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
            return if (list && popup.list.contains(at)) || self.popup_click(at).is_some() {
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
                Drag::Sidebar | Drag::Agent | Drag::Split { vertical: true, .. } => "ew-resize",
                _ => "ns-resize",
            };
        }
        match self.hit(at) {
            Hit::SidebarTab(_)
            | Hit::Activity(_)
            | Hit::SearchToggle(_)
            | Hit::AgentTitle
            | Hit::AgentMode
            | Hit::AgentChip(_)
            | Hit::AgentSlash(_)
            | Hit::AgentTool(_)
            | Hit::AgentOption(_) => "pointer",
            Hit::AgentInput => "text",
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
        if let Some(agent) = &map.agent
            && agent.area.contains(at)
        {
            if let Some(&(_, n)) = agent.options.iter().find(|(r, _)| r.contains(at)) {
                return Hit::AgentOption(n);
            }
            if let Some(&(_, i)) = agent.slash.iter().find(|(r, _)| r.contains(at)) {
                return Hit::AgentSlash(i);
            }
            if let Some((_, id)) = agent.chips.iter().find(|(r, _)| r.contains(at)) {
                return Hit::AgentChip(id.clone());
            }
            if agent.mode.is_some_and(|r| r.contains(at)) {
                return Hit::AgentMode;
            }
            if agent.title.contains(at) {
                return Hit::AgentTitle;
            }
            if agent.input.contains(at) {
                return Hit::AgentInput;
            }
            if agent.chat.contains(at)
                && let Some((_, id)) = agent.tools.iter().find(|(y, _)| *y == at.y)
            {
                return Hit::AgentTool(id.clone());
            }
            return Hit::Agent;
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
        if map.agent.as_ref().is_some_and(|a| a.edge.contains(at)) {
            return Some(Drag::Agent);
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
                | Mode::Keybindings
                | Mode::Panel
                | Mode::Agent
        )
    }

    /// A click in the keybinding editor: the search box, or a row (the
    /// selected one again: press new keys for it).
    fn click_in_keybindings(&mut self, at: Position) -> bool {
        let Some(map) = self.screen_map.keybindings.clone() else {
            return false;
        };
        if !map.area.contains(at)
            || !matches!(
                self.editor.mode(),
                Mode::Keybindings | Mode::Sidebar | Mode::Terminal | Mode::Normal
            )
        {
            return false;
        }
        self.input.reset();
        self.editor.set_mode(Mode::Keybindings);
        if self.editor.keybindings_recording() {
            return true;
        }
        if map.search.contains(at) {
            self.editor.keybindings_search();
        } else if map.list.contains(at) {
            let index = map.first + (at.y - map.list.y) as usize;
            let again = self
                .editor
                .keybindings()
                .is_some_and(|v| v.selected == index);
            self.editor.keybindings_select(index);
            if again {
                self.execute("keybindings_change", Invocation::default());
            }
        }
        true
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
        // A click in the hover leaves it be; anywhere else, it goes.
        if self.over_hover(at) {
            return;
        }
        self.hover_wait = None;
        self.editor.end_any_hover();
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
        // An open menu, popup, picker or input box over the windows: it
        // takes the click before what's under it (the tab bar, the
        // breadcrumbs...).
        if self.over_focused_box(at) && self.click_in_box(at) {
            return;
        }
        // The bottom panel's tabs.
        if let Some(tab) = self.panel_tab_at(at) {
            self.input.reset();
            if let Err(error) = self.editor.panel_show(tab) {
                self.editor.set_message(Message::error(error));
            }
            return;
        }
        // The status bar: its items do what they show.
        if let Some(element) = self.status_item_at(at) {
            if self.editor.popup().is_some() {
                self.editor.close_popup();
            }
            self.status_click(element, at);
            return;
        }
        // The breadcrumbs: a folder shows in the sidebar, a symbol is gone
        // to.
        if let Some((window, crumb)) = self.crumb_at(at) {
            use crate::ui::breadcrumbs::Crumb;
            self.input.reset();
            self.editor.focus_window(window);
            match crumb {
                Crumb::Folder(path) => self.editor.reveal_in_tree(&path),
                Crumb::File => {
                    if let Some(path) = self
                        .editor
                        .document
                        .path()
                        .map(std::path::Path::to_path_buf)
                    {
                        self.editor.reveal_in_tree(&path);
                    }
                }
                Crumb::Symbol(index) => self.editor.go_to_symbol(index),
            }
            return;
        }
        // The tab bar: a tab shows its buffer, its × closes it.
        if let Some((window, id, on_close)) = self.tab_at(at) {
            if self.editor.popup().is_some() {
                self.editor.close_popup();
            }
            if on_close {
                self.close_tab(window, id);
            } else {
                self.show_tab(window, id);
                self.dragging = Some(Drag::Tab {
                    window,
                    document: id,
                });
            }
            return;
        }
        // A hover-like popup goes with any click.
        if self.editor.mode() != Mode::Popup && self.editor.popup().is_some() {
            self.editor.close_popup();
        }
        if self.click_in_settings(at)
            || self.click_in_keybindings(at)
            || self.click_in_box(at)
            || !self.clicks_allowed()
        {
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
                self.editor.clear_extra_cursors();
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
                let dragged = self.editor.tree_row_on_screen(row);
                if let Err(error) = self.editor.sidebar_click_times(row, clicks) {
                    self.editor.set_message(Message::error(error));
                }
                self.editor.finish_command();
                if let Some(entry) = dragged {
                    self.dragging = Some(Drag::Tree {
                        path: entry.path,
                        row,
                    });
                }
            }
            Hit::Sidebar => self.editor.set_mode(Mode::Sidebar),
            Hit::AgentTitle => self.agent_click("agent_choose"),
            Hit::AgentMode => self.agent_click("agent_mode"),
            Hit::AgentChip(id) => {
                if let Err(error) = self.editor.agent_config_picker(Some(&id)) {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::AgentSlash(i) => {
                if let Err(error) = self.editor.agent_slash_pick(i) {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::AgentTool(id) => self.editor.agent_toggle_tool(&id),
            Hit::AgentOption(n) => {
                if let Err(error) = self.editor.agent_choose(n) {
                    self.editor.set_message(Message::error(error));
                }
            }
            Hit::AgentInput | Hit::Agent => {
                if self.editor.mode() != Mode::Agent {
                    self.editor.set_mode(Mode::Agent);
                }
            }
            Hit::Terminal => self.panel_click(at),
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
                    let list = self.editor.popup().is_some_and(|p| p.select);
                    if !list && area.list.contains(at) {
                        // A key shown in it (`y move it`): as if pressed.
                        if let Some(command) = self.popup_click(at) {
                            self.execute(&command, Invocation::default());
                        }
                    } else if area.list.contains(at) {
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

    /// Whether `at` is on the box that has focus: a popup (a menu...), the
    /// picker or an input box.
    fn over_focused_box(&self, at: Position) -> bool {
        let map = &self.screen_map;
        match self.editor.mode() {
            Mode::Popup => map.popup.as_ref().is_some_and(|p| p.area.contains(at)),
            Mode::Picker => map.picker.as_ref().is_some_and(|p| p.area.contains(at)),
            Mode::Input => map.input.is_some_and(|(area, _)| area.contains(at)),
            _ => false,
        }
    }

    /// The command a click at `at` in the popup runs: the one bound (in the
    /// popup) to the key shown there, if any.
    fn popup_click(&self, at: Position) -> Option<String> {
        let area = self.screen_map.popup?;
        if !area.list.contains(at) {
            return None;
        }
        let popup = self.editor.popup()?;
        // Its text starts a column in from the border.
        let col = at.x.checked_sub(area.list.x + 1)? as usize;
        let line = area.first + (at.y - area.list.y) as usize;
        let key = popup.key_at(line, col)?;
        [key.to_string(), format!("<{key}>")]
            .iter()
            .filter_map(|notation| crate::input::parse_keys(notation).ok())
            .find_map(|keys| match self.mode_keymap().lookup(Mode::Popup, &keys) {
                Lookup::Command(command) => Some(command.to_string()),
                _ => None,
            })
    }

    /// The bottom panel's tab under `at`.
    fn panel_tab_at(&self, at: Position) -> Option<crate::editor::panel::PanelTab> {
        let map = self.screen_map.panel.as_ref()?;
        if at.y != map.area.y {
            return None;
        }
        map.tabs
            .iter()
            .find(|(from, to, _)| (*from..*to).contains(&at.x))
            .map(|(_, _, tab)| *tab)
    }

    /// A click in the bottom panel: the terminal's focus, or a problem
    /// gone to.
    fn panel_click(&mut self, at: Position) {
        use crate::editor::panel::PanelTab;
        match self.editor.panel_shown() {
            Some(PanelTab::Terminal) | None => {
                if let Err(error) = self.editor.focus_terminal() {
                    self.editor.set_message(Message::error(error));
                }
            }
            Some(PanelTab::Problems | PanelTab::Locations) => {
                self.editor.set_mode(Mode::Panel);
                let Some(map) = self.screen_map.panel.clone() else {
                    return;
                };
                if map.list.contains(at) {
                    let index = map.first + (at.y - map.list.y) as usize;
                    let before = self.editor.panel().selected;
                    self.editor.panel_select(index);
                    if self.editor.panel().selected == index || before == index {
                        self.execute("panel_open", Invocation::default());
                    }
                }
            }
            Some(PanelTab::Output) => self.editor.set_mode(Mode::Panel),
        }
    }

    /// The status bar item under `at`.
    fn status_item_at(&self, at: Position) -> Option<crate::config::StatusElement> {
        let (row, items) = self.screen_map.status_bar.as_ref()?;
        if at.y != *row {
            return None;
        }
        items
            .iter()
            .find(|(_, from, to)| (*from..*to).contains(&at.x))
            .map(|(element, _, _)| element.clone())
    }

    /// A status bar item clicked, as in VS Code.
    fn status_click(&mut self, element: crate::config::StatusElement, at: Position) {
        use crate::config::StatusElement as E;
        use crate::editor::status_actions::{ENCODING_MENU, INDENT_MENU, LINE_ENDING_MENU};
        self.input.reset();
        let command = match element {
            E::Git => "git_branch",
            E::Diagnostics => "problems",
            E::Position | E::PositionPercentage | E::TotalLines => "goto_line_prompt",
            E::FileType => "language",
            E::LspProgress => "lsp_info",
            E::FileName | E::FileBaseName => "sidebar_focus",
            E::Indent => return self.editor.status_menu(INDENT_MENU, at.x, at.y),
            E::FileEncoding => return self.editor.status_menu(ENCODING_MENU, at.x, at.y),
            E::FileLineEnding => return self.editor.status_menu(LINE_ENDING_MENU, at.x, at.y),
            _ => return,
        };
        self.execute(command, Invocation::default());
    }

    /// The breadcrumb under `at`.
    fn crumb_at(&self, at: Position) -> Option<(WindowId, crate::ui::breadcrumbs::Crumb)> {
        let map = self
            .screen_map
            .breadcrumbs
            .iter()
            .find(|m| m.row.contains(at))?;
        map.crumbs
            .iter()
            .find(|(from, to, _)| (*from..*to).contains(&at.x))
            .map(|(_, _, crumb)| (map.window, crumb.clone()))
    }

    /// The tab under `at`, and whether it's on its `×`.
    fn tab_at(&self, at: Position) -> Option<(WindowId, crate::editor::DocumentId, bool)> {
        let map = self
            .screen_map
            .tab_bars
            .iter()
            .find(|m| m.row.contains(at))?;
        map.tabs
            .iter()
            .find(|(from, to, _, _)| (*from..*to).contains(&at.x))
            .map(|&(_, _, close, id)| (map.window, id, at.x == close))
    }

    /// A tab clicked: its window focused, showing its buffer, there to
    /// type in.
    fn show_tab(&mut self, window: WindowId, id: crate::editor::DocumentId) {
        self.input.reset();
        self.editor.close_views();
        self.editor.focus_window(window);
        self.editor.switch_to(id);
        if !matches!(self.editor.mode(), Mode::Normal | Mode::Insert) {
            self.editor.set_mode(Mode::Normal);
        }
    }

    /// A tab's `×` (or a middle click): closes its buffer (asking first,
    /// with VS Code's keys, if it has unsaved changes).
    fn close_tab(&mut self, window: WindowId, id: crate::editor::DocumentId) {
        self.show_tab(window, id);
        self.execute("tab_close", Invocation::default());
    }

    /// A right-click: a menu of what can be done there, at the pointer, as
    /// in a desktop editor. In the text it goes where the cursor is put
    /// (a selection stays, for Cut and Copy); in the file tree, on the
    /// entry clicked.
    fn right_click(&mut self, at: Position) {
        if self.editor.popup().is_some() {
            self.editor.close_popup();
        }
        if self
            .screen_map
            .menu_bar
            .as_ref()
            .is_some_and(|(y, _)| *y == at.y)
        {
            return;
        }
        match self.hit(at) {
            Hit::Window(_) if self.clicks_allowed() => {
                if self.editor.mode() != Mode::Visual {
                    self.mouse_down(at);
                    self.dragging = None;
                }
                self.editor.context_menu(false, at.x, at.y);
            }
            Hit::SidebarLine(row) if self.clicks_allowed() && self.editor.tree_select_line(row) => {
                self.editor.context_menu(true, at.x, at.y);
            }
            // Below the files: the project's folder.
            Hit::SidebarLine(_) | Hit::Sidebar
                if self.clicks_allowed() && self.editor.file_tree_shown() =>
            {
                self.editor.set_mode(Mode::Sidebar);
                self.editor.tree_root_menu(at.x, at.y);
            }
            _ => {}
        }
    }

    /// A click on the agent sidebar's name or mode: a list to choose from.
    fn agent_click(&mut self, command: &str) {
        self.execute(command, Invocation::default());
    }

    /// A click back into the text leaves the sidebar, the terminal or the
    /// agent's box.
    fn leave_panes(&mut self) {
        if matches!(
            self.editor.mode(),
            Mode::Sidebar | Mode::Terminal | Mode::Agent
        ) {
            self.editor.set_mode(Mode::Normal);
        }
    }

    /// Ctrl+click in the active window: a cursor added there (or the one
    /// there taken away).
    fn add_cursor_at(&mut self, at: Position) {
        let id = self.editor.windows().active();
        let document = self.editor.document.id();
        let Some(state) = self.views.get(&(id, document)) else {
            return;
        };
        let tab_width = self.editor.options_for(id, document).tab_width;
        if let Some(position) = state.position_at(&self.editor.document, at.x, at.y, tab_width) {
            self.editor.toggle_cursor_at(position);
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
        match self.dragging.clone() {
            Some(Drag::Tab { window, document }) => {
                self.pointer = "grabbing";
                self.drag_tab(window, document, at);
                return;
            }
            Some(Drag::Tree { row, .. }) => {
                self.pointer = "grabbing";
                // The row it'd drop on, selected.
                if let Hit::SidebarLine(over) = self.hit(at)
                    && over != row
                {
                    self.editor.tree_select_on_screen(over);
                }
                return;
            }
            _ => {}
        }
        match self.dragging {
            Some(Drag::Text(id)) => self.drag_select(id, at),
            Some(Drag::Sidebar) => {
                if let Some(sidebar) = &self.screen_map.sidebar {
                    self.editor.options.sidebar_width = at.x.saturating_sub(sidebar.area.x).max(10);
                }
            }
            Some(Drag::Agent) => {
                if let Some(agent) = &self.screen_map.agent {
                    let width = agent.area.right().saturating_sub(at.x + 1);
                    self.editor.options.agent_width = width.max(24);
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
            Some(Drag::Tab { .. } | Drag::Tree { .. }) | None => {}
        }
    }

    /// A tab dragged along its own row: it takes the place of the tab
    /// it's over, once the pointer's where it would then be (so tabs of
    /// different widths don't swap back and forth).
    fn drag_tab(&mut self, window: WindowId, document: crate::editor::DocumentId, at: Position) {
        let Some(map) = self
            .screen_map
            .tab_bars
            .iter()
            .find(|m| m.window == window && m.row.contains(at))
        else {
            return;
        };
        let Some(&(from, _, _, _)) = map.tabs.iter().find(|t| t.3 == document) else {
            return;
        };
        let width = map
            .tabs
            .iter()
            .find(|t| t.3 == document)
            .map_or(0, |t| t.1 - t.0);
        let Some(&(over_from, over_to, _, over)) =
            map.tabs.iter().find(|t| (t.0..t.1).contains(&at.x))
        else {
            return;
        };
        if over == document {
            return;
        }
        let lands = if over_from > from {
            at.x >= over_to.saturating_sub(width)
        } else {
            at.x < over_from + width
        };
        if !lands {
            return;
        }
        let tabs = self.editor.window_tabs(window);
        if let Some(index) = tabs.iter().position(|&t| t == over) {
            self.editor.move_tab(window, document, index);
        }
    }

    /// The button let go: a dragged tab or tree entry dropped.
    fn mouse_up(&mut self, at: Position) {
        let dragging = self.dragging.take();
        self.pointer = self.pointer_for(at);
        match dragging {
            Some(Drag::Tab { window, document }) => {
                // On another window's tab row (before the tab it's over),
                // or in its text (at the end).
                let row = self
                    .screen_map
                    .tab_bars
                    .iter()
                    .find(|m| m.row.contains(at))
                    .map(|m| {
                        let over = m
                            .tabs
                            .iter()
                            .find(|t| (t.0..t.1).contains(&at.x))
                            .map(|t| t.3);
                        (m.window, over)
                    });
                let target = match row {
                    Some((to, over)) => Some((
                        to,
                        over.and_then(|o| self.editor.window_tabs(to).iter().position(|&t| t == o)),
                    )),
                    None => match self.hit(at) {
                        Hit::Window(to) => Some((to, None)),
                        _ => None,
                    },
                };
                if let Some((to, index)) = target
                    && to != window
                {
                    self.editor.move_tab_to_window(document, window, to, index);
                    if !matches!(self.editor.mode(), Mode::Normal | Mode::Insert) {
                        self.editor.set_mode(Mode::Normal);
                    }
                }
            }
            Some(Drag::Tree { path, row }) => match self.hit(at) {
                Hit::SidebarLine(over) if over == row => {}
                Hit::SidebarLine(over) if self.editor.tree_row_on_screen(over).is_some() => {
                    let Some(target) = self.editor.tree_row_on_screen(over) else {
                        return;
                    };
                    let folder = if target.is_dir {
                        target.path
                    } else {
                        match target.path.parent() {
                            Some(parent) => parent.to_path_buf(),
                            None => return,
                        }
                    };
                    if let Err(error) = self.editor.tree_ask_move(path, folder) {
                        self.editor.set_message(Message::error(error));
                    }
                }
                // Below the files (or on the sidebar's title): the
                // project's folder.
                Hit::SidebarLine(_) | Hit::Sidebar if self.editor.file_tree_shown() => {
                    let Some(root) = self.editor.sidebar().map(|e| e.root().to_path_buf()) else {
                        return;
                    };
                    if let Err(error) = self.editor.tree_ask_move(path, root) {
                        self.editor.set_message(Message::error(error));
                    }
                }
                Hit::Window(id) if path.is_file() => {
                    self.leave_panes();
                    self.editor.focus_window(id);
                    if let Err(error) = self.editor.open(&path) {
                        self.editor.set_message(Message::error(error.to_string()));
                    }
                }
                _ => {}
            },
            _ => {}
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
        // Over the hover, it scrolls; elsewhere the text moves from under
        // it, so it goes.
        if self.over_hover(at) {
            self.editor.scroll_hover_by(3 * sign);
            return;
        }
        self.hover_wait = None;
        self.editor.end_any_hover();
        if self
            .screen_map
            .agent
            .as_ref()
            .is_some_and(|a| a.chat.contains(at))
        {
            self.editor.agent_scroll(-sign * WHEEL_LINES as isize);
            return;
        }
        if self
            .screen_map
            .settings
            .as_ref()
            .is_some_and(|s| s.list.contains(at))
        {
            self.editor.settings_move(sign);
            return;
        }
        if self
            .screen_map
            .keybindings
            .as_ref()
            .is_some_and(|k| k.list.contains(at))
        {
            self.editor.keybindings_move(sign * WHEEL_LINES as isize);
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
            Hit::Terminal => match self.editor.panel_shown() {
                Some(crate::editor::panel::PanelTab::Terminal) => {
                    self.editor.terminal_scroll_rows(-lines);
                }
                _ => self.editor.panel_move(lines),
            },
            Hit::Activity(_)
            | Hit::ActivityBar
            | Hit::SearchField
            | Hit::SearchReplace
            | Hit::SearchReplaceAll
            | Hit::SearchToggle(_)
            | Hit::AgentTitle
            | Hit::AgentMode
            | Hit::AgentChip(_)
            | Hit::AgentSlash(_)
            | Hit::AgentTool(_)
            | Hit::AgentOption(_)
            | Hit::AgentInput
            | Hit::Agent
            | Hit::Nothing => {}
        }
    }
}
