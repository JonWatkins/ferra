//! The sidebar, left of the windows: the file tree (`<Space>e`).
//!
//! ```text
//!  FERRA                │
//!  ▾ src                │ fn main() {
//!    ▸ ui               │
//!      main.rs      ●   │
//!    Cargo.toml         │
//! ```
//!
//! The selected row is highlighted while the sidebar has focus; the current
//! file's row is marked always, and unsaved files have a `●`.

use std::path::{Path, PathBuf};

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    widgets::Widget,
};

use super::text::truncate_end;
use crate::config::Theme;
use crate::editor::explorer::Explorer;
use crate::editor::sidebar_views::PluginView;

pub struct SidebarView<'a> {
    explorer: &'a Explorer,
    theme: &'a Theme,
    focused: bool,
    current: Option<PathBuf>,
    modified: Vec<PathBuf>,
}

/// Columns before a row's name: a space, two per level, then the folder
/// marker (or two spaces for a file).
fn name_column(depth: usize) -> usize {
    1 + 2 * depth + 2
}

impl<'a> SidebarView<'a> {
    pub fn new(explorer: &'a Explorer, theme: &'a Theme) -> Self {
        Self {
            explorer,
            theme,
            focused: false,
            current: None,
            modified: Vec::new(),
        }
    }

    /// Whether the sidebar has focus (the selection is shown).
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// The current file, marked in the tree.
    pub fn current(mut self, path: Option<&Path>) -> Self {
        self.current = path.map(canonical);
        self
    }

    /// Files with unsaved changes.
    pub fn modified<'p>(mut self, paths: impl IntoIterator<Item = &'p Path>) -> Self {
        self.modified = paths.into_iter().map(canonical).collect();
        self
    }

    /// Where the terminal cursor goes while the sidebar has focus: at the
    /// selected row's name.
    pub fn cursor_position(explorer: &Explorer, area: Rect) -> Option<Position> {
        let list = list_area(area);
        let row = explorer.selected().checked_sub(explorer.scroll.get())?;
        let selected = explorer.selected_row()?;
        if row >= list.height as usize {
            return None;
        }
        let x = (list.x as usize + name_column(selected.depth)).min(list.right() as usize - 1);
        Some(Position::new(x as u16, list.y + row as u16))
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The rows under the title.
fn list_area(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y + 1.min(area.height),
        area.width,
        area.height.saturating_sub(1),
    )
}

impl Widget for SidebarView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = self.theme;
        buf.set_style(area, theme.get("ui.sidebar"));

        // The title: the project folder's name.
        let root = self.explorer.root();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_uppercase())
            .unwrap_or_else(|| root.display().to_string());
        let title = truncate_end(&format!(" {name}"), area.width as usize);
        buf.set_string(area.x, area.y, title, theme.get("ui.sidebar.title"));

        let list = list_area(area);
        if list.height == 0 {
            return;
        }
        let rows = self.explorer.rows();
        if rows.is_empty() {
            buf.set_string(
                list.x,
                list.y,
                truncate_end(" (no files)", list.width as usize),
                theme.get("ui.sidebar.empty"),
            );
            return;
        }
        let first = self.explorer.scroll_to_selected(list.height as usize);
        let width = list.width as usize;
        for (offset, (i, row)) in rows
            .iter()
            .enumerate()
            .skip(first)
            .take(list.height as usize)
            .enumerate()
        {
            let y = list.y + offset as u16;
            let line = Rect::new(list.x, y, list.width, 1);
            let is_current = self.current.as_deref() == Some(row.path.as_path());
            let modified = !row.is_dir && self.modified.contains(&row.path);

            let mut style = if row.is_dir {
                theme.get("ui.sidebar.folder")
            } else {
                theme.get("ui.sidebar.file")
            };
            if is_current {
                style = style.patch(theme.get("ui.sidebar.current"));
            }
            if self.focused && i == self.explorer.selected() {
                buf.set_style(line, theme.get("ui.sidebar.selected"));
            }

            let indent = " ".repeat(1 + 2 * row.depth);
            let marker = match (row.is_dir, row.open) {
                (true, true) => "▾ ",
                (true, false) => "▸ ",
                (false, _) => "  ",
            };
            // Room for the name, keeping two columns for the unsaved mark.
            let reserved = if modified { 2 } else { 0 };
            let room = width.saturating_sub(name_column(row.depth) + reserved);
            let start = format!("{indent}{marker}");
            let x = buf
                .set_stringn(list.x, y, &start, width, theme.get("ui.sidebar.marker"))
                .0;
            if room > 0 {
                buf.set_stringn(x, y, truncate_end(&row.name, room), room, style);
            }
            // In the last-but-one column, if the name gets any room.
            if modified && room > 0 {
                let mark_x = list.x + list.width - 2;
                buf.set_string(mark_x, y, "●", theme.get("ui.sidebar.modified"));
            }
        }
    }
}

