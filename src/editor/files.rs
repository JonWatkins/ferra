//! Opening and saving at the editor level: replacing the document and
//! reporting the result on the message line. Also noticing when another
//! program changes an open file: an unmodified buffer reloads itself, a
//! modified one gets a warning (and `:w` refuses until `:w!` or `:e!`).
//!
//! Leaving insert mode sets the buffer's `^` mark (an ordinary local mark, so
//! it follows edits), which `gi` and `` `^ `` go back to. `gf`
//! (`Editor::open_file_under_cursor`) takes the run of file-name characters
//! under the cursor, splits off a `:line[:col]` suffix, and opens the first of
//! "beside the current file" and "from the working directory" that exists,
//! through `open`, so it's a jump and `<C-^>` comes back. `gx`
//! (`Editor::url_under_cursor`) finds the URL around the cursor (`url_at`),
//! or failing that the file `gf` would, for the system's opener.

use std::path::{Path, PathBuf};

use super::{DiskState, Document, Editor, Message, Mode, OpenInfo, SaveInfo};
use crate::core::{LineEnding, Position};
use crate::events::Event;
use crate::files::FileError;

/// Vim's "E37" message, used wherever unsaved changes would be lost.
pub const UNSAVED_CHANGES: &str = "No write since last change (add ! to override)";

impl Editor {
    /// Replaces the current document with the file at `path`.
    /// Opens the file at `path` in a new buffer, or switches to it if it's
    /// already open.
    /// `gf`: opens the file named under the cursor. A `:line` (and
    /// `:col`) after the name, as in compiler messages and grep output,
    /// goes there. The name is looked for next to the current file, then
    /// in the working directory; `~` is the home directory.
    pub fn open_file_under_cursor(&mut self) -> Result<(), String> {
        let word = self
            .file_name_under_cursor()
            .ok_or("No file name under the cursor")?;
        let (name, row, col) = split_location(word.trim_end_matches([':', '.']));
        let path = self
            .find_named_file(name)
            .map_err(|name| format!("Can't find file \"{}\"", name.display()))?;
        self.open(&path).map_err(|e| e.to_string())?;
        if let Some(row) = row {
            let row = row
                .saturating_sub(1)
                .min(self.document.buffer().line_count() - 1);
            let col = match col {
                Some(col) => col.saturating_sub(1),
                None => crate::core::motion::first_non_blank(self.document.buffer(), row),
            };
            self.document.set_cursor(Position::new(row, col));
            self.enforce_cursor_rules();
        }
        Ok(())
    }

    /// `gx`: what to hand the system's opener: the URL under the cursor
    /// (`www.` ones get `https://`), or else the file named there, found as
    /// `gf` finds it (a `:line` suffix is dropped).
    pub fn url_under_cursor(&self) -> Result<String, String> {
        if let Some(url) = self.link_under_cursor() {
            return Ok(url);
        }
        let word = self
            .file_name_under_cursor()
            .ok_or("No URL under the cursor")?;
        let (name, _, _) = split_location(word.trim_end_matches([':', '.']));
        let path = self
            .find_named_file(name)
            .map_err(|name| format!("Can't find file \"{}\"", name.display()))?;
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        Ok(path.to_string_lossy().into_owned())
    }

    /// The URL under the cursor, if it's on one (no file names: a click
    /// that opens links goes to definitions everywhere else).
    pub fn link_under_cursor(&self) -> Option<String> {
        let cursor = self.document.cursor();
        url_at(&self.document.buffer().line(cursor.row), cursor.col)
    }

    /// The run of file-name characters under (or, past the end, before) the
    /// cursor.
    fn file_name_under_cursor(&self) -> Option<String> {
        let cursor = self.document.cursor();
        let line: Vec<char> = self.document.buffer().line(cursor.row).chars().collect();
        let in_name = |c: char| c.is_alphanumeric() || "/._-~+:@%#\\".contains(c);
        let at = cursor.col.min(line.len().saturating_sub(1));
        if line.is_empty() || !in_name(line[at]) {
            return None;
        }
        let start = line[..at]
            .iter()
            .rposition(|&c| !in_name(c))
            .map_or(0, |i| i + 1);
        let end = line[at..]
            .iter()
            .position(|&c| !in_name(c))
            .map_or(line.len(), |i| at + i);
        Some(line[start..end].iter().collect())
    }

