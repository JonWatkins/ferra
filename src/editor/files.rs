//! Opening and saving at the editor level: replacing the document and
//! reporting the result on the message line. Also noticing when another
//! program changes an open file: an unmodified buffer reloads itself, a
//! modified one gets a warning (and `:w` refuses until `:w!` or `:e!`).

use std::path::Path;

use super::{DiskState, Document, Editor, Message, Mode, OpenInfo, SaveInfo};
use crate::core::LineEnding;
use crate::events::Event;
use crate::files::FileError;

/// Vim's "E37" message, used wherever unsaved changes would be lost.
pub const UNSAVED_CHANGES: &str = "No write since last change (add ! to override)";

impl Editor {
    /// Replaces the current document with the file at `path`.
    /// Opens the file at `path` in a new buffer, or switches to it if it's
    /// already open.
    pub fn open(&mut self, path: &Path) -> Result<(), FileError> {
        if let Some(id) = self.find_open_path(path) {
            if id != self.document.id() {
                self.push_jump();
            }
            self.switch_to(id);
            let name = self.document.name().into_owned();
            self.set_message(Message::info(format!("\"{name}\"")));
            return Ok(());
        }
        let (mut document, info) = Document::open(path)?;
        if !self.document.is_pristine_scratch() {
            self.push_jump();
        }
        // A file replacing the untouched starting buffer takes its number,
        // so the first file opened is buffer 1, as in Vim.
        let id = if self.document.is_pristine_scratch() {
            self.document.id()
        } else {
            self.new_document_id()
        };
        document.set_id(id);
        self.activate(document);
        self.set_message(Message::info(describe_open(&info)));
        self.emit(Event::Opened { path: info.path });
        Ok(())
    }

    /// `:e!`: re-reads the active buffer's file, discarding unsaved changes.
    /// The reload is one undo step, so `u` brings the discarded text back.
    pub fn reload(&mut self) -> Result<(), FileError> {
        let path = self
            .document
            .path()
            .ok_or(FileError::NoFileName)?
            .to_path_buf();
        self.set_mode(Mode::Normal);
        let info = match self.document.reload_in_place()? {
            Some(info) => info,
            None => {
                // The file is gone: start again from an empty new file.
                let (mut document, info) = Document::open(&path)?;
                document.set_id(self.document.id());
                document.follow_version_of(&self.document);
                self.document = document;
                info
            }
        };
        self.enforce_cursor_rules();
        self.set_message(Message::info(describe_open(&info)));
        self.emit(Event::Opened { path: info.path });
        Ok(())
    }

    /// Checks every buffer's file for changes made by other programs (when
    /// the terminal regains focus, and `:checktime`).
    pub fn check_disk_changes(&mut self) {
        let mut reports: Vec<DiskReport> = check_document(&mut self.document).into_iter().collect();
        if matches!(reports.first(), Some(DiskReport::Reloaded(_))) {
            self.after_active_reload();
        }
        for document in &mut self.background {
            reports.extend(check_document(document));
        }
        self.show_disk_reports(&reports);
    }

    /// Checks the active buffer's file (after switching to it).
    pub(super) fn check_active_disk(&mut self) {
        if let Some(report) = check_document(&mut self.document) {
            if matches!(report, DiskReport::Reloaded(_)) {
                self.after_active_reload();
            }
            self.show_disk_reports(&[report]);
        }
    }

    fn after_active_reload(&mut self) {
        self.enforce_cursor_rules();
        self.emit(Event::BufferChanged);
    }

