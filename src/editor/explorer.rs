//! The file tree in the sidebar (the explorer): the project folder's files
//! and folders, folders opened and closed in place, as in VS Code's.
//!
//! Folders are listed as they're opened (`files::list_dir`: `.gitignore`d
//! files left out, as in the file picker) and kept until `refresh`. The
//! rows shown are the open folders' contents, depth first.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::files::{DirEntry, list_dir};

/// One row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// How deep in the tree (0: in the root folder).
    pub depth: usize,
    /// For a folder: whether it's open.
    pub open: bool,
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeAction {
    /// A folder was opened or closed.
    Toggled,
    /// A file to open in the editor.
    Open(PathBuf),
    Nothing,
}

#[derive(Debug, Clone, Default)]
pub struct Explorer {
    root: PathBuf,
    open: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<DirEntry>>,
    rows: Vec<TreeRow>,
    selected: usize,
    /// The first row shown (the view scrolls to keep the selected one in
    /// sight as it draws).
    pub scroll: std::cell::Cell<usize>,
}

impl Explorer {
    /// The tree of `root`, with its contents listed.
    pub fn new(root: &Path) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut explorer = Self {
            open: HashSet::from([root.clone()]),
            root,
            ..Self::default()
        };
        explorer.rebuild();
        explorer
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.selected)
    }

    fn listing(&mut self, dir: &Path) -> Vec<DirEntry> {
        self.listings
            .entry(dir.to_path_buf())
            .or_insert_with(|| list_dir(dir))
            .clone()
    }

    /// Lists the rows again from the open folders, keeping the selected path
    /// selected if it's still there.
    fn rebuild(&mut self) {
        let selected = self.selected_row().map(|r| r.path.clone());
        let mut rows = Vec::new();
        let root = self.root.clone();
        self.add_rows(&root, 0, &mut rows);
        self.rows = rows;
        if let Some(path) = selected {
            self.select_path(&path);
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn add_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
        for entry in self.listing(dir) {
            let open = entry.is_dir && self.open.contains(&entry.path);
            rows.push(TreeRow {
                path: entry.path.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir,
                depth,
                open,
            });
            if open {
                self.add_rows(&entry.path, depth + 1, rows);
            }
        }
    }

    /// Selects the row for `path`, if it's shown. Returns whether it is.
    fn select_path(&mut self, path: &Path) -> bool {
        match self.rows.iter().position(|r| r.path == path) {
            Some(i) => {
                self.selected = i;
                true
            }
            None => false,
        }
    }

    /// Moves the selection `delta` rows, stopping at the ends.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    /// Selects row `index` (clamped to the rows there are).
    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.rows.len().saturating_sub(1));
    }

    pub fn select_last(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    /// Enter / `l`: opens or closes the selected folder, or gives the file
    /// to open.
    pub fn choose(&mut self) -> TreeAction {
        let Some(row) = self.selected_row().cloned() else {
            return TreeAction::Nothing;
        };
        if !row.is_dir {
            return TreeAction::Open(row.path);
        }
        if !self.open.remove(&row.path) {
            self.open.insert(row.path);
        }
        self.rebuild();
        TreeAction::Toggled
    }

    /// `l` / `→` on a folder: opens it (or, if it's open, goes into it).
    /// On a file: gives it to open.
    pub fn expand(&mut self) -> TreeAction {
        let Some(row) = self.selected_row().cloned() else {
            return TreeAction::Nothing;
        };
        if !row.is_dir {
            return TreeAction::Open(row.path);
        }
        if row.open {
            // Into it, if it has anything.
            if self
                .rows
                .get(self.selected + 1)
                .is_some_and(|r| r.depth > row.depth)
            {
                self.selected += 1;
            }
        } else {
            self.open.insert(row.path);
            self.rebuild();
        }
        TreeAction::Toggled
    }

    /// `h` / `←`: closes the selected folder if it's open, else goes to the
    /// folder it's in.
    pub fn collapse(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if row.is_dir && row.open {
            self.open.remove(&row.path);
            self.rebuild();
        } else if let Some(parent) = row.path.parent() {
            self.select_path(parent);
        }
    }

    /// Closes every folder.
    pub fn collapse_all(&mut self) {
        self.open.retain(|p| *p == self.root);
        self.selected = 0;
        self.rebuild();
    }

    /// Opens the folders down to `path` (a file in the tree) and selects it.
    /// Returns whether it's in the tree (under the root, not ignored).
    pub fn reveal(&mut self, path: &Path) -> bool {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !path.starts_with(&self.root) {
            return false;
        }
        let mut changed = false;
        for ancestor in path.ancestors().skip(1) {
            if !ancestor.starts_with(&self.root) {
                break;
            }
            changed |= self.open.insert(ancestor.to_path_buf());
        }
        if changed {
            self.rebuild();
        }
        self.select_path(&path)
    }

    /// Lists the open folders again from disk (`R`, and after file changes).
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.open.retain(|p| p.is_dir());
        self.rebuild();
    }

    /// A file was written: lists its folder again if it's open and the file
    /// is new to it.
    pub fn saw_file(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(dir) = path.parent() else { return };
        let known = self
            .listings
            .get(dir)
            .is_none_or(|entries| entries.iter().any(|e| e.path == path));
        if !known {
            self.listings.remove(dir);
            self.rebuild();
        }
    }

    /// The first row to show in a view `height` rows tall, scrolled (and
    /// remembered) so the selected row is in it.
    pub fn scroll_to_selected(&self, height: usize) -> usize {
        let mut scroll = self.scroll.get();
        if height > 0 {
            if self.selected < scroll {
                scroll = self.selected;
            } else if self.selected >= scroll + height {
                scroll = self.selected + 1 - height;
            }
        }
        scroll = scroll.min(self.rows.len().saturating_sub(height.max(1)));
        self.scroll.set(scroll);
        scroll
    }
}

