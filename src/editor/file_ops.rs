//! Making, naming and moving files from the editor: a new buffer
//! (`Ctrl+N`), saving one that has no name yet (asking for it), and the
//! file tree's new file and folder, rename, and moving to the bin. Names
//! are asked for in an input box; moving to the bin is confirmed first.

use std::path::{Path, PathBuf};

use super::input_box::InputBox;
use super::popup::{Popup, PopupSpan};
use super::{Document, Editor, Message, Mode};

impl Editor {
    /// `Ctrl+N`, File → New File: a new, empty buffer with no name yet
    /// (saving asks for one).
    pub fn new_buffer(&mut self) {
        let mut document = Document::default();
        document.set_id(self.new_document_id());
        self.settings = None;
        self.keybindings = None;
        self.activate(document);
        self.set_message(Message::info("New file: Ctrl+S (or :w name) saves it"));
    }

    /// `Ctrl+S` on a buffer with no name: asks for one (starting in the
    /// file tree's folder, or the working directory), then saves to it.
    pub fn ask_save_as(&mut self) {
        let folder = self.tree_folder().map(|dir| self.relative(&dir));
        let start = match folder {
            Some(dir) if !dir.is_empty() => format!("{dir}/"),
            _ => String::new(),
        };
        let mut input = InputBox::new("Save as", &start, "write");
        input.placeholder = "path/to/file (Tab completes)".into();
        input.completes_paths = true;
        input.field.end();
        self.open_input_box(input);
    }

    /// Whether the sidebar shows the file tree (not Search, Source Control
    /// or a plugin's view).
    pub fn file_tree_shown(&self) -> bool {
        self.sidebar().is_some()
            && !self.search_view_shown()
            && !self.git_view_shown()
            && self.plugin_view().is_none()
    }

    /// The file tree's actions work when it's the sidebar's view.
    fn in_file_tree(&self) -> Result<(), String> {
        if !self.file_tree_shown() {
            return Err("That's for the file tree (the sidebar's Explorer)".into());
        }
        Ok(())
    }

    /// The folder new files go in: the tree's selected folder, or the one
    /// its selected file is in.
    fn tree_folder(&self) -> Option<PathBuf> {
        let explorer = self.explorer.as_ref()?;
        let row = explorer.selected_row()?;
        Some(if row.is_dir {
            row.path.clone()
        } else {
            row.path.parent()?.to_path_buf()
        })
    }

