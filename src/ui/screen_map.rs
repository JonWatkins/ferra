//! Where things were drawn, for the mouse ([`ScreenMap`]).
//!
//! As `ui::draw` paints the screen, each widget records the regions it
//! drew that the mouse can do something with, each with a [`Target`]
//! saying what it is: a window's text, a tab and its `×`, a row of the
//! picker. Regions are recorded in paint order, so what's painted later
//! is on top: [`ScreenMap::at`] answers with the last region recorded at
//! a cell. A box records its whole area first (its `Rest`), then its parts
//! over it, so a cell in a box never answers with what's under the box.
//!
//! The answer is all the mouse needs: a click, the wheel, a right-click,
//! a drop and the pointer's shape ([`Target::pointer`]) all go by it, so
//! they agree on what's where. Lists record a region for each row that has
//! an item, never for the empty rows under the last one.
//!
//! What a click on a target does depends on the mode too (a click outside
//! the picker closes it); that's for `app/mouse.rs`.

use ratatui::layout::{Position, Rect};

use crate::config::StatusElement;
use crate::editor::find::FindField;
use crate::editor::git_gui::GitPanel;
use crate::editor::panel::PanelTab;
use crate::editor::{DocumentId, WindowId};

use super::breadcrumbs::Crumb;

/// What's at a place on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The menu bar: a menu's title (which), or the rest of its row.
    MenuBar(Option<usize>),
    /// The activity bar: an icon's block (what it shows), or between them.
    Activity(Option<String>),
    Sidebar(SidebarPart),
    /// The line between the sidebar and the windows (dragged to resize it).
    SidebarEdge,
    /// The agent sidebar, on the right.
    Agent(AgentPart),
    /// The line between the agent sidebar and the windows.
    AgentEdge,
    /// A window's text (its tab row and breadcrumbs not included).
    Window(WindowId),
    /// A `▸` or `▾` in a window's fold column, and its line: a click
    /// opens or closes that fold.
    Fold {
        window: WindowId,
        row: usize,
    },
    /// The line after `window` (to its right if `vertical`, else under
    /// it): dragged to resize it.
    Split {
        window: WindowId,
        vertical: bool,
    },
    /// A window's tab row.
    TabBar(WindowId),
    /// A tab, and whether this is its `×`.
    Tab {
        window: WindowId,
        document: DocumentId,
        close: bool,
    },
    /// One of a window's breadcrumbs.
    Crumb {
        window: WindowId,
        crumb: Crumb,
    },
    /// The bottom panel.
    Panel(PanelPart),
    /// An item of the status bar.
    Status(StatusElement),
    /// A language server's hover.
    Hover,
    /// The settings editor.
    Settings(SettingsPart),
    /// The keybinding editor.
    Keybindings(KeybindingsPart),
    /// The find box.
    Find(FindPart),
    /// The git view.
    Git(GitPart),
    /// A plugin's popup (a menu, a list, a message).
    Popup(PopupPart),
    /// A plugin's input box: its field, or the rest of it.
    Input {
        field: bool,
    },
    /// The picker: a match's row (its index), or the rest of it.
    Picker(Option<usize>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    /// A tab of its row of tabs (which).
    Tab(usize),
    /// A row of its list with an item on it: rows down from the list's top.
    Line(usize),
    /// The Search view's query field, its replace field, the replace-all
    /// chip, and its toggles (match case, whole word, regex).
    SearchField,
    SearchReplace,
    SearchReplaceAll,
    SearchToggle(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    /// The agent's name (lists the agents) and its mode (lists the modes).
    Title,
    Mode,
    /// The chat's rows.
    Chat,
    /// A tool call's row (opens or closes it): its id.
    Tool(String),
    /// One of the question's options (from 1).
    Option(usize),
    /// The box typed in.
    Input,
    /// A setting's chip under the box (lists its values): its id.
    Chip(String),
    /// A row of the slash menu (its row in the list).
    Slash(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelPart {
    /// Anywhere else in it (recorded over all of it, title row included).
    Rest,
    /// Its title row (dragged to resize it).
    Title,
    /// A tab on the title row.
    Tab(PanelTab),
    /// The terminal's screen (the cells its program draws), while it's
    /// the tab shown.
    Terminal,
    /// A shell's row in the terminal's list (which, from 0).
    Shell(usize),
    /// The line left of the terminal's list (dragged to resize it).
    ListEdge,
    /// The terminal's title row buttons: a new shell, and ending the one
    /// shown.
    NewShell,
    EndShell,
    /// A row of the Problems' or Locations' list with an item: its index.
    Row(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    Search,
    /// A category's row (which).
    Category(usize),
    /// The settings' column (the wheel scrolls it).
    List,
    /// A setting's rows, and its value's (its index in the list).
    Item(usize),
    Value(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindingsPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    Search,
    /// The table's rows (the wheel scrolls them).
    List,
    /// A row with a binding on it: its index.
    Row(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    Field(FindField),
    /// A toggle: 0 match case, 1 whole word, 2 regex.
    Toggle(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    /// A panel (border included).
    Panel(GitPanel),
    /// A row of a panel's list: its index.
    Row(GitPanel, usize),
    /// A row of the blame (its line), or of the side-by-side view.
    BlameRow(usize),
    SideRow(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupPart {
    /// Anywhere else in it (recorded over all of it).
    Rest,
    /// A line of a list to choose from: its index.
    Row(usize),
    /// A key shown in it (`y move it`, all of it): as if pressed.
    Key(String),
}

impl Target {
    /// The pointer's shape over it: a CSS cursor name.
    pub fn pointer(&self) -> &'static str {
        use StatusElement as E;
        const HAND: &str = "pointer";
        const TEXT: &str = "text";
        const DEFAULT: &str = "default";
        match self {
            Self::MenuBar(Some(_)) | Self::Activity(Some(_)) | Self::Tab { .. } => HAND,
            Self::Crumb { .. } | Self::Fold { .. } => HAND,
            Self::MenuBar(None) | Self::Activity(None) | Self::TabBar(_) | Self::Hover => DEFAULT,
            Self::SidebarEdge
            | Self::AgentEdge
            | Self::Panel(PanelPart::ListEdge)
            | Self::Split { vertical: true, .. } => "ew-resize",
            Self::Split {
                vertical: false, ..
            }
            | Self::Panel(PanelPart::Title) => "ns-resize",
            Self::Window(_) | Self::Panel(PanelPart::Terminal) => TEXT,
            Self::Panel(
                PanelPart::Tab(_)
                | PanelPart::Row(_)
                | PanelPart::Shell(_)
                | PanelPart::NewShell
                | PanelPart::EndShell,
            ) => HAND,
            Self::Panel(PanelPart::Rest) => DEFAULT,
            Self::Status(element) => match element {
                E::Mode
                | E::Modified
                | E::PendingKeys
                | E::Recording
                | E::Separator
                | E::Spacer => DEFAULT,
                _ => HAND,
            },
            Self::Sidebar(part) => match part {
                SidebarPart::Rest => DEFAULT,
                SidebarPart::SearchField | SidebarPart::SearchReplace => TEXT,
                SidebarPart::Tab(_)
                | SidebarPart::Line(_)
                | SidebarPart::SearchReplaceAll
                | SidebarPart::SearchToggle(_) => HAND,
            },
            Self::Agent(part) => match part {
                AgentPart::Rest | AgentPart::Chat => DEFAULT,
                AgentPart::Input => TEXT,
                AgentPart::Title
                | AgentPart::Mode
                | AgentPart::Tool(_)
                | AgentPart::Option(_)
                | AgentPart::Chip(_)
                | AgentPart::Slash(_) => HAND,
            },
            Self::Settings(part) => match part {
                SettingsPart::Search => TEXT,
                SettingsPart::Category(_) | SettingsPart::Value(_) => HAND,
                SettingsPart::Rest | SettingsPart::List | SettingsPart::Item(_) => DEFAULT,
            },
            Self::Keybindings(part) => match part {
                KeybindingsPart::Search => TEXT,
                KeybindingsPart::Row(_) => HAND,
                KeybindingsPart::Rest | KeybindingsPart::List => DEFAULT,
            },
            Self::Find(part) => match part {
                FindPart::Field(_) => TEXT,
                FindPart::Toggle(_) => HAND,
                FindPart::Rest => DEFAULT,
            },
            Self::Git(part) => match part {
                GitPart::Row(panel, _) if *panel != GitPanel::Diff => HAND,
                _ => DEFAULT,
            },
            Self::Popup(part) => match part {
                PopupPart::Row(_) | PopupPart::Key(_) => HAND,
                PopupPart::Rest => DEFAULT,
            },
            Self::Input { field: true } => TEXT,
            Self::Input { field: false } => DEFAULT,
            Self::Picker(Some(_)) => HAND,
            Self::Picker(None) => DEFAULT,
        }
    }
}

/// A region drawn, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub area: Rect,
    pub target: Target,
}

/// Where things were drawn, in paint order: see the module's docs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenMap {
    regions: Vec<Region>,
}

impl ScreenMap {
    /// Records `target` drawn over `area` (on top of all recorded so far).
    pub fn add(&mut self, area: Rect, target: Target) {
        if !area.is_empty() {
            self.regions.push(Region { area, target });
        }
    }

    /// Records a list's rows: one for each of `list`'s rows with an item
    /// on it, the first showing item `first` of `len`.
    pub fn rows(&mut self, list: Rect, first: usize, len: usize, target: impl Fn(usize) -> Target) {
        let shown = len.saturating_sub(first).min(list.height as usize);
        for row in 0..shown {
            let area = Rect::new(list.x, list.y + row as u16, list.width, 1);
            self.add(area, target(first + row));
        }
    }

    /// What's on top at `at`, and the region it was drawn over.
    pub fn at(&self, at: Position) -> Option<&Region> {
        self.regions.iter().rev().find(|r| r.area.contains(at))
    }

    /// Where `target` was drawn (the last time, if more than once).
    pub fn area(&self, target: &Target) -> Option<Rect> {
        self.regions
            .iter()
            .rev()
            .find(|r| r.target == *target)
            .map(|r| r.area)
    }

    /// Every region, in paint order.
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(map: &ScreenMap, x: u16, y: u16) -> Option<Target> {
        map.at(Position::new(x, y)).map(|r| r.target.clone())
    }

    #[test]
    fn what_is_drawn_later_is_on_top() {
        let mut map = ScreenMap::default();
        map.add(Rect::new(0, 0, 20, 10), Target::Activity(None));
        map.add(Rect::new(5, 2, 10, 4), Target::Picker(None));
        map.add(Rect::new(6, 3, 8, 1), Target::Picker(Some(0)));
        assert_eq!(at(&map, 1, 1), Some(Target::Activity(None)));
        assert_eq!(at(&map, 5, 2), Some(Target::Picker(None)));
        assert_eq!(at(&map, 7, 3), Some(Target::Picker(Some(0))));
        assert_eq!(at(&map, 7, 4), Some(Target::Picker(None)));
    }

    #[test]
    fn nothing_drawn_there_is_nothing() {
        let mut map = ScreenMap::default();
        map.add(Rect::new(0, 0, 4, 4), Target::Hover);
        assert_eq!(at(&map, 4, 0), None);
        assert_eq!(at(&map, 0, 4), None);
    }

    #[test]
    fn an_empty_area_is_not_recorded() {
        let mut map = ScreenMap::default();
        map.add(Rect::new(0, 0, 0, 4), Target::Hover);
        assert!(map.regions().is_empty());
    }

    #[test]
    fn a_list_records_only_rows_with_items() {
        let mut map = ScreenMap::default();
        let list = Rect::new(2, 1, 10, 5);
        // Items 3 to 6 of 7: four rows, the fifth empty.
        map.rows(list, 3, 7, |i| Target::Picker(Some(i)));
        assert_eq!(at(&map, 2, 1), Some(Target::Picker(Some(3))));
        assert_eq!(at(&map, 11, 4), Some(Target::Picker(Some(6))));
        assert_eq!(at(&map, 2, 5), None);
        // More items than rows: only as many as fit.
        let mut map = ScreenMap::default();
        map.rows(list, 0, 100, |i| Target::Picker(Some(i)));
        assert_eq!(map.regions().len(), 5);
    }

    #[test]
    fn an_area_is_found_by_its_target() {
        let mut map = ScreenMap::default();
        map.add(Rect::new(0, 0, 4, 4), Target::Sidebar(SidebarPart::Rest));
        map.add(Rect::new(0, 1, 4, 1), Target::Sidebar(SidebarPart::Line(0)));
        assert_eq!(
            map.area(&Target::Sidebar(SidebarPart::Rest)),
            Some(Rect::new(0, 0, 4, 4))
        );
        assert_eq!(map.area(&Target::SidebarEdge), None);
    }

    #[test]
    fn pointers_over_what_can_be_clicked_are_hands() {
        use crate::editor::panel::PanelTab;
        for target in [
            Target::MenuBar(Some(0)),
            Target::Panel(PanelPart::Tab(PanelTab::Problems)),
            Target::Keybindings(KeybindingsPart::Row(0)),
            Target::Popup(PopupPart::Key("y".into())),
        ] {
            assert_eq!(target.pointer(), "pointer", "{target:?}");
        }
        assert_eq!(Target::MenuBar(None).pointer(), "default");
        assert_eq!(Target::Panel(PanelPart::Title).pointer(), "ns-resize");
    }
}
