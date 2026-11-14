//! What the log needs when ferra catches a panic: the editor's state, to
//! see how it got there, and, if it has to give up, unsaved buffers saved
//! somewhere safe.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use super::{Document, Editor};

impl Editor {
    /// The state a bug report needs: the mode, the windows and their
    /// layout, and the open buffers (names, not their text).
    pub fn debug_state(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "mode: {:?}", self.mode());
        let windows = self.windows();
        let _ = writeln!(
            out,
            "windows: {} (active {:?}), layout {:?}",
            windows.count(),
            windows.active(),
            windows.layout()
        );
        for window in windows.list() {
            let _ = writeln!(
                out,
                "  window {:?}: buffer {:?}, cursor {}:{}",
                window.id, window.document, window.cursor.row, window.cursor.col
            );
        }
        let _ = writeln!(out, "  sizes: {:?}", windows.sizes());
        for doc in self.all_documents() {
            let _ = writeln!(
                out,
                "buffer {:?}: {}{}, {} lines, cursor {}:{}{}",
                doc.id(),
                doc.name(),
                if doc.is_modified() { " [+]" } else { "" },
                doc.buffer().line_count(),
                doc.cursor().row,
                doc.cursor().col,
                if doc.id() == self.document.id() {
                    " (active)"
                } else {
                    ""
                }
            );
        }
        if let Some(popup) = self.popup() {
            let _ = writeln!(out, "popup: {}", popup.name);
        }
        out
    }

    /// Writes every buffer with unsaved changes into `dir`, as
    /// `<number>-<file name>`. Returns the files written.
    pub fn save_recovery(&self, dir: &Path) -> Vec<PathBuf> {
        let modified: Vec<&Document> = self
            .all_documents()
            .filter(|doc| doc.is_modified())
            .collect();
        if modified.is_empty() || std::fs::create_dir_all(dir).is_err() {
            return Vec::new();
        }
        modified
            .into_iter()
            .filter_map(|doc| {
                let name = doc
                    .path()
                    .and_then(Path::file_name)
                    .map_or("scratch".into(), |n| n.to_string_lossy().into_owned());
                let path = dir.join(format!("{}-{name}", doc.id().0));
                std::fs::write(&path, doc.contents()).ok()?;
                Some(path)
            })
            .collect()
    }

    /// `:log`: opens ferra's log, at its end (the latest entries).
    pub fn open_log(&mut self) -> Result<(), String> {
        let path = crate::log::path()
            .or_else(crate::log::default_path)
            .ok_or("No folder for the log (is HOME set?)")?;
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&path, "").map_err(|e| format!("{}: {e}", path.display()))?;
        }
        if self.mode() == super::Mode::Sidebar {
            self.set_mode(super::Mode::Normal);
        }
        self.open(&path).map_err(|e| e.to_string())?;
        let last = self.document.buffer().line_count().saturating_sub(1);
        self.document
            .set_cursor(crate::core::Position::new(last, 0));
        Ok(())
    }

    fn all_documents(&self) -> impl Iterator<Item = &Document> {
        std::iter::once(&self.document).chain(&self.background)
    }
}
