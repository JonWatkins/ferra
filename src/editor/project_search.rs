//! The Search view: finding in every file of the project, as VS Code's
//! (`<Space>F`, or the activity bar's search icon).
//!
//! It's a view in the sidebar: a query field with the find box's toggles
//! (match case, whole word, regex), and the results under it, grouped by
//! file. The query field is typed in `Mode::Find` with the find box's own
//! keys: while it has focus (`ProjectSearch::typing`), the find box's
//! commands edit the view's fields instead (`find.rs`). `<CR>`, `<Down>`
//! or `<Tab>` go to the results (`Mode::Sidebar`), where `j`/`k` move,
//! `<CR>` opens the match (or folds a file's matches away) and `/` goes
//! back to the query.
//!
//! The replace field (`<Tab>` from the query) previews each match's
//! replacement in the results (the old text struck through, the new after
//! it). `<CR>` there (or the chip after it) replaces every match, after
//! asking; `r` in the results replaces the selected match, or all of the
//! selected file's. A file's replacements are one edit in its buffer (opened
//! in the background if it isn't open, as a rename's are): undoable, and
//! saved only when you save (`:wa`).
//!
//! Each change to the query starts a search on a thread (`search_files`,
//! run by the app), which walks the project as the file picker does
//! (leaving out what `.gitignore` ignores), skips binary and very large
//! files, and sends what it finds file by file. A newer search makes the
//! app drop what an older one sends.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::find::{FindField, FindState};
use super::{Editor, Message, Mode};
use crate::core::Position;
use crate::core::search::SearchPattern;
use crate::files::{WalkOptions, walk_files};

/// The most matches a search gathers.
pub const MAX_MATCHES: usize = 10_000;

/// Files bigger than this aren't searched.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// A line with matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// 0-based.
    pub row: usize,
    pub text: String,
    /// Each match's columns (chars).
    pub ranges: Vec<(usize, usize)>,
}

/// One file's matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatches {
    /// Relative to the folder searched.
    pub path: PathBuf,
    pub lines: Vec<LineMatch>,
}

impl FileMatches {
    pub fn count(&self) -> usize {
        self.lines.iter().map(|l| l.ranges.len()).sum()
    }
}

/// A row of the results list.
#[derive(Debug, Clone, Copy)]
pub enum SearchRow<'a> {
    /// A file: its matches, and whether they're shown.
    File(&'a FileMatches, bool),
    Line(&'a FileMatches, &'a LineMatch),
}

/// A search for the app to run on a thread.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub job: u64,
    pub root: PathBuf,
    pub pattern: SearchPattern,
    pub options: WalkOptions,
    /// The text of open buffers with unsaved changes, by canonical path:
    /// searched instead of what's on disk.
    pub unsaved: HashMap<PathBuf, String>,
}

#[derive(Debug, Default)]
pub struct ProjectSearch {
    /// The query (and, later, the replacement) and the toggles.
    pub fields: FindState,
    /// The query field has focus (in `Mode::Find`).
    pub typing: bool,
    files: Vec<FileMatches>,
    /// Files whose matches are folded away.
    folded: HashSet<PathBuf>,
    /// The selected row.
    selected: usize,
    /// The first row shown.
    pub scroll: Cell<usize>,
    /// The view follows the selection (after keys), rather than staying
    /// where the mouse wheel scrolled it.
    follow: Cell<bool>,
    /// The folder searched (results' paths are relative to it).
    root: PathBuf,
    /// The search the results are from.
    job: u64,
    searching: bool,
    /// It stopped at `MAX_MATCHES`.
    truncated: bool,
    request: Option<SearchRequest>,
}