    /// `path` from the working directory, for showing.
    fn relative(&self, path: &Path) -> String {
        let here = std::env::current_dir().unwrap_or_default();
        let here = std::fs::canonicalize(&here).unwrap_or(here);
        path.strip_prefix(&here)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    /// `a` / `A` in the file tree: asks for the new file's (or folder's)
    /// name, in the selected folder.
    pub fn tree_ask_new(&mut self, folder: bool) -> Result<(), String> {
        self.in_file_tree()?;
        let dir = self.tree_folder().ok_or("No folder selected")?;
        self.ask_new_in(dir, folder);
        Ok(())
    }

    /// New File / New Folder from a right-click on the tree's empty space:
    /// in the project's folder.
    pub fn tree_ask_new_at_root(&mut self, folder: bool) -> Result<(), String> {
        self.in_file_tree()?;
        let root = self.explorer_mut().root().to_path_buf();
        self.ask_new_in(root, folder);
        Ok(())
    }

    fn ask_new_in(&mut self, dir: PathBuf, folder: bool) {
        let shown = self.relative(&dir);
        let title = match (folder, shown.is_empty()) {
            (false, true) => "New file".to_string(),
            (false, false) => format!("New file in {shown}/"),
            (true, true) => "New folder".to_string(),
            (true, false) => format!("New folder in {shown}/"),
        };
        let command = if folder {
            "tree_create_folder"
        } else {
            "tree_create_file"
        };
        self.tree_target = Some(dir);
        let mut input = InputBox::new(&title, "", command);
        input.placeholder = if folder {
            "name"
        } else {
            "name.ext (a/b.ext makes folders)"
        }
        .into();
        self.open_input_box(input);
    }

    /// The new file or folder `name` (asked for by `tree_ask_new`): made,
    /// shown in the tree, and a file opened.
    pub fn tree_create(&mut self, name: &str, folder: bool) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("No name given".into());
        }
        let dir = self
            .tree_target
            .take()
            .or_else(|| self.tree_folder())
            .ok_or("No folder")?;
        let path = dir.join(name);
        if path.exists() {
            return Err(format!("{} already exists", self.relative(&path)));
        }
        let error = |e: std::io::Error| format!("{}: {e}", path.display());
        if folder {
            std::fs::create_dir_all(&path).map_err(error)?;
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(error)?;
            }
            std::fs::File::create_new(&path).map_err(error)?;
        }
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.refresh();
            explorer.reveal(&path);
        }
        if folder {
            self.set_mode(Mode::Sidebar);
            self.set_message(Message::info(format!("Made {}/", self.relative(&path))));
            Ok(())
        } else {
            if self.mode() == Mode::Sidebar {
                self.set_mode(Mode::Normal);
            }
            self.open(&path).map_err(|e| e.to_string())
        }
    }

    /// `r` (`F2`) in the file tree: asks for the selected file's or
    /// folder's new name.
    pub fn tree_ask_rename(&mut self) -> Result<(), String> {
        self.in_file_tree()?;
        let row = self
            .explorer
            .as_ref()
            .and_then(|e| e.selected_row())
            .cloned()
            .ok_or("Nothing selected")?;
        if self.explorer.as_ref().is_some_and(|e| e.root() == row.path) {
            return Err("The project's folder can't be renamed here".into());
        }
        self.tree_target = Some(row.path.clone());
        let mut input = InputBox::new(&format!("Rename {}", row.name), &row.name, "tree_rename_to");
        input.field.select_all();
        self.open_input_box(input);
        Ok(())
    }

    /// The new name (asked for by `tree_ask_rename`): renamed, beside it
    /// (or elsewhere, with a `/`), open buffers following.
    pub fn tree_rename(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        let from = self.tree_target.take().ok_or("Nothing to rename")?;
        if name.is_empty() {
            return Err("No name given".into());
        }
        let parent = from.parent().ok_or("Nothing to rename")?;
        let to = parent.join(name);
        if to == from {
            return Ok(());
        }
        self.move_path(&from, &to)?;
        self.set_message(Message::info(format!(
            "Renamed {} to {}",
            self.relative(&from),
            self.relative(&to)
        )));
        Ok(())
    }

    /// Moves `from` (a file or folder) to `to`, open buffers in it
    /// following, and shows it in the tree.
    fn move_path(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        let (from, to) = (from.to_path_buf(), to.to_path_buf());
        if to.exists() {
            return Err(format!("{} already exists", self.relative(&to)));
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        // Open buffers in it follow it.
        let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        for doc in std::iter::once(&mut self.document).chain(self.background.iter_mut()) {
            let Some(path) = doc.path().map(canonical) else {
                continue;
            };
            if let Ok(rest) = path.strip_prefix(&from) {
                let moved = if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                };
                doc.moved_to(moved);
            }
        }
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.refresh();
            explorer.reveal(&to);
        }
        self.refresh_options();
        Ok(())
    }

    /// A file or folder dragged onto `folder` in the tree: asks before
    /// moving it there (nothing to ask if it's there already).
    pub fn tree_ask_move(&mut self, from: PathBuf, folder: PathBuf) -> Result<(), String> {
        let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let (from, folder) = (canonical(&from), canonical(&folder));
        if from.parent() == Some(folder.as_path()) || from == folder {
            return Ok(());
        }
        if folder.starts_with(&from) {
            return Err("A folder can't be moved into itself".into());
        }
        if self
            .explorer
            .as_ref()
            .is_some_and(|e| canonical(e.root()) == from)
        {
            return Err("The project's folder can't be moved here".into());
        }
        let name = from
            .file_name()
            .ok_or("Nothing to move")?
            .to_string_lossy()
            .into_owned();
        let to = folder.join(&name);
        if to.exists() {
            return Err(format!("{} already exists", self.relative(&to)));
        }
        let span = |text: &str, style: Option<&str>| PopupSpan {
            text: text.to_string(),
            style: style.map(str::to_string),
        };
        let into = self.relative(&folder);
        let into = if into.is_empty() {
            "the project's folder".to_string()
        } else {
            into
        };
        self.tree_target = Some(from);
        self.tree_move_to = Some(to);
        self.open_popup(Popup {
            name: "tree-move".into(),
            title: "Move".into(),
            lines: vec![
                vec![span(&format!("Move {name} into {into}?"), None)],
                vec![],
                vec![
                    span("y", Some("ui.popup.title")),
                    span(" move it   ", None),
                    span("n", Some("ui.popup.title")),
                    span(" cancel", None),
                ],
            ],
            keys: vec![
                ("y".into(), "tree_move_confirmed".into()),
                ("<CR>".into(), "tree_move_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..Popup::default()
        });
        Ok(())
    }

    /// `y` there: moves it.
    pub fn tree_move(&mut self) -> Result<(), String> {
        self.close_popup();
        let from = self.tree_target.take().ok_or("Nothing to move")?;
        let to = self.tree_move_to.take().ok_or("Nothing to move")?;
        self.move_path(&from, &to)?;
        self.set_message(Message::info(format!(
            "Moved {} to {}",
            self.relative(&from),
            self.relative(&to)
        )));
        Ok(())
    }

    /// `d` (`Delete`) in the file tree: asks before moving the selected
    /// file or folder to the bin.
    pub fn tree_ask_trash(&mut self) -> Result<(), String> {
        self.in_file_tree()?;
        let row = self
            .explorer
            .as_ref()
            .and_then(|e| e.selected_row())
            .cloned()
            .ok_or("Nothing selected")?;
        if self.explorer.as_ref().is_some_and(|e| e.root() == row.path) {
            return Err("The project's folder can't be moved to the bin here".into());
        }
        self.tree_target = Some(row.path.clone());
        let span = |text: &str, style: Option<&str>| PopupSpan {
            text: text.to_string(),
            style: style.map(str::to_string),
        };
        let what = if row.is_dir {
            format!("the folder {} and everything in it", row.name)
        } else {
            row.name.clone()
        };
        self.open_popup(Popup {
            name: "trash".into(),
            title: "Move to the bin".into(),
            lines: vec![
                vec![span(&format!("Move {what} to the bin?"), None)],
                vec![],
                vec![span("You can put it back from there.", None)],
                vec![],
                vec![
                    span("y", Some("ui.popup.title")),
                    span(" move it   ", None),
                    span("n", Some("ui.popup.title")),
                    span(" cancel", None),
                ],
            ],
            keys: vec![
                ("y".into(), "tree_trash_confirmed".into()),
                ("<CR>".into(), "tree_trash_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..Popup::default()
        });
        Ok(())
    }

    /// `y` there: moves it to the bin.
    pub fn tree_trash(&mut self) -> Result<(), String> {
        self.close_popup();
        let path = self.tree_target.take().ok_or("Nothing to move")?;
        crate::files::trash(&path)?;
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.refresh();
        }
        self.set_message(Message::info(format!(
            "Moved {} to the bin",
            self.relative(&path)
        )));
        Ok(())
    }

    /// The tree's selected file or folder (`tree`), or the buffer's file.
    fn target_path(&self, tree: bool) -> Option<PathBuf> {
        if tree {
            return self
                .explorer
                .as_ref()
                .and_then(|e| e.selected_row())
                .map(|r| r.path.clone());
        }
        self.document
            .path()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
    }

    /// Copy Path / Copy Relative Path: onto the clipboard.
    pub fn copy_path(&mut self, relative: bool, tree: bool) -> Result<(), String> {
        let path = self.target_path(tree).ok_or("No file")?;
        let text = if relative {
            self.relative(&path)
        } else {
            path.to_string_lossy().into_owned()
        };
        self.registers.store(
            Some('+'),
            super::registers::Register::charwise(text.clone()),
            super::registers::StoreKind::Yank,
        )?;
        self.set_message(Message::info(format!("Copied {text}")));
        Ok(())
    }

    /// Reveal in Finder (the file manager).
    pub fn reveal_in_file_manager(&mut self, tree: bool) -> Result<(), String> {
        let path = self.target_path(tree).ok_or("No file")?;
        crate::files::reveal(&path)
    }
}
