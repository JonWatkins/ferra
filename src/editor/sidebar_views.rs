//! Views in the sidebar besides the file tree: plugins add them with
//! `editor.view(name, {...})`. The sidebar shows one at a time, with a row
//! of tabs when there's more than the file tree (`<Tab>`/`<S-Tab>` in it
//! switch, `:sidebar_view name` goes to one).
//!
//! A plugin view is a list of styled lines (as in popups) that the plugin's
//! `lines` function makes. The editor can't call Lua, so it marks a view
//! *stale* when it's shown, on `R`, when the current buffer changes, after
//! the events the plugin named, or when the plugin asks; the app calls the
//! function for a stale view before drawing it. The view's own keys run
//! the plugin's commands, which read `ctx:view_selected()`.

use std::cell::Cell;

use super::popup::PopupLine;
use super::{DocumentId, Editor, Mode};
use crate::events::EventKind;

/// A plugin's view, as registered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewSpec {
    pub name: String,
    pub title: String,
    /// Its own keys: notation and command.
    pub keys: Vec<(String, String)>,
    /// Events after which its lines are made again.
    pub refresh: Vec<EventKind>,
}

#[derive(Debug, Default)]
pub struct PluginView {
    pub spec: ViewSpec,
    pub lines: Vec<PopupLine>,
    selected: usize,
    /// The first line shown.
    pub scroll: Cell<usize>,
    stale: bool,
    /// The buffer its lines were made for.
    made_for: Option<DocumentId>,
}

impl PluginView {
    pub fn selected(&self) -> Option<usize> {
        (!self.lines.is_empty()).then_some(self.selected.min(self.lines.len() - 1))
    }

    fn move_by(&mut self, delta: isize) {
        let last = self.lines.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last.max(0)) as usize;
    }

    /// The first line to show in `height` rows, keeping the selection in view.
    pub fn first_shown(&self, height: usize) -> usize {
        let mut top = self.scroll.get();
        if let Some(selected) = self.selected() {
            if selected < top {
                top = selected;
            } else if height > 0 && selected >= top + height {
                top = selected + 1 - height;
            }
        }
        top = top.min(self.lines.len().saturating_sub(height.max(1)));
        self.scroll.set(top);
        top
    }
}

/// Which view the sidebar shows.
#[derive(Debug, Clone, Copy)]
pub enum SidebarShows<'a> {
    Files,
    Plugin(&'a PluginView),
}

impl Editor {
    /// Adds plugins' views (after they load).
    pub fn add_sidebar_views(&mut self, specs: Vec<ViewSpec>) {
        for spec in specs {
            self.sidebar_views.retain(|v| v.spec.name != spec.name);
            self.sidebar_views.push(PluginView {
                spec,
                stale: true,
                ..PluginView::default()
            });
        }
    }

    /// The views' names: `files`, then the plugins'.
    pub fn sidebar_view_names(&self) -> Vec<&str> {
        std::iter::once("files")
            .chain(self.sidebar_views.iter().map(|v| v.spec.name.as_str()))
            .collect()
    }

    /// The views' titles, for the tabs, and which is shown.
    pub fn sidebar_tabs(&self) -> (Vec<&str>, usize) {
        let titles = std::iter::once("Files")
            .chain(self.sidebar_views.iter().map(|v| v.spec.title.as_str()))
            .collect();
        (titles, self.sidebar_view)
    }