    /// Problems first; otherwise what was reloaded.
    fn show_disk_reports(&mut self, reports: &[DiskReport]) {
        let problem = reports.iter().find_map(|report| match report {
            DiskReport::Reloaded(_) => None,
            DiskReport::Conflict(name) => Some(format!(
                "\"{name}\" changed on disk: :e! loads it, :w! keeps your version"
            )),
            DiskReport::Deleted(name) => Some(format!(
                "\"{name}\" was deleted on disk (:w writes it again)"
            )),
            DiskReport::Failed(error) => Some(error.clone()),
        });
        if let Some(problem) = problem {
            self.set_message(Message::error(problem));
            return;
        }
        let reloaded: Vec<&str> = reports
            .iter()
            .filter_map(|r| match r {
                DiskReport::Reloaded(name) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        match reloaded.as_slice() {
            [] => {}
            [name] => self.set_message(Message::info(format!(
                "\"{name}\" reloaded: changed on disk"
            ))),
            names => self.set_message(Message::info(format!(
                "{} buffers reloaded: changed on disk",
                names.len()
            ))),
        }
    }

    /// Saves the document (see [`Document::save`]) and reports what was written.
    pub fn save(&mut self, target: Option<&Path>, force: bool) -> Result<(), FileError> {
        let info = self.document.save(target, force)?;
        self.set_message(Message::info(describe_save(&info)));
        self.emit(Event::AfterSave { path: info.path });
        Ok(())
    }
}

/// What checking one buffer's file found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DiskReport {
    Reloaded(String),
    /// Changed on disk, but the buffer has unsaved changes too.
    Conflict(String),
    Deleted(String),
    Failed(String),
}

/// Reloads `document` if its file changed and it has no unsaved changes;
/// otherwise notes the problem once.
fn check_document(document: &mut Document) -> Option<DiskReport> {
    let name = document.name().into_owned();
    let report = match document.disk_state() {
        DiskState::Unchanged => return None,
        DiskState::Changed if !document.is_modified() => match document.reload_in_place() {
            Ok(Some(_)) => return Some(DiskReport::Reloaded(name)),
            Ok(None) => DiskReport::Deleted(name),
            Err(error) => DiskReport::Failed(error.to_string()),
        },
        DiskState::Changed => DiskReport::Conflict(name),
        DiskState::Deleted => DiskReport::Deleted(name),
    };
    document.acknowledge_disk_state();
    Some(report)
}

/// `"src/main.rs" 12L, 340B`, plus `[New]`, `[CRLF]` or `[noeol]` when relevant.
fn describe_open(info: &OpenInfo) -> String {
    let path = info.path.display();
    if info.new_file {
        return format!("\"{path}\" [New]");
    }
    let mut text = format!("\"{path}\" {}L, {}B", info.lines, info.bytes);
    if info.format.line_ending == LineEnding::Crlf {
        text.push_str(" [CRLF]");
    }
    if !info.format.final_newline && info.bytes > 0 {
        text.push_str(" [noeol]");
    }
    text
}

/// `"src/main.rs" 12L, 340B written`.
fn describe_save(info: &SaveInfo) -> String {
    format!(
        "\"{}\" {}L, {}B written",
        info.path.display(),
        info.lines,
        info.bytes
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::TextFormat;

    fn open_info(new_file: bool, format: TextFormat, bytes: usize) -> OpenInfo {
        OpenInfo {
            path: PathBuf::from("a.txt"),
            new_file,
            lines: 2,
            bytes,
            format,
        }
    }

    #[test]
    fn describes_opened_files() {
        let lf = TextFormat::default();
        assert_eq!(describe_open(&open_info(false, lf, 8)), "\"a.txt\" 2L, 8B");
        assert_eq!(describe_open(&open_info(true, lf, 0)), "\"a.txt\" [New]");

        let crlf_noeol = TextFormat {
            line_ending: LineEnding::Crlf,
            final_newline: false,
        };
        assert_eq!(
            describe_open(&open_info(false, crlf_noeol, 8)),
            "\"a.txt\" 2L, 8B [CRLF] [noeol]"
        );
    }

    #[test]
    fn empty_file_is_not_noeol() {
        let format = TextFormat {
            final_newline: false,
            ..TextFormat::default()
        };
        assert_eq!(
            describe_open(&open_info(false, format, 0)),
            "\"a.txt\" 2L, 0B"
        );
    }

    #[test]
    fn describes_saves() {
        let info = SaveInfo {
            path: PathBuf::from("a.txt"),
            lines: 3,
            bytes: 12,
        };
        assert_eq!(describe_save(&info), "\"a.txt\" 3L, 12B written");
    }

    #[test]
    fn open_and_save_through_the_editor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hi\n").unwrap();

        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.open(&path).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);
        assert_eq!(editor.document.buffer().text(), "hi");
        assert!(editor.message().unwrap().text.ends_with("1L, 3B"));

        editor.save(None, false).unwrap();
        assert!(editor.message().unwrap().text.ends_with("1L, 3B written"));

        let events = editor.take_events();
        assert!(events.contains(&Event::Opened { path: path.clone() }));
        assert!(events.contains(&Event::AfterSave { path }));
    }
}