use super::{Editor, Message, Mode};

impl Editor {
    /// The file tree, if the sidebar is open.
    pub fn sidebar(&self) -> Option<&Explorer> {
        self.explorer.as_ref().filter(|_| self.sidebar_open)
    }

    fn explorer_mut(&mut self) -> &mut Explorer {
        let root = std::env::current_dir().unwrap_or_default();
        self.explorer.get_or_insert_with(|| Explorer::new(&root))
    }

    /// `<Space>e`: opens the sidebar and gives it focus, the current file
    /// selected; from the sidebar, back to the editor (leaving it open).
    pub fn toggle_sidebar_focus(&mut self) {
        if self.mode() == Mode::Sidebar {
            self.set_mode(Mode::Normal);
            return;
        }
        self.sidebar_open = true;
        let path = self.document.path().map(Path::to_path_buf);
        let explorer = self.explorer_mut();
        if let Some(path) = path {
            explorer.reveal(&path);
        }
        self.set_mode(Mode::Sidebar);
    }

    /// Opens or closes the sidebar (`:explorer`, `q` in it).
    pub fn toggle_sidebar(&mut self) {
        if self.sidebar_open {
            self.sidebar_open = false;
            if self.mode() == Mode::Sidebar {
                self.set_mode(Mode::Normal);
            }
        } else {
            self.toggle_sidebar_focus();
        }
    }

    pub fn sidebar_move(&mut self, delta: isize) {
        if !self.plugin_view_move(delta) {
            self.explorer_mut().move_by(delta);
        }
    }

    pub fn sidebar_first(&mut self) {
        if !self.plugin_view_move(isize::MIN / 2) {
            self.explorer_mut().select_first();
        }
    }

    pub fn sidebar_last(&mut self) {
        if !self.plugin_view_move(isize::MAX / 2) {
            self.explorer_mut().select_last();
        }
    }

    /// Enter: opens or closes the selected folder, or opens the file (and
    /// goes to it).
    pub fn sidebar_choose(&mut self) -> Result<(), String> {
        if self.plugin_view().is_some() {
            return Ok(());
        }
        let action = self.explorer_mut().choose();
        self.sidebar_act(action)
    }

    /// `l` / `→`: opens the folder (or goes into it), or opens the file.
    pub fn sidebar_expand(&mut self) -> Result<(), String> {
        if self.plugin_view().is_some() {
            return Ok(());
        }
        let action = self.explorer_mut().expand();
        self.sidebar_act(action)
    }