/// The row of tabs at the top of the sidebar, when plugins add views:
/// the one shown highlighted.
pub struct SidebarTabs<'a> {
    titles: &'a [&'a str],
    shown: usize,
    focused: bool,
    theme: &'a Theme,
}

impl<'a> SidebarTabs<'a> {
    pub fn new(titles: &'a [&'a str], shown: usize, focused: bool, theme: &'a Theme) -> Self {
        Self {
            titles,
            shown,
            focused,
            theme,
        }
    }
}

impl Widget for SidebarTabs<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = self.theme;
        buf.set_style(
            Rect::new(area.x, area.y, area.width, 1),
            theme.get("ui.sidebar.tabs"),
        );
        let mut x = area.x + 1;
        for (i, title) in self.titles.iter().enumerate() {
            let style = match (i == self.shown, self.focused) {
                (true, true) => theme.get("ui.sidebar.tab.active"),
                (true, false) => theme.get("ui.sidebar.tab.shown"),
                _ => theme.get("ui.sidebar.tab"),
            };
            let label = format!(" {title} ");
            if x >= area.right() {
                break;
            }
            let room = (area.right() - x) as usize;
            x = buf.set_stringn(x, area.y, &label, room, style).0 + 1;
        }
    }
}

/// A plugin's view: its title, then its lines, the selected one
/// highlighted while the sidebar has focus.
pub struct PluginViewWidget<'a> {
    view: &'a PluginView,
    focused: bool,
    theme: &'a Theme,
}

impl<'a> PluginViewWidget<'a> {
    pub fn new(view: &'a PluginView, focused: bool, theme: &'a Theme) -> Self {
        Self {
            view,
            focused,
            theme,
        }
    }

    /// Where the terminal cursor goes while the sidebar has focus: the
    /// start of the selected line.
    pub fn cursor_position(view: &PluginView, area: Rect) -> Option<Position> {
        let list = list_area(area);
        let row = view.selected()?.checked_sub(view.scroll.get())?;
        (row < list.height as usize && list.width > 1)
            .then(|| Position::new(list.x + 1, list.y + row as u16))
    }
}

