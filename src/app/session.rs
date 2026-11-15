//! Sessions (`restore-session`): what was open when ferra quit in a
//! folder (each file and its cursor, the one shown, whether the sidebar
//! was open), opened again the next time it starts there with no files
//! named. Kept in `$XDG_STATE_HOME/ferra/sessions/` (else
//! `~/.local/state/ferra/sessions/`), a file per folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::App;
use crate::core::Position;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Session {
    /// The folder it's for.
    folder: PathBuf,
    /// The one shown, in `files`.
    active: usize,
    sidebar: bool,
    files: Vec<OpenFile>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct OpenFile {
    path: PathBuf,
    line: usize,
    column: usize,
}

/// Where the session for `folder` is kept, in the sessions folder `dir`.
pub fn session_file(dir: &Path, folder: &Path) -> PathBuf {
    // FNV-1a: stable from one build to the next, unlike std's hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in folder.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let name = folder
        .file_name()
        .map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned());
    dir.join(format!("{name}-{hash:016x}.toml"))
}

/// The default sessions folder.
pub fn default_dir() -> Option<PathBuf> {
    crate::log::default_path().and_then(|log| log.parent().map(|dir| dir.join("sessions")))
}

fn current_folder() -> Option<PathBuf> {
    let folder = std::env::current_dir().ok()?;
    Some(std::fs::canonicalize(&folder).unwrap_or(folder))
}

impl App {
    /// Where this ferra keeps sessions (`main` sets it; tests set their own,
    /// or leave it unset and none is kept).
    pub fn set_session_dir(&mut self, dir: Option<PathBuf>) {
        self.session_dir = dir;
    }

    /// Writes what's open, for the next start in this folder. A buffer
    /// without a file isn't kept; with nothing open, the session goes.
    pub fn save_session(&self) -> Result<(), String> {
        let (Some(dir), Some(folder)) = (&self.session_dir, current_folder()) else {
            return Ok(());
        };
        if !self.editor.options.restore_session {
            return Ok(());
        }
        let file = session_file(dir, &folder);
        let mut session = Session {
            folder,
            sidebar: self.editor.sidebar().is_some(),
            ..Session::default()
        };
        let active = self.editor.document.id();
        for buffer in self.editor.buffers() {
            let Some(doc) = self.editor.document_by_id(buffer.id) else {
                continue;
            };
            let Some(path) = doc.path() else { continue };
            let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if buffer.id == active {
                session.active = session.files.len();
            }
            let cursor = doc.cursor();
            session.files.push(OpenFile {
                path,
                line: cursor.row,
                column: cursor.col,
            });
        }
        if session.files.is_empty() {
            let _ = std::fs::remove_file(&file);
            return Ok(());
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let text = toml::to_string(&session).map_err(|e| e.to_string())?;
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Opens what was open when ferra last quit in this folder. Returns
    /// whether there was anything to open. Files gone since are skipped.
    pub fn restore_session(&mut self) -> bool {
        let (Some(dir), Some(folder)) = (&self.session_dir, current_folder()) else {
            return false;
        };
        if !self.editor.options.restore_session {
            return false;
        }
        let Ok(text) = std::fs::read_to_string(session_file(dir, &folder)) else {
            return false;
        };
        let Ok(session) = toml::from_str::<Session>(&text) else {
            return false;
        };
        if session.folder != folder {
            return false;
        }
        let mut active = None;
        let mut opened = false;
        for (i, file) in session.files.iter().enumerate() {
            if !file.path.is_file() {
                continue;
            }
            self.open(&file.path);
            let buffer = self.editor.document.buffer();
            let line = file.line.min(buffer.line_count().saturating_sub(1));
            let column = file.column.min(buffer.line_len(line));
            self.editor.document.set_cursor(Position::new(line, column));
            self.editor.enforce_cursor_rules();
            opened = true;
            if i == session.active {
                active = Some(self.editor.document.id());
            }
        }
        if let Some(id) = active {
            self.editor.switch_to(id);
        }
        if opened && session.sidebar && self.editor.sidebar().is_none() {
            self.editor.toggle_sidebar();
        }
        if opened {
            self.editor.clear_message();
        }
        opened
    }
}