    fn sidebar_act(&mut self, action: TreeAction) -> Result<(), String> {
        if let TreeAction::Open(path) = action {
            // Named relative to the current folder, as the file picker does.
            let here = std::env::current_dir()
                .ok()
                .and_then(|d| std::fs::canonicalize(d).ok());
            let shown = here
                .and_then(|dir| path.strip_prefix(dir).ok().map(Path::to_path_buf))
                .unwrap_or(path);
            self.set_mode(Mode::Normal);
            self.open(&shown).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// A click on the sidebar's list, `row` rows down from its top: gives
    /// the sidebar focus and selects that line; in the file tree, opens the
    /// file or opens or closes the folder, as in VS Code.
    pub fn sidebar_click(&mut self, row: usize) -> Result<(), String> {
        self.set_mode(Mode::Sidebar);
        if self.plugin_view().is_some() {
            self.select_plugin_view_line_on_screen(row);
            return Ok(());
        }
        let explorer = self.explorer_mut();
        let index = explorer.scroll.get() + row;
        if index >= explorer.rows().len() {
            return Ok(());
        }
        explorer.select(index);
        self.sidebar_choose()
    }

    /// `h` / `←`: closes the folder, or goes to the one it's in.
    pub fn sidebar_collapse(&mut self) {
        if self.plugin_view().is_none() {
            self.explorer_mut().collapse();
        }
    }

    pub fn sidebar_collapse_all(&mut self) {
        if self.plugin_view().is_none() {
            self.explorer_mut().collapse_all();
        }
    }

    /// The files open with unsaved changes (marked in the tree).
    pub fn modified_paths(&self) -> Vec<&Path> {
        std::iter::once(&self.document)
            .chain(&self.background)
            .filter(|doc| doc.is_modified())
            .filter_map(|doc| doc.path())
            .collect()
    }

    /// After a save: lists the saved file's folder again, so a new file
    /// shows up.
    pub(super) fn sidebar_saw_save(&mut self, path: &Path) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.saw_file(path);
        }
    }

    /// `R`: lists the files again.
    pub fn sidebar_refresh(&mut self) {
        if self.plugin_view().is_some() {
            self.refresh_plugin_view(None);
            return;
        }
        self.explorer_mut().refresh();
        self.set_message(Message::info("Files listed again"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/ui")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("src/ui/view.rs"), "").unwrap();
        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::create_dir(root.join("docs")).unwrap();
        dir
    }

    fn shown(explorer: &Explorer) -> Vec<String> {
        explorer
            .rows()
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mark = if i == explorer.selected() { ">" } else { " " };
                let icon = match (r.is_dir, r.open) {
                    (true, true) => "v ",
                    (true, false) => "> ",
                    _ => "",
                };
                format!("{mark}{}{icon}{}", "  ".repeat(r.depth), r.name)
            })
            .collect()
    }

    #[test]
    fn folders_open_and_close_in_place() {
        let dir = project();
        let mut tree = Explorer::new(dir.path());
        assert_eq!(shown(&tree), [">> docs", " > src", " Cargo.toml"]);
        tree.move_by(1);
        tree.expand();
        assert_eq!(
            shown(&tree),
            [" > docs", ">v src", "   > ui", "   main.rs", " Cargo.toml"]
        );
        tree.expand();
        assert_eq!(tree.selected_row().unwrap().name, "ui", "into it");
        tree.collapse();
        assert_eq!(
            tree.selected_row().unwrap().name,
            "src",
            "to the folder it's in"
        );
        tree.collapse();
        assert_eq!(shown(&tree), [" > docs", ">> src", " Cargo.toml"]);
        tree.move_by(5);
        assert_eq!(
            tree.choose(),
            TreeAction::Open(tree.root().join("Cargo.toml"))
        );
    }

    #[test]
    fn revealing_a_file_opens_the_folders_down_to_it() {
        let dir = project();
        let mut tree = Explorer::new(dir.path());
        assert!(tree.reveal(&dir.path().join("src/ui/view.rs")));
        assert_eq!(
            shown(&tree),
            [
                "  > docs",
                "  v src",
                "    v ui",
                ">     view.rs",
                "    main.rs",
                "  Cargo.toml",
            ]
            .map(|s| s.replacen("  ", " ", 1))
        );
        assert!(
            !tree.reveal(Path::new("/elsewhere/x.rs")),
            "not in the tree"
        );
    }

    #[test]
    fn refreshing_shows_new_files_and_keeps_the_selection() {
        let dir = project();
        let mut tree = Explorer::new(dir.path());
        tree.move_by(2);
        fs::write(dir.path().join("build.rs"), "").unwrap();
        assert_eq!(tree.rows().len(), 3, "not until refreshed");
        tree.refresh();
        assert_eq!(
            shown(&tree),
            [" > docs", " > src", " build.rs", ">Cargo.toml"]
        );
    }

    #[test]
    fn saving_a_new_file_lists_its_folder_again() {
        let dir = project();
        let mut tree = Explorer::new(dir.path());
        let new = dir.path().join("notes.md");
        fs::write(&new, "").unwrap();
        tree.saw_file(&new);
        assert_eq!(
            shown(&tree),
            [">> docs", " > src", " Cargo.toml", " notes.md"]
        );
        // A closed folder's files aren't listed until it's opened anyway.
        fs::write(dir.path().join("docs/a.md"), "").unwrap();
        tree.saw_file(&dir.path().join("docs/a.md"));
        assert_eq!(tree.rows().len(), 4);
    }
}