    pub fn sidebar_shows(&self) -> SidebarShows<'_> {
        match self.sidebar_view.checked_sub(1) {
            Some(i) => self
                .sidebar_views
                .get(i)
                .map_or(SidebarShows::Files, SidebarShows::Plugin),
            None => SidebarShows::Files,
        }
    }

    fn plugin_view_mut(&mut self) -> Option<&mut PluginView> {
        let i = self.sidebar_view.checked_sub(1)?;
        self.sidebar_views.get_mut(i)
    }

    /// The plugin view shown, if one is.
    pub fn plugin_view(&self) -> Option<&PluginView> {
        match self.sidebar_shows() {
            SidebarShows::Plugin(view) => Some(view),
            SidebarShows::Files => None,
        }
    }

    /// Shows view `index` (0: the files), marking a plugin's to be made.
    fn show_view(&mut self, index: usize) {
        self.sidebar_view = index;
        if let Some(view) = self.plugin_view_mut() {
            view.stale = true;
        }
        self.view_keys_changed = true;
    }

    /// `<Tab>` / `<S-Tab>` in the sidebar: the next or previous view.
    pub fn sidebar_cycle_view(&mut self, delta: isize) {
        let count = 1 + self.sidebar_views.len() as isize;
        let next = (self.sidebar_view as isize + delta).rem_euclid(count);
        self.show_view(next as usize);
    }

    /// `:sidebar_view name`: opens the sidebar at that view, with focus.
    pub fn open_sidebar_view(&mut self, name: &str) -> Result<(), String> {
        let index = self
            .sidebar_view_names()
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| {
                format!(
                    "No sidebar view {name} (there's {})",
                    self.sidebar_view_names().join(", ")
                )
            })?;
        if self.mode() != Mode::Sidebar {
            self.toggle_sidebar_focus();
        }
        self.show_view(index);
        Ok(())
    }

    /// Moves the selection in the plugin view shown. Returns whether one is.
    pub(super) fn plugin_view_move(&mut self, delta: isize) -> bool {
        match self.plugin_view_mut() {
            Some(view) => {
                view.move_by(delta);
                true
            }
            None => false,
        }
    }

    /// Selects the line `row` rows down the plugin view's list, as drawn.
    pub(super) fn select_plugin_view_line_on_screen(&mut self, row: usize) {
        if let Some(view) = self.plugin_view_mut() {
            let index = view.scroll.get() + row;
            if index < view.lines.len() {
                view.selected = index;
            }
        }
    }

    /// `R`, or `ctx:view_refresh()`: makes the view `name` (or the one shown)
    /// again.
    pub fn refresh_plugin_view(&mut self, name: Option<&str>) {
        let shown = self.plugin_view().map(|v| v.spec.name.clone());
        let name = name.map(str::to_string).or(shown);
        for view in &mut self.sidebar_views {
            if Some(&view.spec.name) == name.as_ref() {
                view.stale = true;
            }
        }
    }

    /// After an event: views that asked for it are made again.
    pub fn plugin_views_saw(&mut self, kind: EventKind) {
        for view in &mut self.sidebar_views {
            if view.spec.refresh.contains(&kind) {
                view.stale = true;
            }
        }
    }

    /// The plugin view to make before drawing: the one shown, if it's
    /// stale or was made for another buffer.
    pub fn plugin_view_to_make(&self) -> Option<String> {
        self.sidebar()?;
        let view = self.plugin_view()?;
        (view.stale || view.made_for != Some(self.document.id())).then(|| view.spec.name.clone())
    }

    /// The lines its function made (keeping the selection where it can).
    pub fn set_plugin_view_lines(&mut self, name: &str, lines: Vec<PopupLine>) {
        let document = self.document.id();
        if let Some(view) = self.sidebar_views.iter_mut().find(|v| v.spec.name == name) {
            view.lines = lines;
            view.selected = view.selected.min(view.lines.len().saturating_sub(1));
            view.stale = false;
            view.made_for = Some(document);
        }
    }

    /// Whether the plugin view's keys need binding again (it changed).
    pub fn take_view_keys_changed(&mut self) -> bool {
        std::mem::take(&mut self.view_keys_changed)
    }

    /// The keys of the plugin view shown, while the sidebar has focus.
    pub fn plugin_view_keys(&self) -> &[(String, String)] {
        self.plugin_view().map_or(&[], |v| v.spec.keys.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::popup::PopupSpan;

    fn spec(name: &str) -> ViewSpec {
        ViewSpec {
            name: name.into(),
            title: name.to_uppercase(),
            keys: vec![("<CR>".into(), "go".into())],
            refresh: vec![EventKind::AfterSave],
        }
    }

    fn lines(n: usize) -> Vec<PopupLine> {
        (0..n)
            .map(|i| {
                vec![PopupSpan {
                    text: format!("line {i}"),
                    style: None,
                }]
            })
            .collect()
    }

    #[test]
    fn views_are_switched_made_and_moved_about() {
        let mut editor = Editor::default();
        editor.add_sidebar_views(vec![spec("todos"), spec("outline")]);
        assert_eq!(editor.sidebar_view_names(), ["files", "todos", "outline"]);
        assert_eq!(editor.plugin_view_to_make(), None, "the sidebar's closed");

        editor.open_sidebar_view("todos").unwrap();
        assert_eq!(editor.mode(), Mode::Sidebar);
        assert!(editor.take_view_keys_changed());
        assert_eq!(editor.plugin_view_keys().len(), 1);
        assert_eq!(editor.plugin_view_to_make().as_deref(), Some("todos"));
        editor.set_plugin_view_lines("todos", lines(5));
        assert_eq!(editor.plugin_view_to_make(), None, "made");

        editor.sidebar_move(3);
        assert_eq!(editor.plugin_view().unwrap().selected(), Some(3));
        editor.sidebar_last();
        assert_eq!(editor.plugin_view().unwrap().selected(), Some(4));
        editor.set_plugin_view_lines("todos", lines(2));
        assert_eq!(
            editor.plugin_view().unwrap().selected(),
            Some(1),
            "kept in range"
        );

        editor.plugin_views_saw(EventKind::AfterSave);
        assert_eq!(editor.plugin_view_to_make().as_deref(), Some("todos"));
        editor.set_plugin_view_lines("todos", lines(2));

        editor.sidebar_cycle_view(1);
        assert_eq!(editor.plugin_view().unwrap().spec.name, "outline");
        editor.sidebar_cycle_view(1);
        assert!(
            matches!(editor.sidebar_shows(), SidebarShows::Files),
            "round to the files"
        );
        assert!(editor.plugin_view_keys().is_empty());
        editor.sidebar_cycle_view(-1);
        assert_eq!(editor.plugin_view().unwrap().spec.name, "outline");

        assert!(
            editor
                .open_sidebar_view("nope")
                .unwrap_err()
                .contains("files, todos, outline")
        );
    }
}
