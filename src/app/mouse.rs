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
//! `ui::draw` returns a `ScreenMap` (`ui/screen_map.rs`): the regions it
//! drew, in paint order, each with a `Target` saying what it is (a window's
//! text, a tab or its `×`, a row of the picker, the line between two
//! windows...). The app keeps the last one, and every mouse event starts
//! from `App::target_at`: what's on top at the cell. Clicks, the wheel,
//! right-clicks, drops and the pointer's shape all match on that one
//! answer, so they can't disagree about what's where. Text positions come
//! from the window's `ViewState`, which records each screen row as it's
//! drawn (`DrawnRow`: the buffer row, the chars on it, the display column
//! it starts at and the wrap indent), so `ViewState::position_at` maps a
//! cell back through wrapping, folds, tabs, wide chars and sideways
//! scrolling without laying anything out again.
//!
//! What a target does can depend on the mode. A click goes first to a box
//! with focus (`click_in_box`): the picker chooses or closes, a list popup
//! runs its `<CR>` command (a key shown in a popup runs what it's bound
//! to), the find box flips toggles and moves between fields (a click
//! outside it closes it and goes on to the text), an input box places its
//! cursor or cancels. Otherwise an edge starts a resize drag
//! (`Drag::Sidebar` sets `sidebar-width`, `Drag::Panel` `terminal-height`,
//! `Drag::Split` resizes the window before the line through
//! `resize_window`), and anything else is a click on a window, the sidebar,
//! the agent sidebar or the panel. A window click focuses it and sets the
//! cursor; double and triple clicks (same cell, within 400 ms) run
//! `visual_mode` + `inner_word` or `visual_line_mode`; a drag from it
//! starts visual mode and moves the cursor, kept inside the window.
//!
//! As the pointer moves, `App::pointer_for` names its shape from the target
//! (`Target::pointer`, CSS names: `pointer`, `text`, `ew-resize`,
//! `ns-resize`, `default`); the event loop
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
use crate::editor::git_gui::GitPanel;
use crate::editor::windows::SplitDirection;
use crate::editor::{Message, Mode, WindowId};
use crate::input::Lookup;
use crate::ui::screen_map::{
    AgentPart, FindPart, GitPart, KeybindingsPart, PanelPart, PopupPart, SettingsPart, SidebarPart,
    Target,
};
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
                    && self.target_at(at)
                        == Some(Target::Window(self.editor.windows().active())) =>
            {
                self.add_cursor_at(at);
            }
            // Ctrl+click (Ctrl+Shift+click with VS Code's keys, or
            // ⌥+click: a Mac's terminal can take Ctrl+click for its
            // right-click) on a word: its definition.
            MouseEventKind::Down(MouseButton::Left)
                if (event.modifiers.contains(KeyModifiers::CONTROL)
                    || event.modifiers.contains(KeyModifiers::ALT))
                    && matches!(self.target_at(at), Some(Target::Window(_))) =>
            {
                self.mouse_down(at);
                self.dragging = None;
                self.execute("goto_definition", Invocation::default());
            }
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(at),
            MouseEventKind::Down(MouseButton::Right) => self.right_click(at),
            MouseEventKind::Down(MouseButton::Middle) => {
                if let Some(Target::Tab {
                    window, document, ..
                }) = self.target_at(at)
                {
                    self.close_tab(window, document);
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

    /// What's on top at `at`, as the last frame drew it.
    pub fn target_at(&self, at: Position) -> Option<Target> {
        self.screen_map.at(at).map(|region| region.target.clone())
    }

    /// The pointer's shape over `at`: a CSS cursor name.
    pub fn pointer_for(&self, at: Position) -> &'static str {
        self.target_at(at)
            .map_or("default", |target| target.pointer())
    }

    /// The drag an edge starts: the sidebar's, the agent sidebar's, the
    /// panel's title row, or the line after a window.
    fn edge(&self, target: &Option<Target>) -> Option<Drag> {
        let map = &self.screen_map;
        match target.as_ref()? {
            Target::SidebarEdge => Some(Drag::Sidebar),
            Target::AgentEdge => Some(Drag::Agent),
            Target::Panel(PanelPart::Title) => Some(Drag::Panel {
                bottom: map.area(&Target::Panel(PanelPart::Rest))?.bottom(),
            }),
            &Target::Split { window, vertical } => Some(Drag::Split {
                window,
                area: map.area(&Target::Window(window))?,
                vertical,
            }),
            _ => None,
        }
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
    fn click_in_keybindings(&mut self, target: &Option<Target>) -> bool {
        let Some(Target::Keybindings(part)) = *target else {
            return false;
        };
        if !matches!(
            self.editor.mode(),
            Mode::Keybindings | Mode::Sidebar | Mode::Terminal | Mode::Normal
        ) {
            return false;
        }
        self.input.reset();
        self.editor.set_mode(Mode::Keybindings);
        if self.editor.keybindings_recording() {
            return true;
        }
        match part {
            KeybindingsPart::Search => self.editor.keybindings_search(),
            KeybindingsPart::Row(index) => {
                let again = self
                    .editor
                    .keybindings()
                    .is_some_and(|v| v.selected == index);
                self.editor.keybindings_select(index);
                if again {
                    self.execute("keybindings_change", Invocation::default());
                }
            }
            KeybindingsPart::Rest | KeybindingsPart::List => {}
        }
        true
    }

    /// A click in the settings editor: a category shows its settings, a
    /// setting is selected, and one on its value changes it (as `<CR>`).
    fn click_in_settings(&mut self, target: &Option<Target>) -> bool {
        let Some(Target::Settings(part)) = *target else {
            return false;
        };
        if !matches!(
            self.editor.mode(),
            Mode::Settings | Mode::Sidebar | Mode::Terminal | Mode::Normal
        ) {
            return false;
        }
        self.input.reset();
        self.editor.set_mode(Mode::Settings);
        match part {
            SettingsPart::Search => self.editor.settings_search(),
            SettingsPart::Category(i) => self.editor.settings_category(i),
            SettingsPart::Item(i) => self.editor.settings_select(i),
            SettingsPart::Value(i) => {
                self.editor.settings_select(i);
                self.execute("settings_change", Invocation::default());
            }
            SettingsPart::Rest | SettingsPart::List => {}
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
        let target = self.target_at(at);
        // The menu bar first: a title opens its menu (another one's open
        // menu switching to it), the open one's closes it.
        match target {
            Some(Target::MenuBar(Some(index))) => {
                let name = crate::editor::menu_bar::MENUS[index].title.to_lowercase();
                self.execute(&format!("menu_{name}"), Invocation::default());
                return;
            }
            Some(Target::MenuBar(None)) => {
                if self.editor.open_menu().is_some() {
                    self.execute("popup_close", Invocation::default());
                }
                return;
            }
            _ => {}
        }
        // An open menu, popup, picker or input box over the windows: it
        // takes the click before what's under it (the tab bar, the
        // breadcrumbs...).
        if self.over_focused_box(&target) && self.click_in_box(at, &target) {
            return;
        }
        match target.clone() {
            // The bottom panel's tabs.
            Some(Target::Panel(PanelPart::Tab(tab))) => {
                self.input.reset();
                if let Err(error) = self.editor.panel_show(tab) {
                    self.editor.set_message(Message::error(error));
                }
                return;
            }
            // The status bar: its items do what they show.
            Some(Target::Status(element)) => {
                if self.editor.popup().is_some() {
                    self.editor.close_popup();
                }
                self.status_click(element, at);
                return;
            }
            // The breadcrumbs: a folder shows in the sidebar, a symbol is
            // gone to.
            Some(Target::Crumb { window, crumb }) => {
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
            Some(Target::Tab {
                window,
                document,
                close,
            }) => {
                if self.editor.popup().is_some() {
                    self.editor.close_popup();
                }
                if close {
                    self.close_tab(window, document);
                } else {
                    self.show_tab(window, document);
                    self.dragging = Some(Drag::Tab { window, document });
                }
                return;
            }
            _ => {}
        }
        // A hover-like popup goes with any click.
        if self.editor.mode() != Mode::Popup && self.editor.popup().is_some() {
            self.editor.close_popup();
        }
        if self.click_in_settings(&target)
            || self.click_in_keybindings(&target)
            || self.click_in_box(at, &target)
            || !self.clicks_allowed()
        {
            return;
        }
        self.input.reset();
        if let Some(edge) = self.edge(&target) {
            if let Drag::Panel { .. } = edge
                && let Err(error) = self.editor.focus_terminal()
            {
                self.editor.set_message(Message::error(error));
            }
            self.dragging = Some(edge);
            return;
        }
        let clicks = self.count_click(at);
        match target {
            Some(Target::Window(id)) => {
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
            Some(Target::Activity(Some(name))) => {
                self.leave_panes();
                if let Err(error) = self.editor.activity(&name) {
                    self.editor.set_message(Message::error(error));
                }
            }
            Some(Target::Sidebar(part)) => self.sidebar_click(part, clicks),
            Some(Target::Agent(part)) => self.agent_click(part),
            Some(Target::Panel(part)) => self.panel_click(part),
            _ => {}
        }
    }

    /// A click in the sidebar: it gets focus; a tab shows its view, a line
    /// is chosen (and can be dragged, in the file tree).
    fn sidebar_click(&mut self, part: SidebarPart, clicks: u8) {
        match part {
            SidebarPart::SearchField => self.editor.search_focus_query(),
            SidebarPart::SearchReplace => self.editor.search_focus_replace(),
            SidebarPart::SearchReplaceAll => {
                if let Err(error) = self.editor.search_ask_replace_all() {
                    self.editor.set_message(Message::error(error));
                }
            }
            SidebarPart::SearchToggle(i) => {
                // Toggled as the keys do, from the query field.
                self.editor.search_focus_query();
                let command = ["find_toggle_case", "find_toggle_word", "find_toggle_regex"][i];
                self.execute(command, Invocation::default());
            }
            SidebarPart::Tab(index) => {
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
            SidebarPart::Line(row) => {
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
            SidebarPart::Rest => self.editor.set_mode(Mode::Sidebar),
        }
    }

    /// A click in the agent sidebar: its name and mode list what to choose
    /// from, a setting's chip its values; anywhere else gives its box focus.
    fn agent_click(&mut self, part: AgentPart) {
        let result = match part {
            AgentPart::Title => {
                self.execute("agent_choose", Invocation::default());
                Ok(())
            }
            AgentPart::Mode => {
                self.execute("agent_mode", Invocation::default());
                Ok(())
            }
            AgentPart::Chip(id) => self.editor.agent_config_picker(Some(&id)),
            AgentPart::Slash(i) => self.editor.agent_slash_pick(i),
            AgentPart::Tool(id) => {
                self.editor.agent_toggle_tool(&id);
                Ok(())
            }
            AgentPart::Option(n) => self.editor.agent_choose(n),
            AgentPart::Input | AgentPart::Chat | AgentPart::Rest => {
                if self.editor.mode() != Mode::Agent {
                    self.editor.set_mode(Mode::Agent);
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            self.editor.set_message(Message::error(error));
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
    /// an input box, the git view). Returns whether the click is used up.
    fn click_in_box(&mut self, at: Position, target: &Option<Target>) -> bool {
        let drawn = |t: Target| self.screen_map.area(&t).is_some();
        match self.editor.mode() {
            Mode::Picker => {
                match *target {
                    Some(Target::Picker(Some(index))) => {
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
                    }
                    Some(Target::Picker(None)) => {}
                    _ if drawn(Target::Picker(None)) => {
                        self.execute("picker_cancel", Invocation::default());
                    }
                    _ => {}
                }
                true
            }
            Mode::Popup => {
                match target {
                    // A key shown in it (`y move it`): as if pressed.
                    Some(Target::Popup(PopupPart::Key(key))) => {
                        if let Some(command) = self.popup_command(key) {
                            self.execute(&command, Invocation::default());
                        }
                    }
                    &Some(Target::Popup(PopupPart::Row(index))) => {
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
                    }
                    Some(Target::Popup(PopupPart::Rest)) => {}
                    _ if drawn(Target::Popup(PopupPart::Rest)) => {
                        self.execute("popup_close", Invocation::default());
                    }
                    _ => {}
                }
                true
            }
            Mode::Input => {
                match target {
                    Some(Target::Input { field: true }) => {
                        let area = self.screen_map.area(&Target::Input { field: false });
                        if let (Some(input), Some(area)) = (self.editor.input_box(), area) {
                            let col = InputBoxView::field_col(input, area, at.x);
                            self.editor.input_box_edit(|f| f.set_cursor(col));
                        }
                    }
                    Some(Target::Input { field: false }) => {}
                    _ if drawn(Target::Input { field: false }) => {
                        self.execute("input_cancel", Invocation::default());
                    }
                    _ => {}
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
                let Some(Target::Find(part)) = *target else {
                    if !drawn(Target::Find(FindPart::Rest)) {
                        return true;
                    }
                    // Out of the box: it closes, and the click goes on.
                    self.editor.close_find();
                    return false;
                };
                match part {
                    FindPart::Toggle(toggle) => self.editor.find_toggle(|f| match toggle {
                        0 => f.match_case = !f.match_case,
                        1 => f.whole_word = !f.whole_word,
                        _ => f.regex = !f.regex,
                    }),
                    FindPart::Field(which) => {
                        let Some(area) = self.screen_map.at(at).map(|r| r.area) else {
                            return true;
                        };
                        let state = self.editor.find_state();
                        let field = match which {
                            FindField::Find => &state.find,
                            FindField::Replace => &state.replace,
                        };
                        let col = FindView::field_col(field, area, at.x);
                        self.editor.find_click_field(which, col);
                    }
                    FindPart::Rest => {}
                }
                true
            }
            Mode::Git => {
                match *target {
                    Some(Target::Git(GitPart::BlameRow(line))) => {
                        self.editor.git_gui_blame_select(line);
                    }
                    Some(Target::Git(GitPart::SideRow(row))) => {
                        self.editor.git_gui_side_select(row);
                    }
                    Some(Target::Git(GitPart::Row(panel, index))) => {
                        self.editor.git_gui_select(panel, index);
                    }
                    Some(Target::Git(GitPart::Panel(panel))) => self.editor.git_gui_focus(panel),
                    _ => {}
                }
                true
            }
            // The `:` line, or an operator waiting: clicks wait too.
            Mode::Command | Mode::OperatorPending => true,
            _ => false,
        }
    }

    /// Whether `target` is the box that has focus: a popup (a menu...),
    /// the picker or an input box.
    fn over_focused_box(&self, target: &Option<Target>) -> bool {
        match self.editor.mode() {
            Mode::Popup => matches!(target, Some(Target::Popup(_))),
            Mode::Picker => matches!(target, Some(Target::Picker(_))),
            Mode::Input => matches!(target, Some(Target::Input { .. })),
            _ => false,
        }
    }

    /// The command `key`, shown in the popup, runs there (if it's bound).
    fn popup_command(&self, key: &str) -> Option<String> {
        [key.to_string(), format!("<{key}>")]
            .iter()
            .filter_map(|notation| crate::input::parse_keys(notation).ok())
            .find_map(|keys| match self.mode_keymap().lookup(Mode::Popup, &keys) {
                Lookup::Command(command) => Some(command.to_string()),
                _ => None,
            })
    }

    /// A click in the bottom panel: the terminal's focus, or a problem
    /// gone to.
    fn panel_click(&mut self, part: PanelPart) {
        use crate::editor::panel::PanelTab;
        match self.editor.panel_shown() {
            Some(PanelTab::Terminal) | None => {
                if let Err(error) = self.editor.focus_terminal() {
                    self.editor.set_message(Message::error(error));
                }
            }
            Some(PanelTab::Problems | PanelTab::Locations) => {
                self.editor.set_mode(Mode::Panel);
                if let PanelPart::Row(index) = part {
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
        match self.target_at(at) {
            Some(Target::Window(_)) if self.clicks_allowed() => {
                if self.editor.mode() != Mode::Visual {
                    self.mouse_down(at);
                    self.dragging = None;
                }
                self.editor.context_menu(false, at.x, at.y);
            }
            Some(Target::Sidebar(SidebarPart::Line(row)))
                if self.clicks_allowed() && self.editor.tree_select_line(row) =>
            {
                self.editor.context_menu(true, at.x, at.y);
            }
            // Below the files: the project's folder.
            Some(Target::Sidebar(SidebarPart::Line(_) | SidebarPart::Rest))
                if self.clicks_allowed() && self.editor.file_tree_shown() =>
            {
                self.editor.set_mode(Mode::Sidebar);
                self.editor.tree_root_menu(at.x, at.y);
            }
            _ => {}
        }
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
                if let Some(Target::Sidebar(SidebarPart::Line(over))) = self.target_at(at)
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
                if let Some(sidebar) = self.screen_map.area(&Target::Sidebar(SidebarPart::Rest)) {
                    self.editor.options.sidebar_width = at.x.saturating_sub(sidebar.x).max(10);
                }
            }
            Some(Drag::Agent) => {
                if let Some(agent) = self.screen_map.area(&Target::Agent(AgentPart::Rest)) {
                    let width = agent.right().saturating_sub(at.x + 1);
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

    /// Where tab `document` of `window` was drawn (its `×` included).
    fn tab_area(&self, window: WindowId, document: crate::editor::DocumentId) -> Option<Rect> {
        self.screen_map.area(&Target::Tab {
            window,
            document,
            close: false,
        })
    }

    /// A tab dragged along its own row: it takes the place of the tab
    /// it's over, once the pointer's where it would then be (so tabs of
    /// different widths don't swap back and forth).
    fn drag_tab(&mut self, window: WindowId, document: crate::editor::DocumentId, at: Position) {
        let Some(Target::Tab {
            window: over_window,
            document: over,
            ..
        }) = self.target_at(at)
        else {
            return;
        };
        if over_window != window || over == document {
            return;
        }
        let (Some(mine), Some(theirs)) =
            (self.tab_area(window, document), self.tab_area(window, over))
        else {
            return;
        };
        let lands = if theirs.x > mine.x {
            at.x >= theirs.right().saturating_sub(mine.width)
        } else {
            at.x < theirs.x + mine.width
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
        let target = self.target_at(at);
        match dragging {
            Some(Drag::Tab { window, document }) => {
                // On another window's tab row (before the tab it's over),
                // or in its text (at the end).
                let place = match target {
                    Some(Target::Tab {
                        window: to,
                        document: over,
                        ..
                    }) => Some((
                        to,
                        self.editor.window_tabs(to).iter().position(|&t| t == over),
                    )),
                    Some(Target::TabBar(to) | Target::Window(to)) => Some((to, None)),
                    _ => None,
                };
                if let Some((to, index)) = place
                    && to != window
                {
                    self.editor.move_tab_to_window(document, window, to, index);
                    if !matches!(self.editor.mode(), Mode::Normal | Mode::Insert) {
                        self.editor.set_mode(Mode::Normal);
                    }
                }
            }
            Some(Drag::Tree { path, row }) => match target {
                Some(Target::Sidebar(SidebarPart::Line(over))) if over == row => {}
                Some(Target::Sidebar(SidebarPart::Line(over)))
                    if self.editor.tree_row_on_screen(over).is_some() =>
                {
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
                Some(Target::Sidebar(SidebarPart::Line(_) | SidebarPart::Rest))
                    if self.editor.file_tree_shown() =>
                {
                    let Some(root) = self.editor.sidebar().map(|e| e.root().to_path_buf()) else {
                        return;
                    };
                    if let Err(error) = self.editor.tree_ask_move(path, root) {
                        self.editor.set_message(Message::error(error));
                    }
                }
                Some(Target::Window(id)) if path.is_file() => {
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
        let Some(area) = self.screen_map.area(&Target::Window(id)) else {
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
        let target = self.target_at(at);
        match &target {
            Some(Target::Agent(
                AgentPart::Chat | AgentPart::Tool(_) | AgentPart::Option(_) | AgentPart::Slash(_),
            )) => {
                self.editor.agent_scroll(-sign * WHEEL_LINES as isize);
                return;
            }
            Some(Target::Settings(
                SettingsPart::List | SettingsPart::Item(_) | SettingsPart::Value(_),
            )) => {
                self.editor.settings_move(sign);
                return;
            }
            Some(Target::Keybindings(KeybindingsPart::List | KeybindingsPart::Row(_))) => {
                self.editor.keybindings_move(sign * WHEEL_LINES as isize);
                return;
            }
            _ => {}
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
                let lines = WHEEL_LINES as isize * sign;
                let whole = self
                    .editor
                    .git_gui()
                    .is_some_and(|g| g.blame.is_some() || g.side.is_some());
                let panel = match target {
                    Some(Target::Git(GitPart::Panel(panel) | GitPart::Row(panel, _))) => {
                        Some(panel)
                    }
                    _ => None,
                };
                match panel {
                    _ if whole => self.editor.git_gui_scroll(lines),
                    Some(GitPanel::Diff) => self.editor.git_gui_scroll(lines),
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
        match target {
            Some(Target::Window(id)) => {
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
            Some(Target::Sidebar(
                SidebarPart::Line(_) | SidebarPart::Rest | SidebarPart::Tab(_),
            )) => {
                if self.editor.search_view_shown() {
                    self.editor.search_scroll(lines);
                } else {
                    self.editor.sidebar_move(lines);
                }
            }
            Some(Target::Panel(_)) => match self.editor.panel_shown() {
                Some(crate::editor::panel::PanelTab::Terminal) => {
                    self.editor.terminal_scroll_rows(-lines);
                }
                _ => self.editor.panel_move(lines),
            },
            _ => {}
        }
    }
}