    /// `name` beside the current file, else from the working directory;
    /// `~/` is the home directory. The expanded name if neither exists.
    fn find_named_file(&self, name: &str) -> Result<PathBuf, PathBuf> {
        let name = match name.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME")
                .map(|home| Path::new(&home).join(rest))
                .unwrap_or_else(|| Path::new(name).to_path_buf()),
            None => Path::new(name).to_path_buf(),
        };
        let beside = self
            .document
            .path()
            .and_then(Path::parent)
            .map(|dir| dir.join(&name));
        [beside, Some(name.clone())]
            .into_iter()
            .flatten()
            .find(|p| p.is_file())
            .ok_or(name)
    }

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
        let fallback = self.options.fallback_encoding.0;
        let (mut document, info) = Document::open_with(path, fallback)?;
        // What a server reported for it while it wasn't open, until it
        // reports again.
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(diagnostics) = self.file_diagnostics.remove(&canonical) {
            document.set_diagnostics(diagnostics);
        }
        if !self.document.is_pristine_scratch() {
            self.push_jump();
        }
        // A file replacing the untouched starting buffer takes its number,
        // so the first file opened is buffer 1, as in Vim. Not if another
        // window shows it or has a tab for it: it stays open, keeping its
        // number (and that tab isn't this file's).
        let replaces_scratch =
            self.document.is_pristine_scratch() && !self.in_other_window(self.document.id());
        let id = if replaces_scratch {
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
        let info = match self
            .document
            .reload_in_place(self.options.fallback_encoding.0)?
        {
            Some(info) => info,
            None => {
                // The file is gone: start again from an empty new file.
                let fallback = self.options.fallback_encoding.0;
                let (mut document, info) = Document::open_with(&path, fallback)?;
                document.set_id(self.document.id());
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
        let mut reports: Vec<DiskReport> =
            check_document(&mut self.document, self.options.fallback_encoding.0)
                .into_iter()
                .collect();
        if matches!(reports.first(), Some(DiskReport::Reloaded(_))) {
            self.after_active_reload();
        }
        let fallback = self.options.fallback_encoding.0;
        for document in &mut self.background {
            reports.extend(check_document(document, fallback));
        }
        self.show_disk_reports(&reports);
    }

    /// Checks the active buffer's file (after switching to it).
    pub(super) fn check_active_disk(&mut self) {
        if let Some(report) = check_document(&mut self.document, self.options.fallback_encoding.0) {
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

    /// Something to add to the next save's message (why it wasn't
    /// formatted).
    pub fn note_for_save(&mut self, note: String) {
        self.save_note = Some(note);
    }

    /// Saves the document (see [`Document::save`]) and reports what was written.
    pub fn save(&mut self, target: Option<&Path>, force: bool) -> Result<(), FileError> {
        let note = self.save_note.take();
        let info = self.document.save(target, force)?;
        let mut text = describe_save(&info);
        if let Some(note) = note {
            text.push_str(&format!(" ({note})"));
        }
        self.set_message(Message::info(text));
        self.sidebar_saw_save(&info.path);
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
fn check_document(
    document: &mut Document,
    fallback: &'static crate::core::Encoding,
) -> Option<DiskReport> {
    let name = document.name().into_owned();
    let report = match document.disk_state() {
        DiskState::Unchanged => return None,
        DiskState::Changed if !document.is_modified() => match document.reload_in_place(fallback) {
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
    if !info.format.encoding.is_plain_utf8() {
        text.push_str(&format!(" [{}]", info.format.encoding.name()));
    }
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
            ..TextFormat::default()
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

    #[test]
    fn gx_target_is_a_url_or_a_file_gf_would_find() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "[x](https://a.b/c) b.txt:3 nowhere.txt\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        let mut editor = Editor::default();
        editor.open(&path).unwrap();
        let at = |editor: &mut Editor, col| {
            editor.document.set_cursor(Position::new(0, col));
            editor.url_under_cursor()
        };
        assert_eq!(at(&mut editor, 6), Ok("https://a.b/c".into()));
        let b = std::fs::canonicalize(dir.path().join("b.txt")).unwrap();
        assert_eq!(
            at(&mut editor, 19),
            Ok(b.to_string_lossy().into_owned()),
            "beside the file, without its :line"
        );
        assert_eq!(
            at(&mut editor, 28),
            Err("Can't find file \"nowhere.txt\"".into())
        );
        assert_eq!(at(&mut editor, 2), Err("No URL under the cursor".into()));

        // Ctrl+click opens links only: a file name goes to its definition.
        editor.document.set_cursor(Position::new(0, 6));
        assert_eq!(editor.link_under_cursor(), Some("https://a.b/c".into()));
        editor.document.set_cursor(Position::new(0, 19));
        assert_eq!(editor.link_under_cursor(), None);
    }
}

/// `src/main.rs:12:5` → the name, line 12 and column 5 (1-based); plain
/// names have neither.
fn split_location(word: &str) -> (&str, Option<usize>, Option<usize>) {
    let mut parts = word.rsplitn(3, ':');
    let last = parts.next().unwrap_or(word);
    let middle = parts.next();
    let first = parts.next();
    let number = |s: &str| s.parse::<usize>().ok();
    match (first, middle.and_then(number), number(last)) {
        (Some(name), Some(row), Some(col)) => (name, Some(row), Some(col)),
        _ => match (middle, number(last)) {
            (Some(rest), Some(row)) => {
                // `name:line`, with `rest` possibly `dir:name` (rare).
                let name = match first {
                    Some(first) => &word[..first.len() + 1 + rest.len()],
                    None => rest,
                };
                (name, Some(row), None)
            }
            _ => (word, None, None),
        },
    }
}

#[cfg(test)]
mod location_tests {
    use super::split_location;

    #[test]
    fn names_with_lines_and_columns() {
        assert_eq!(split_location("src/main.rs"), ("src/main.rs", None, None));
        assert_eq!(
            split_location("src/main.rs:12"),
            ("src/main.rs", Some(12), None)
        );
        assert_eq!(
            split_location("src/main.rs:12:5"),
            ("src/main.rs", Some(12), Some(5))
        );
        assert_eq!(split_location("a:b.rs:3"), ("a:b.rs", Some(3), None));
    }
}

/// The URL on `line` that covers column `col`: one starting `scheme://`,
/// `mailto:` or `www.` (given `https://`), running to whitespace or a quote
/// or angle bracket, less trailing punctuation and a closing bracket it
/// didn't open (so `(see https://a.b/c).` and Markdown links work).
fn url_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let starts_with = |i: usize, s: &str| {
        let s: Vec<char> = s.chars().collect();
        chars.len() >= i + s.len()
            && chars[i..i + s.len()]
                .iter()
                .zip(&s)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    };
    // A scheme is a letter then letters, digits, `+`, `-` or `.`.
    let scheme_end = |i: usize| {
        if !chars[i].is_ascii_alphabetic() {
            return None;
        }
        let len = chars[i..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || "+-.".contains(**c))
            .count();
        (len >= 2 && starts_with(i + len, "://")).then_some(i + len + 3)
    };
    let ends = |c: char| c.is_whitespace() || "<>\"'`".contains(c);
    let mut i = 0;
    while i < chars.len() {
        // Not partway into a word or host name (`a.www.b`, `xhttps://`).
        let boundary =
            i == 0 || !(chars[i - 1].is_alphanumeric() || ".-+@/".contains(chars[i - 1]));
        let body = if !boundary {
            None
        } else if let Some(end) = scheme_end(i) {
            Some(end)
        } else if starts_with(i, "mailto:") {
            Some(i + 7)
        } else if starts_with(i, "www.") {
            Some(i + 4)
        } else {
            None
        };
        let Some(body) = body else {
            i += 1;
            continue;
        };
        let mut end = chars[body..]
            .iter()
            .position(|&c| ends(c))
            .map_or(chars.len(), |n| body + n);
        while end > body {
            let last = chars[end - 1];
            let unopened = |open: char| {
                let count = |c: char| chars[i..end].iter().filter(|&&x| x == c).count();
                count(last) > count(open)
            };
            let trim = match last {
                '.' | ',' | ';' | ':' | '!' | '?' => true,
                ')' => unopened('('),
                ']' => unopened('['),
                '}' => unopened('{'),
                _ => false,
            };
            if !trim {
                break;
            }
            end -= 1;
        }
        if end > body && (i..end).contains(&col) {
            let url: String = chars[i..end].iter().collect();
            return Some(if starts_with(i, "www.") {
                format!("https://{url}")
            } else {
                url
            });
        }
        i = end.max(i + 1);
    }
    None
}

#[cfg(test)]
mod url_tests {
    use super::url_at;

    fn at(line: &str, needle: &str) -> Option<String> {
        url_at(line, line.find(needle).unwrap())
    }

    #[test]
    fn finds_the_url_around_the_cursor() {
        let line = "see https://example.com/a?b=1#c for more";
        let url = Some("https://example.com/a?b=1#c".to_string());
        assert_eq!(at(line, "https"), url);
        assert_eq!(at(line, "?b"), url);
        assert_eq!(at(line, "#c"), url);
        assert_eq!(at(line, "see"), None);
        assert_eq!(at(line, "for"), None);
    }

    #[test]
    fn leaves_off_trailing_punctuation_and_unopened_brackets() {
        let url = Some("https://a.b/c".to_string());
        assert_eq!(at("(see https://a.b/c).", "https"), url);
        assert_eq!(at("[link](https://a.b/c)", "https"), url);
        assert_eq!(at("<https://a.b/c>", "https"), url);
        assert_eq!(at("\"https://a.b/c\",", "https"), url);
        assert_eq!(
            at("https://en.wikipedia.org/wiki/Rust_(language).", "https"),
            Some("https://en.wikipedia.org/wiki/Rust_(language)".to_string())
        );
    }

    #[test]
    fn other_schemes_mailto_and_www() {
        assert_eq!(
            at("file:///tmp/x.txt", "tmp"),
            Some("file:///tmp/x.txt".to_string())
        );
        assert_eq!(
            at("mail mailto:a@b.c now", "a@"),
            Some("mailto:a@b.c".to_string())
        );
        assert_eq!(
            at("go to www.example.com.", "example"),
            Some("https://www.example.com".to_string())
        );
        assert_eq!(at("a.www.example.com", "example"), None);
        assert_eq!(at("x://", "x"), None, "a one-letter scheme, like C:\\");
        assert_eq!(at("https://", "https"), None, "nothing after the scheme");
    }

    #[test]
    fn picks_the_url_the_cursor_is_on() {
        let line = "https://one.example https://two.example";
        assert_eq!(at(line, "two"), Some("https://two.example".to_string()));
        assert_eq!(at(line, "one"), Some("https://one.example".to_string()));
    }
}
