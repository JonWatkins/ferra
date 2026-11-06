//! Opening and saving a document's file.

use std::path::{Path, PathBuf};

use super::Document;
use crate::core::{Buffer, Position, TextFormat};
use crate::files::{self, FileError};

/// What was found when opening a file, for the status message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenInfo {
    pub path: PathBuf,
    /// The file didn't exist; it will be created on save.
    pub new_file: bool,
    pub lines: usize,
    pub bytes: usize,
    pub format: TextFormat,
}

/// How the file on disk compares with what the document last read or wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskState {
    /// Nothing new (or no file to compare with, or already reported).
    Unchanged,
    /// Another program changed (or created) the file.
    Changed,
    /// The file is gone.
    Deleted,
}

/// What was written by a save, for the status message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveInfo {
    pub path: PathBuf,
    pub lines: usize,
    pub bytes: usize,
}

impl Document {
    /// Opens `path`. A path that doesn't exist yet gives an empty document
    /// that will create the file when saved.
    pub fn open(path: &Path) -> Result<(Document, OpenInfo), FileError> {
        let path = path.to_path_buf();
        let Some(contents) = files::read(&path)? else {
            let document = Document {
                path: Some(path.clone()),
                ..Document::default()
            };
            let info = OpenInfo {
                path,
                new_file: true,
                lines: 0,
                bytes: 0,
                format: TextFormat::default(),
            };
            return Ok((document, info));
        };

        let (text, format) = TextFormat::decode(&contents.text);
        let document = Document {
            buffer: Buffer::from_text(&text),
            path: Some(path.clone()),
            format,
            disk_modified: contents.modified,
            ..Document::default()
        };
        let info = OpenInfo {
            path,
            new_file: false,
            lines: line_count(&contents.text, &document),
            bytes: contents.text.len(),
            format,
        };
        Ok((document, info))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether another program changed or deleted the file since it was
    /// read or written. A state already reported with
    /// [`Document::acknowledge_disk_state`] reads as unchanged until the
    /// file changes again.
    pub fn disk_state(&self) -> DiskState {
        let Some(path) = &self.path else {
            return DiskState::Unchanged;
        };
        let now = files::modified_time(path);
        if now == self.disk_modified || self.disk_acknowledged == Some(now) {
            return DiskState::Unchanged;
        }
        match now {
            Some(_) => DiskState::Changed,
            None => DiskState::Deleted,
        }
    }

    /// Stops [`Document::disk_state`] reporting the file's current state, so
    /// a warning about it is shown once.
    pub fn acknowledge_disk_state(&mut self) {
        self.disk_acknowledged = self.path.as_deref().map(files::modified_time);
    }

    /// Brings in the file's current contents as one undoable change, so
    /// `u` gets the old text back. Only the changed part of the text is
    /// replaced, so the cursor and marks outside it stay put. Returns
    /// `None` if the file no longer exists (the document is left alone).
    pub fn reload_in_place(&mut self) -> Result<Option<OpenInfo>, FileError> {
        let path = self.path.clone().ok_or(FileError::NoFileName)?;
        let Some(contents) = files::read(&path)? else {
            return Ok(None);
        };
        let (text, format) = TextFormat::decode(&contents.text);
        let cursor = self.cursor;

        self.history.commit();
        let old: Vec<char> = self.buffer.text().chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        if old != new {
            let start = position_at(&old, prefix);
            let end = position_at(&old, old.len() - suffix);
            let replacement: String = new[prefix..new.len() - suffix].iter().collect();
            self.delete(start, end);
            self.insert(start, &replacement);
        }
        self.history.commit();

        self.saved_revision = self.history.revision();
        self.disk_modified = contents.modified;
        self.disk_acknowledged = None;
        self.format = format;
        self.set_cursor(cursor);
        Ok(Some(OpenInfo {
            new_file: false,
            lines: line_count(&contents.text, self),
            bytes: contents.text.len(),
            format,
            path,
        }))
    }

    pub fn format(&self) -> TextFormat {
        self.format
    }

    /// Saves to `target`, or to the document's own path if `None`.
    ///
    /// Like Vim's `:w`:
    /// - A document without a path adopts `target` as its path.
    /// - Saving to a different path writes a copy; the document keeps its path
    ///   and stays modified.
    /// - Unless `force` is set, refuses to overwrite a file that changed on
    ///   disk since it was read, or an existing file this document never read.
    pub fn save(&mut self, target: Option<&Path>, force: bool) -> Result<SaveInfo, FileError> {
        let (path, is_own_file) = match (target, &self.path) {
            (None, None) => return Err(FileError::NoFileName),
            (None, Some(own)) => (own.clone(), true),
            (Some(target), None) => (target.to_path_buf(), true),
            (Some(target), Some(own)) => (target.to_path_buf(), target == own),
        };

        if !force {
            let known = if is_own_file {
                self.disk_modified
            } else {
                None
            };
            match (files::modified_time(&path), known) {
                (Some(on_disk), Some(known)) if on_disk != known => {
                    return Err(FileError::ChangedOnDisk { path });
                }
                (Some(_), None) => return Err(FileError::AlreadyExists { path }),
                _ => {}
            }
        }

        self.history.commit();
        let contents = self.format.encode(&self.buffer.text());
        let modified = files::write(&path, &contents)?;

        if is_own_file {
            self.path = Some(path.clone());
            self.saved_revision = self.history.revision();
            self.disk_modified = modified;
            self.disk_acknowledged = None;
        }
        Ok(SaveInfo {
            lines: line_count(&contents, self),
            bytes: contents.len(),
            path,
        })
    }
}

/// The position of char `index` in `text`.
fn position_at(text: &[char], index: usize) -> Position {
    let before = &text[..index];
    let row = before.iter().filter(|&&c| c == '\n').count();
    let col = before.iter().rev().take_while(|&&c| c != '\n').count();
    Position::new(row, col)
}

/// Lines as Vim counts them: an empty file has none.
fn line_count(contents: &str, document: &Document) -> usize {
    if contents.is_empty() {
        0
    } else {
        document.buffer.line_count()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::core::LineEnding;

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    /// Changes a file behind the document's back, with a distinct mtime.
    fn touch_externally(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    #[test]
    fn opens_existing_file() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "one\ntwo\n");
        let (doc, info) = Document::open(&path).unwrap();
        assert_eq!(doc.buffer().text(), "one\ntwo");
        assert_eq!(doc.path(), Some(path.as_path()));
        assert!(!doc.is_modified());
        assert_eq!((info.new_file, info.lines, info.bytes), (false, 2, 8));
    }

    #[test]
    fn opens_missing_file_as_new() {
        let fx = Fixture::new();
        let (doc, info) = Document::open(&fx.path("new.txt")).unwrap();
        assert!(info.new_file);
        assert_eq!(doc.buffer().text(), "");
        assert!(!doc.is_modified());
    }

    #[test]
    fn save_round_trips_untouched_file_exactly() {
        let fx = Fixture::new();
        for (name, contents) in [("lf", "a\nb\n"), ("crlf", "a\r\nb\r\n"), ("noeol", "a\nb")] {
            let path = fx.file(name, contents);
            let (mut doc, _) = Document::open(&path).unwrap();
            doc.save(None, false).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), contents, "{name}");
        }
    }

    #[test]
    fn edits_are_saved_in_the_files_format() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "a\r\nb\r\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        doc.insert_str("x\n");
        let info = doc.save(None, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "x\r\na\r\nb\r\n");
        assert_eq!((info.lines, info.bytes), (3, 9));
        assert_eq!(doc.format().line_ending, LineEnding::Crlf);
    }

    #[test]
    fn saving_clears_modified() {
        let fx = Fixture::new();
        let (mut doc, _) = Document::open(&fx.path("a.txt")).unwrap();
        doc.insert_str("hi");
        assert!(doc.is_modified());
        doc.save(None, false).unwrap();
        assert!(!doc.is_modified());
        doc.undo();
        assert!(doc.is_modified(), "undoing past the save is a change");
    }

    #[test]
    fn new_file_is_created_with_final_newline() {
        let fx = Fixture::new();
        let path = fx.path("new.txt");
        let (mut doc, _) = Document::open(&path).unwrap();
        doc.insert_str("hello");
        doc.save(None, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello\n");
    }

    #[test]
    fn scratch_needs_a_file_name() {
        let mut doc = Document::with_text("x");
        assert!(matches!(doc.save(None, false), Err(FileError::NoFileName)));
    }

    #[test]
    fn scratch_adopts_path_on_save() {
        let fx = Fixture::new();
        let path = fx.path("named.txt");
        let mut doc = Document::with_text("x");
        doc.save(Some(&path), false).unwrap();
        assert_eq!(doc.path(), Some(path.as_path()));
        assert!(!doc.is_modified());
    }

    #[test]
    fn saving_a_copy_keeps_path_and_modified_state() {
        let fx = Fixture::new();
        let original = fx.file("a.txt", "a\n");
        let copy = fx.path("copy.txt");
        let (mut doc, _) = Document::open(&original).unwrap();
        doc.insert_str("b");
        doc.save(Some(&copy), false).unwrap();

        assert_eq!(fs::read_to_string(&copy).unwrap(), "ba\n");
        assert_eq!(fs::read_to_string(&original).unwrap(), "a\n");
        assert_eq!(doc.path(), Some(original.as_path()));
        assert!(doc.is_modified());
    }

    #[test]
    fn refuses_to_overwrite_external_changes_unless_forced() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "mine\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        touch_externally(&path, "theirs\n");

        let err = doc.save(None, false).unwrap_err();
        assert!(matches!(err, FileError::ChangedOnDisk { .. }), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "theirs\n");

        doc.save(None, true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine\n");
    }

    #[test]
    fn refuses_to_clobber_existing_file_it_never_read() {
        let fx = Fixture::new();
        let existing = fx.file("taken.txt", "keep\n");
        let mut doc = Document::with_text("x");
        let err = doc.save(Some(&existing), false).unwrap_err();
        assert!(matches!(err, FileError::AlreadyExists { .. }), "{err}");
        doc.save(Some(&existing), true).unwrap();
        assert_eq!(fs::read_to_string(&existing).unwrap(), "x\n");
    }

    #[test]
    fn saving_twice_does_not_trip_change_detection() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "a\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        doc.insert_str("1");
        doc.save(None, false).unwrap();
        doc.insert_str("2");
        doc.save(None, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "12a\n");
    }

    #[test]
    fn file_deleted_on_disk_is_recreated() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "a\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        doc.save(None, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\n");
    }

    #[test]
    fn disk_state_notices_changes_once_acknowledged() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "one\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        assert_eq!(doc.disk_state(), DiskState::Unchanged);
        touch_externally(&path, "two\n");
        assert_eq!(doc.disk_state(), DiskState::Changed);
        doc.acknowledge_disk_state();
        assert_eq!(doc.disk_state(), DiskState::Unchanged, "reported once");
        fs::remove_file(&path).unwrap();
        assert_eq!(doc.disk_state(), DiskState::Deleted, "a new state is new");
        assert_eq!(Document::with_text("x").disk_state(), DiskState::Unchanged);
    }

    #[test]
    fn reload_in_place_replaces_only_the_changed_middle() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "keep\nold line\nkeep too\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        doc.set_cursor(Position::new(2, 3));
        touch_externally(&path, "keep\nnew\nkeep too\n");
        let info = doc.reload_in_place().unwrap().unwrap();
        assert_eq!(doc.buffer().text(), "keep\nnew\nkeep too");
        assert_eq!(doc.cursor(), Position::new(2, 3));
        assert!(!doc.is_modified());
        assert_eq!(doc.disk_state(), DiskState::Unchanged);
        assert_eq!(info.lines, 3);
        assert!(doc.undo());
        assert_eq!(doc.buffer().text(), "keep\nold line\nkeep too");
    }

    #[test]
    fn reload_in_place_of_a_deleted_file_leaves_the_text() {
        let fx = Fixture::new();
        let path = fx.file("a.txt", "text\n");
        let (mut doc, _) = Document::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(doc.reload_in_place().unwrap().is_none());
        assert_eq!(doc.buffer().text(), "text");
    }

    #[test]
    fn position_at_counts_rows_and_columns() {
        let text: Vec<char> = "ab\ncd".chars().collect();
        assert_eq!(position_at(&text, 0), Position::new(0, 0));
        assert_eq!(position_at(&text, 2), Position::new(0, 2));
        assert_eq!(position_at(&text, 4), Position::new(1, 1));
    }
}