impl Widget for PluginViewWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = self.theme;
        let base = theme.get("ui.sidebar");
        buf.set_style(area, base);
        let title = truncate_end(
            &format!(" {}", self.view.spec.title.to_uppercase()),
            area.width as usize,
        );
        buf.set_string(area.x, area.y, title, theme.get("ui.sidebar.title"));

        let list = list_area(area);
        if list.height == 0 {
            return;
        }
        if self.view.lines.is_empty() {
            buf.set_string(
                list.x,
                list.y,
                truncate_end(" (nothing here)", list.width as usize),
                theme.get("ui.sidebar.empty"),
            );
            return;
        }
        let first = self.view.first_shown(list.height as usize);
        let width = list.width.saturating_sub(1) as usize;
        for (row, (index, line)) in self
            .view
            .lines
            .iter()
            .enumerate()
            .skip(first)
            .take(list.height as usize)
            .enumerate()
        {
            let y = list.y + row as u16;
            let line_style = if self.focused && self.view.selected() == Some(index) {
                let selected = base.patch(theme.get("ui.sidebar.selected"));
                buf.set_style(Rect::new(list.x, y, list.width, 1), selected);
                selected
            } else {
                base
            };
            let mut x = list.x + 1;
            let mut room = width;
            for span in line {
                if room == 0 {
                    break;
                }
                let style = match &span.style {
                    Some(scope) => line_style.patch(theme.get(scope)),
                    None => line_style,
                };
                let text = truncate_end(&span.text, room);
                let next = buf.set_stringn(x, y, &text, room, style).0;
                room = room.saturating_sub((next - x) as usize);
                x = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("a-rather-long-file-name.txt"), "").unwrap();
        dir
    }

    fn render(view: SidebarView, width: u16, height: u16) -> (Vec<String>, Buffer) {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let rows = (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        (rows, buf)
    }

    #[test]
    fn draws_the_title_and_the_tree_indented() {
        let dir = project();
        let root = dir.path().join("demo");
        let mut tree = Explorer::new(&root);
        tree.reveal(&root.join("src/main.rs"));
        let theme = Theme::default();
        let (rows, _) = render(SidebarView::new(&tree, &theme), 20, 5);
        assert_eq!(
            rows,
            [
                " DEMO               ",
                " ▾ src              ",
                "     main.rs        ",
                "   a-rather-long-fi…",
                "                    ",
            ]
        );
    }

    #[test]
    fn marks_unsaved_files_and_highlights_the_selection_when_focused() {
        let dir = project();
        let root = dir.path().join("demo");
        let mut tree = Explorer::new(&root);
        tree.reveal(&root.join("src/main.rs"));
        let theme = crate::config::Config::default().theme;
        let main = root.join("src/main.rs");
        let view = SidebarView::new(&tree, &theme)
            .focused(true)
            .modified([main.as_path()]);
        let (rows, buf) = render(view, 20, 4);
        assert_eq!(rows[2], "     main.rs      ● ");
        let selected = theme.get("ui.sidebar.selected").bg;
        assert!(selected.is_some());
        assert_eq!(buf[(0, 2)].bg, selected.unwrap(), "the whole row");
        assert_ne!(buf[(0, 1)].bg, selected.unwrap());

        let position = SidebarView::cursor_position(&tree, Rect::new(0, 0, 20, 4));
        assert_eq!(position, Some(Position::new(5, 2)), "at the name");
    }

    #[test]
    fn plugin_views_draw_their_title_lines_and_tabs() {
        use crate::editor::popup::PopupSpan;
        use crate::editor::sidebar_views::ViewSpec;
        let mut editor = crate::editor::Editor::default();
        editor.add_sidebar_views(vec![ViewSpec {
            name: "todos".into(),
            title: "Todos".into(),
            ..ViewSpec::default()
        }]);
        editor.open_sidebar_view("todos").unwrap();
        let line = |text: &str, style: Option<&str>| {
            vec![PopupSpan {
                text: text.into(),
                style: style.map(String::from),
            }]
        };
        editor.set_plugin_view_lines(
            "todos",
            vec![line("first", None), line("second", Some("keyword"))],
        );
        editor.sidebar_move(1);
        let theme = crate::config::Config::default().theme;
        let view = editor.plugin_view().unwrap();

        let area = Rect::new(0, 0, 20, 4);
        let mut buf = Buffer::empty(area);
        PluginViewWidget::new(view, true, &theme).render(area, &mut buf);
        let row = |y: u16| -> String { (0..20).map(|x| buf[(x, y)].symbol()).collect() };
        assert_eq!(row(0).trim_end(), " TODOS");
        assert_eq!(row(1).trim_end(), " first", "level with the title");
        assert_eq!(row(2).trim_end(), " second");
        assert_eq!(buf[(0, 2)].bg, theme.get("ui.sidebar.selected").bg.unwrap());
        assert_eq!(buf[(2, 2)].fg, theme.get("keyword").fg.unwrap());
        assert_eq!(
            PluginViewWidget::cursor_position(view, area),
            Some(Position::new(1, 2))
        );

        let (titles, shown) = editor.sidebar_tabs();
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        SidebarTabs::new(&titles, shown, true, &theme).render(Rect::new(0, 0, 20, 1), &mut buf);
        let tabs: String = (0..20).map(|x| buf[(x, 0)].symbol()).collect();
        assert_eq!(tabs.trim_end(), "  Files   Todos");
        assert_eq!(
            buf[(10, 0)].fg,
            theme.get("ui.sidebar.tab.active").fg.unwrap()
        );
    }

    #[test]
    fn scrolls_to_keep_the_selection_in_view() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(format!("f{i}.txt")), "").unwrap();
        }
        let mut tree = Explorer::new(dir.path());
        tree.move_by(8);
        let theme = Theme::default();
        let (rows, _) = render(SidebarView::new(&tree, &theme).focused(true), 12, 4);
        assert_eq!(rows[3].trim(), "f8.txt", "{rows:#?}");
        assert_eq!(
            SidebarView::cursor_position(&tree, Rect::new(0, 0, 12, 4)),
            Some(Position::new(3, 3))
        );
    }
}