impl ProjectSearch {
    /// The results as rows: each file, then its lines unless folded.
    pub fn rows(&self) -> Vec<SearchRow<'_>> {
        let mut rows = Vec::new();
        for file in &self.files {
            let open = !self.folded.contains(&file.path);
            rows.push(SearchRow::File(file, open));
            if open {
                rows.extend(file.lines.iter().map(|line| SearchRow::Line(file, line)));
            }
        }
        rows
    }

    /// The rows `first..first + count` (fewer at the end), without making
    /// all of them: what a frame draws.
    pub fn rows_from(&self, first: usize, count: usize) -> Vec<SearchRow<'_>> {
        let mut rows = Vec::with_capacity(count);
        let mut index = 0;
        for file in &self.files {
            if rows.len() == count {
                break;
            }
            let open = !self.folded.contains(&file.path);
            let size = 1 + if open { file.lines.len() } else { 0 };
            if index + size <= first {
                index += size;
                continue;
            }
            if index >= first {
                rows.push(SearchRow::File(file, open));
            }
            index += 1;
            if !open {
                continue;
            }
            for line in &file.lines {
                if rows.len() == count {
                    break;
                }
                if index >= first {
                    rows.push(SearchRow::Line(file, line));
                }
                index += 1;
            }
        }
        rows
    }

    /// How many rows there are (counted per file, not made).
    pub fn row_count(&self) -> usize {
        self.files
            .iter()
            .map(|f| {
                1 + if self.folded.contains(&f.path) {
                    0
                } else {
                    f.lines.len()
                }
            })
            .sum()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// How many matches, in how many files.
    pub fn totals(&self) -> (usize, usize) {
        (
            self.files.iter().map(FileMatches::count).sum(),
            self.files.len(),
        )
    }

    pub fn searching(&self) -> bool {
        self.searching
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The first row to show in `height` rows, keeping the selection in
    /// view.
    pub fn first_shown(&self, height: usize) -> usize {
        let mut top = self.scroll.get();
        if self.follow.get() {
            if self.selected < top {
                top = self.selected;
            } else if height > 0 && self.selected >= top + height {
                top = self.selected + 1 - height;
            }
        }
        let rows = self.row_count();
        top = top.min(rows.saturating_sub(height.max(1)));
        self.scroll.set(top);
        top
    }
}

/// `n` and `word`, made plural when `n` isn't 1 (`3 matches`, `2 files`).
fn plural(n: usize, word: &str) -> String {
    let ending = match (n, word.ends_with("ch") || word.ends_with('s')) {
        (1, _) => "",
        (_, true) => "es",
        _ => "s",
    };
    format!("{n} {word}{ending}")
}

/// Searches the files under `root` for `pattern`, handing each file with
/// matches to `on_file` as it's found. Stops when `cancel` is set, or at
/// `MAX_MATCHES` (returning `true`, cut short). Blocking: run it on a
/// thread.
pub fn search_files(
    root: &Path,
    pattern: &SearchPattern,
    options: &WalkOptions,
    unsaved: &HashMap<PathBuf, String>,
    cancel: &AtomicBool,
    mut on_file: impl FnMut(FileMatches),
) -> bool {
    let mut found = 0;
    let mut truncated = false;
    walk_files(root, options, cancel, |batch| {
        for relative in batch {
            if cancel.load(Ordering::Relaxed) || truncated {
                return;
            }
            let path = root.join(&relative);
            // An open buffer's unsaved text, else the file's.
            let open = (!unsaved.is_empty())
                .then(|| std::fs::canonicalize(&path).ok())
                .flatten()
                .and_then(|p| unsaved.get(&p));
            let text = match open {
                Some(text) => std::borrow::Cow::Borrowed(text.as_str()),
                None => {
                    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(&path) else {
                        continue;
                    };
                    // Binary files (a NUL near the start) aren't searched.
                    if bytes.iter().take(8192).any(|&b| b == 0) {
                        continue;
                    }
                    std::borrow::Cow::Owned(String::from_utf8_lossy(&bytes).into_owned())
                }
            };
            let mut lines = Vec::new();
            for (row, line) in text.lines().enumerate() {
                let ranges = pattern.matches_in_line(line);
                if ranges.is_empty() {
                    continue;
                }
                found += ranges.len();
                lines.push(LineMatch {
                    row,
                    text: line.to_string(),
                    ranges,
                });
                if found >= MAX_MATCHES {
                    truncated = true;
                    break;
                }
            }
            if !lines.is_empty() {
                on_file(FileMatches {
                    path: relative,
                    lines,
                });
            }
            if truncated {
                cancel.store(true, Ordering::Relaxed);
                return;
            }
        }
    });
    truncated
}

impl Editor {
    pub fn project_search(&self) -> &ProjectSearch {
        &self.project_search
    }

    /// Whether the Search view's query field has the keys.
    pub fn search_typing(&self) -> bool {
        self.project_search.typing && self.mode() == Mode::Find
    }

    /// `<Space>F`: the Search view, its query field focused and selected
    /// (seeded with the selection or the word under the cursor when it's
    /// empty).
    pub fn open_project_search(&mut self) -> Result<(), String> {
        let seed = self
            .project_search
            .fields
            .find
            .text()
            .is_empty()
            .then(|| self.word_under_cursor(false))
            .flatten();
        if self
            .sidebar_view_names()
            .get(self.sidebar_view)
            .is_none_or(|n| *n != "search")
            || self.sidebar().is_none()
        {
            self.open_sidebar_view("search")?;
        }
        match seed {
            Some(word) => {
                self.project_search.fields.find.set_selected(&word);
                self.search_changed();
            }
            None => self.project_search.fields.find.select_all(),
        }
        self.search_focus_query();
        Ok(())
    }

    /// `/` in the results (or a click on the field): back to the query.
    pub fn search_focus_query(&mut self) {
        self.project_search.fields.focus = FindField::Find;
        self.search_focus_field();
    }

    /// Into the field `fields.focus` names.
    fn search_focus_field(&mut self) {
        self.project_search.typing = true;
        self.set_mode(Mode::Find);
    }

    /// From the query to the results (`<CR>`, `<Down>`, `<Tab>`), the
    /// first match selected.
    pub(super) fn search_focus_results(&mut self) {
        self.project_search.follow.set(true);
        self.project_search.typing = false;
        self.set_mode(Mode::Sidebar);
        // The selection kept if it's on a match, else the first match.
        let rows = self.project_search.rows();
        let on_match = matches!(
            rows.get(self.project_search.selected),
            Some(SearchRow::Line(..))
        );
        let first = rows.iter().position(|r| matches!(r, SearchRow::Line(..)));
        drop(rows);
        if let (false, Some(first)) = (on_match, first) {
            self.project_search.selected = first;
        }
    }

    /// A click on the replace field: typing the replacement.
    pub fn search_focus_replace(&mut self) {
        self.project_search.fields.focus = FindField::Replace;
        self.search_focus_field();
    }

    /// A click elsewhere while typing the query: out of the field, to
    /// normal mode (the click then goes where it was).
    pub fn search_stop_typing(&mut self) {
        self.search_leave();
    }

    /// `<Esc>` in the query: back to the editor, the view left open.
    pub(super) fn search_leave(&mut self) {
        self.project_search.typing = false;
        self.set_mode(Mode::Normal);
    }

    /// After the query or a toggle changes: starts searching again.
    pub(super) fn search_changed(&mut self) {
        self.project_search.fields.rebuild();
        self.project_search.files.clear();
        self.project_search.folded.clear();
        self.project_search.selected = 0;
        self.project_search.scroll.set(0);
        self.project_search.follow.set(true);
        self.project_search.truncated = false;
        self.last_job += 1;
        self.project_search.job = self.last_job;
        match self.project_search.fields.pattern() {
            Some(pattern) => {
                let root = std::env::current_dir().unwrap_or_default();
                self.project_search.root = root.clone();
                self.project_search.searching = true;
                let unsaved = std::iter::once(&self.document)
                    .chain(&self.background)
                    .filter(|doc| doc.is_modified())
                    .filter_map(|doc| {
                        let path = std::fs::canonicalize(doc.path()?).ok()?;
                        Some((path, doc.buffer().text()))
                    })
                    .collect();
                self.project_search.request = Some(SearchRequest {
                    job: self.project_search.job,
                    root,
                    unsaved,
                    pattern: pattern.clone(),
                    options: WalkOptions {
                        hidden: self.options.file_picker.hidden,
                        git_ignore: self.options.file_picker.git_ignore,
                        max_files: self.options.file_picker.max_files,
                    },
                });
            }
            None => {
                self.project_search.searching = false;
                self.project_search.request = None;
            }
        }
    }

    /// The search to start, for the app.
    pub fn take_search_request(&mut self) -> Option<SearchRequest> {
        self.project_search.request.take()
    }

    /// The job the results shown are from (the app drops older ones').
    pub fn search_job(&self) -> u64 {
        self.project_search.job
    }

    /// A file's matches from search `job`.
    pub fn search_found(&mut self, job: u64, file: FileMatches) {
        if job == self.project_search.job {
            self.project_search.files.push(file);
        }
    }

    /// Search `job` finished (or was cut short at `MAX_MATCHES`).
    pub fn search_done(&mut self, job: u64, truncated: bool) {
        if job == self.project_search.job {
            self.project_search.searching = false;
            self.project_search.truncated = truncated;
            // Files in name order, as they'd be listed.
            self.project_search
                .files
                .sort_by(|a, b| a.path.cmp(&b.path));
        }
    }

    /// Whether the Search view is the one the sidebar shows.
    pub fn search_view_shown(&self) -> bool {
        self.sidebar().is_some()
            && self.sidebar_view_names().get(self.sidebar_view) == Some(&"search")
    }

    /// The mouse wheel over the results: scrolls them, the selection left
    /// where it is (as in VS Code).
    pub fn search_scroll(&mut self, delta: isize) {
        let search = &self.project_search;
        search.follow.set(false);
        let top = search.scroll.get() as isize + delta;
        let last = search.row_count().saturating_sub(1) as isize;
        search.scroll.set(top.clamp(0, last.max(0)) as usize);
    }

    pub(super) fn search_move(&mut self, delta: isize) {
        self.project_search.follow.set(true);
        let last = self.project_search.row_count().saturating_sub(1) as isize;
        self.project_search.selected =
            (self.project_search.selected as isize + delta).clamp(0, last.max(0)) as usize;
    }

    /// `<CR>` in the results: opens the match (in the editor), or folds a
    /// file's matches away (or back).
    pub(super) fn search_choose(&mut self) -> Result<(), String> {
        self.project_search.follow.set(true);
        let rows = self
            .project_search
            .rows_from(self.project_search.selected, 1);
        let Some(row) = rows.first().copied() else {
            return Ok(());
        };
        match row {
            SearchRow::File(file, _) => {
                let path = file.path.clone();
                if !self.project_search.folded.remove(&path) {
                    self.project_search.folded.insert(path);
                }
                Ok(())
            }
            SearchRow::Line(file, line) => {
                let path = file.path.clone();
                let at = Position::new(line.row, line.ranges.first().map_or(0, |r| r.0));
                self.set_mode(Mode::Normal);
                self.open(&path).map_err(|e| e.to_string())?;
                self.push_jump();
                self.document.set_cursor(at);
                self.reveal_cursor();
                if let Some(pattern) = self.project_search.fields.pattern().cloned() {
                    // `n` and `N` carry on through the file.
                    self.set_last_search(pattern);
                }
                Ok(())
            }
        }
    }

    /// A click on the results list, `row` rows down from its top.
    pub(super) fn search_click(&mut self, row: usize) -> Result<(), String> {
        self.project_search.typing = false;
        self.set_mode(Mode::Sidebar);
        let index = self.project_search.scroll.get() + row;
        if index >= self.project_search.row_count() {
            return Ok(());
        }
        self.project_search.selected = index;
        self.search_choose()
    }

    /// What the view says about the search, for its status row.
    pub fn search_status(&self) -> String {
        let search = &self.project_search;
        if let Some(error) = search.fields.error() {
            return error.to_string();
        }
        if search.fields.find.text().is_empty() {
            return String::new();
        }
        let (matches, files) = search.totals();
        if search.searching && matches == 0 {
            return "Searching…".into();
        }
        if matches == 0 {
            return "No results".into();
        }
        let more = if search.truncated { "+" } else { "" };
        let searching = if search.searching { "…" } else { "" };
        format!(
            "{}{more} in {}{searching}",
            plural(matches, "result"),
            plural(files, "file")
        )
    }

    /// Whether the results show replacements: when something's typed in
    /// the replace field, or it has the keys.
    pub fn search_previewing(&self) -> bool {
        let fields = &self.project_search.fields;
        !fields.replace.text().is_empty()
            || (self.search_typing() && fields.focus == FindField::Replace)
    }

    /// What each match in `found` would become (its `$1`s expanded, with
    /// a regex), in the order of its ranges.
    pub fn search_replacements(&self, found: &LineMatch) -> Vec<String> {
        let fields = &self.project_search.fields;
        let (Some(pattern), replacement) = (fields.pattern(), fields.literal_replacement()) else {
            return Vec::new();
        };
        found
            .ranges
            .iter()
            .map(|&(start, _)| {
                pattern
                    .next_replacement(&found.text, start, &replacement)
                    .map(|(_, _, text)| text)
                    .unwrap_or_default()
            })
            .collect()
    }

    /// `<CR>` in the replace field (or its chip): asks, then replaces every
    /// match.
    pub fn search_ask_replace_all(&mut self) -> Result<(), String> {
        let search = &self.project_search;
        if search.fields.pattern().is_none() {
            return Err("Nothing to replace: type what to find first".into());
        }
        if search.searching {
            return Err("Still searching…".into());
        }
        let (matches, files) = search.totals();
        if matches == 0 {
            return Err("No results to replace".into());
        }
        let replacement = search.fields.replace.text().to_string();
        let span = |text: &str, style: Option<&str>| super::popup::PopupSpan {
            text: text.to_string(),
            style: style.map(str::to_string),
        };
        let lines = vec![
            vec![
                span(
                    &format!(
                        "Replace {} in {} with ",
                        plural(matches, "match"),
                        plural(files, "file")
                    ),
                    None,
                ),
                span(&format!("“{replacement}”"), Some("ui.popup.title")),
                span("?", None),
            ],
            vec![],
            vec![span(
                "Each file's replacements are one edit in its buffer:",
                None,
            )],
            vec![span("u undoes them, :wa saves them.", None)],
            vec![],
            vec![
                span("y", Some("ui.popup.title")),
                span(" replace   ", None),
                span("n", Some("ui.popup.title")),
                span(" cancel", None),
            ],
        ];
        self.project_search.typing = false;
        self.open_popup(super::popup::Popup {
            name: "search-replace".into(),
            title: "Replace in files".into(),
            lines,
            keys: vec![
                ("y".into(), "search_replace_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..super::popup::Popup::default()
        });
        Ok(())
    }

    /// `y` in that popup: replaces every match, file by file.
    pub fn search_replace_confirmed(&mut self) -> Result<(), String> {
        if self.popup().is_none_or(|p| p.name != "search-replace") {
            return Err("Nothing to replace".into());
        }
        self.close_popup();
        let paths: Vec<PathBuf> = self
            .project_search
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect();
        let (mut replaced, mut files, mut problems) = (0, 0, Vec::new());
        for path in &paths {
            match self.replace_in_file(path, None) {
                Ok(0) => {}
                Ok(n) => {
                    replaced += n;
                    files += 1;
                }
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        self.after_replacing(replaced, files, problems);
        Ok(())
    }

    /// `r` in the results: replaces the selected match's line, or all of
    /// the selected file's matches.
    pub fn search_replace_selected(&mut self) -> Result<(), String> {
        if !self.search_view_shown() {
            return Err("Only in the Search view".into());
        }
        if self.project_search.fields.pattern().is_none() {
            return Err("Nothing to replace".into());
        }
        let selected = self.project_search.selected;
        let rows = self.project_search.rows_from(selected, 1);
        let Some(row) = rows.first().copied() else {
            return Err("No result selected".into());
        };
        let (path, only) = match row {
            SearchRow::File(file, _) => (file.path.clone(), None),
            SearchRow::Line(file, line) => (file.path.clone(), Some((line.row, line.text.clone()))),
        };
        drop(rows);
        let replaced = self.replace_in_file(&path, only)?;
        self.after_replacing(replaced, usize::from(replaced > 0), Vec::new());
        // Searching again keeps the selection about where it was.
        self.project_search.selected = selected;
        Ok(())
    }

    /// Replaces the matches in the file at `relative` (in the folder
    /// searched): all of them, or `only` those on that row, if it still
    /// reads as it did. One undo step in the file's buffer, which is opened
    /// in the background if it isn't open. Returns how many.
    fn replace_in_file(
        &mut self,
        relative: &Path,
        only: Option<(usize, String)>,
    ) -> Result<usize, String> {
        let pattern = self
            .project_search
            .fields
            .pattern()
            .cloned()
            .ok_or("Nothing to replace")?;
        let replacement = self.project_search.fields.literal_replacement();
        let path = self.project_search.root.join(relative);
        let id = self.buffer_for_path(&path)?;
        let document = self.document_by_id_mut(id).ok_or("The buffer went")?;
        let rows: Vec<usize> = match &only {
            Some((row, text)) => {
                if document.buffer().line(*row) != *text {
                    return Err("it changed since the search (R searches again)".into());
                }
                vec![*row]
            }
            // Bottom up, so lines put in don't move the rows still to do.
            None => (0..document.buffer().line_count()).rev().collect(),
        };
        let mut replaced = 0;
        for row in rows {
            let line = document.buffer().line(row).into_owned();
            if !pattern.is_match(&line) {
                continue;
            }
            let (text, count) = pattern.replace(&line, &replacement, true);
            if text != line {
                let end = document.buffer().line_end(row);
                document.delete(Position::new(row, 0), end);
                document.insert(Position::new(row, 0), &text);
            }
            replaced += count;
        }
        document.commit();
        Ok(replaced)
    }

    /// Says what was replaced, and searches again (the buffers' new text).
    fn after_replacing(&mut self, replaced: usize, files: usize, problems: Vec<String>) {
        let done = format!(
            "Replaced {} in {}: unsaved (:wa saves them, u in a file undoes)",
            plural(replaced, "match"),
            plural(files, "file")
        );
        self.search_changed();
        if problems.is_empty() {
            self.set_message(Message::info(done));
        } else {
            self.set_message(Message::error(format!(
                "{done}; not replaced: {}",
                problems.join("; ")
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::search::SearchPattern;

    #[test]
    fn searches_files_skipping_binary_and_ignored_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/a.rs"),
            "fn alpha() {}\nlet beta = alpha();\n",
        )
        .unwrap();
        std::fs::write(root.join("b.txt"), "no match here\nALPHA\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"alpha\0\x01").unwrap();
        let pattern = SearchPattern::from_regex("alpha", "alpha", true).unwrap();
        let mut found = Vec::new();
        let truncated = search_files(
            root,
            &pattern,
            &WalkOptions::default(),
            &HashMap::new(),
            &AtomicBool::new(false),
            |file| found.push(file),
        );
        assert!(!truncated);
        found.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<_> = found.iter().map(|f| f.path.display().to_string()).collect();
        assert_eq!(paths, ["b.txt", "src/a.rs"], "the binary file is skipped");
        let a = &found[1];
        assert_eq!(a.count(), 2);
        assert_eq!(a.lines[1].row, 1);
        assert_eq!(a.lines[1].ranges, [(11, 16)]);
    }

    fn line(row: usize) -> LineMatch {
        LineMatch {
            row,
            text: "x".into(),
            ranges: vec![(0, 1)],
        }
    }

    #[test]
    fn rows_from_is_a_window_on_the_rows() {
        let mut search = ProjectSearch::default();
        for (name, lines) in [("a", 2), ("b", 0), ("c", 3), ("d", 1)] {
            search.files.push(FileMatches {
                path: PathBuf::from(name),
                lines: (0..lines).map(line).collect(),
            });
        }
        search.folded.insert(PathBuf::from("c"));
        let all = search.rows();
        assert_eq!(search.row_count(), all.len());
        let key = |rows: &[SearchRow]| {
            rows.iter()
                .map(|r| match r {
                    SearchRow::File(f, _) => f.path.display().to_string(),
                    SearchRow::Line(f, l) => format!("{}:{}", f.path.display(), l.row),
                })
                .collect::<Vec<_>>()
        };
        for first in 0..=all.len() {
            for count in 0..=all.len() + 1 {
                let end = (first + count).min(all.len());
                assert_eq!(
                    key(&search.rows_from(first, count)),
                    key(&all[first.min(all.len())..end]),
                    "{first} {count}"
                );
            }
        }
    }
}
