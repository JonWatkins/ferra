//! What the coding agent can learn from the editor, and ask of it: what's
//! open and selected (sent with each prompt, and as a tool), the
//! diagnostics, and the language servers' answers (the agent's tool calls
//! become `LspRequest`s whose answers come back here, as text, instead of
//! being shown). The tools themselves are served in `agent/tools.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::lsp::{LspFeature, LspQuery, LspRequest, Reply};
use super::{DocumentId, Editor};
use crate::core::{Position, Range};

/// A language server's answer to one of the agent's tool calls: what to
/// tell it, and the buffers it changed (whose problems are waited for).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentAnswer {
    pub call: u64,
    pub answer: Result<String, String>,
    pub changed: Vec<DocumentId>,
}

/// At most this much of a selection goes with a prompt.
const SELECTION_CHARS: usize = 4000;
/// Lines `read_buffer` gives at most, unless asked for more.
const READ_LINES: usize = 2000;
/// The most places `show_locations` takes.
const MOST_LOCATIONS: usize = 500;
/// The terminal's last lines `read_terminal` gives, unless asked for more.
const TERMINAL_LINES: usize = 200;
/// The Output tab's last lines `language_servers` gives.
const LOG_LINES: usize = 50;
/// The most lines of diff `buffer_changes` gives for a file.
const DIFF_LINES: usize = 1000;

impl Editor {
    /// A short note of what's open, for a prompt (`agent-context`): the
    /// file and where the cursor is, what's selected, the other files open.
    pub fn agent_context(&self) -> Option<String> {
        if !self.options.agent_context {
            return None;
        }
        let path = self.document.path()?;
        let cursor = self.document.cursor();
        let mut note = format!(
            "The user is in the editor (ferra), at {}:{}:{}{}.",
            shown(path),
            cursor.row + 1,
            cursor.col + 1,
            if self.document.is_modified() {
                " (unsaved changes)"
            } else {
                ""
            }
        );
        if let Some(symbol) = self.agent_enclosing_symbol() {
            note.push_str(&format!("\nIn: {symbol}"));
        }
        if let Some((first, last, text)) = self.agent_selection() {
            let text = clipped(&text, SELECTION_CHARS);
            note.push_str(&format!(
                "\nSelected, lines {}-{}:\n```\n{text}\n```",
                first + 1,
                last + 1
            ));
        }
        // What's selected in the panel and the sidebar ("that one").
        for view in self.agent_view_notes() {
            note.push('\n');
            note.push_str(&view);
        }
        let others: Vec<String> = self
            .background
            .iter()
            .filter_map(|d| d.path().map(shown))
            .take(20)
            .collect();
        if !others.is_empty() {
            note.push_str(&format!("\nAlso open: {}", others.join(", ")));
        }
        Some(note)
    }

    /// The selection: its first and last rows, and its text.
    fn agent_selection(&self) -> Option<(usize, usize, String)> {
        let range = self.selection_range()?;
        let buffer = self.document.buffer();
        let (first, last) = range.rows();
        let text = match range {
            Range::Chars { start, end } => buffer.text_range(start, end),
            Range::Lines { first, last } => (first..=last)
                .map(|row| buffer.line(row).to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        };
        Some((first, last, text))
    }

    /// The `editor_state` tool: what's open and where.
    pub fn agent_editor_state(&self) -> String {
        let mut lines = Vec::new();
        match self.document.path() {
            Some(path) => {
                let cursor = self.document.cursor();
                lines.push(format!(
                    "Current file: {} (cursor at line {}, column {}{})",
                    shown(path),
                    cursor.row + 1,
                    cursor.col + 1,
                    if self.document.is_modified() {
                        ", unsaved changes"
                    } else {
                        ""
                    }
                ));
            }
            None => lines.push("Current buffer: an unnamed one (no file)".into()),
        }
        lines.extend(self.agent_where_notes());
        if let Some((first, last, text)) = self.agent_selection() {
            lines.push(format!(
                "Selected, lines {}-{}:\n```\n{}\n```",
                first + 1,
                last + 1,
                clipped(&text, SELECTION_CHARS)
            ));
        } else {
            lines.push("Nothing selected".into());
        }
        let open: Vec<String> = std::iter::once(&self.document)
            .chain(&self.background)
            .filter_map(|d| {
                let Some(path) = d.path() else {
                    // Named as buffer_changes names it, if there's text.
                    return (!d.buffer().text().is_empty())
                        .then(|| "- [unnamed buffer] (unsaved)".to_string());
                };
                let note = if d.is_modified() {
                    " (unsaved)"
                } else if self.agent.opened.contains(&d.id()) {
                    " (opened for your tools: closes when your turn ends)"
                } else {
                    ""
                };
                Some(format!("- {}{note}", shown(path)))
            })
            .collect();
        let splits = self.windows().layout().windows();
        if splits.len() > 1 {
            let shows: Vec<String> = splits
                .iter()
                .map(|&w| {
                    let name = self
                        .window_view(w)
                        .and_then(|(d, _)| d.path().map(shown))
                        .unwrap_or_else(|| "an unnamed buffer".into());
                    if w == self.windows().active() {
                        format!("{name} (focused)")
                    } else {
                        name
                    }
                })
                .collect();
            lines.push(format!(
                "{} splits, showing: {}",
                splits.len(),
                shows.join(", ")
            ));
        }
        if !open.is_empty() {
            lines.push(format!("Open files:\n{}", open.join("\n")));
        }
        lines.extend(self.agent_view_notes());
        if let Some(terminal) = self.agent_terminal_note() {
            lines.push(format!("{terminal} (read_terminal reads it)"));
        }
        lines.join("\n")
    }

    /// The symbols the cursor's in, from the language server's outline:
    /// `impl Editor > agent_editor_state (method, lines 84-151)`.
    fn agent_enclosing_symbol(&self) -> Option<String> {
        let symbols = self.outline()?;
        let path = self.symbols_at_cursor();
        let symbol = &symbols[*path.last()?];
        let names: Vec<&str> = path.iter().map(|&i| symbols[i].name.as_str()).collect();
        Some(format!(
            "{} ({}, lines {}-{})",
            names.join(" > "),
            symbol.kind,
            symbol.start.row + 1,
            symbol.end.row + 1
        ))
    }

    /// Where in the current file the user is: the symbols the cursor's in
    /// (from the language server's outline), the lines on screen, and the
    /// last places changed.
    fn agent_where_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        let cursor = self.document.cursor();
        if let Some(symbol) = self.agent_enclosing_symbol() {
            notes.push(format!("In: {symbol}"));
        }
        let windows = self.windows();
        let active = windows.active();
        let height = windows.height(active);
        if height > 0 {
            let top = windows.view_top(active, &self.document, cursor, &self.options);
            let layout = super::windows::TextLayout::new(
                windows.width(active),
                &self.document,
                &self.options,
            );
            let bottom = layout.bottom(&self.document, top, height);
            notes.push(format!("Showing lines {}-{}", top + 1, bottom + 1));
        }
        let changes: Vec<String> = self
            .document
            .recent_changes(5)
            .iter()
            .map(|p| (p.row + 1).to_string())
            .collect();
        if !changes.is_empty() {
            notes.push(format!(
                "Last changed here (newest first): line {}",
                changes.join(", ")
            ));
        }
        notes
    }

    /// What the user's looking at beside the text: the Problems panel's
    /// selected problem, the Search view's search, Source Control's
    /// selected file; and a merge with conflicts, if there's one.
    fn agent_view_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.panel_shown() == Some(super::panel::PanelTab::Problems) {
            let rows = self.problem_rows();
            match rows.get(self.panel.selected) {
                Some(super::panel::ProblemRow::Problem { path, diagnostic }) => {
                    notes.push(format!(
                        "Problems panel: open, selected {}:{}: {}",
                        shown(path),
                        diagnostic.start.row + 1,
                        diagnostic.describe()
                    ));
                }
                _ => {
                    let count = rows
                        .iter()
                        .filter(|r| matches!(r, super::panel::ProblemRow::Problem { .. }))
                        .count();
                    let plural = if count == 1 { "" } else { "s" };
                    notes.push(format!("Problems panel: open ({count} problem{plural})"));
                }
            }
        }
        if self.panel_shown() == Some(super::panel::PanelTab::Locations)
            && let Some(Some((path, position))) = self.panel_selected_place()
        {
            let note = self
                .location_rows()
                .get(self.panel.selected)
                .and_then(|row| match row {
                    super::panel::LocationRow::Item(item) => Some(item.note.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let places = self.list_places_count();
            let at = self.panel_selected_number();
            let title = self
                .panel
                .locations
                .as_ref()
                .map(|l| l.title.clone())
                .unwrap_or_default();
            notes.push(format!(
                "Locations tab (\"{title}\"): open, selected {at} of {places}, {}:{}:{}: {note}",
                shown(&path),
                position.row + 1,
                position.col + 1
            ));
        }
        if self.sidebar().is_some() {
            match self.sidebar_shows() {
                super::sidebar_views::SidebarShows::Search => {
                    let search = self.project_search();
                    let query = search.fields.find.text();
                    if !query.is_empty() {
                        let (matches, files) = search.totals();
                        notes.push(format!(
                            "Search view: \"{query}\" ({matches} matches in {files} files)"
                        ));
                    }
                }
                super::sidebar_views::SidebarShows::Git => {
                    if let Some(super::git::GitRow::File { file, staged }) =
                        self.git_rows().get(self.git_selected())
                    {
                        notes.push(format!(
                            "Source Control view: selected {} ({})",
                            file.path,
                            if *staged { "staged" } else { "not staged" }
                        ));
                    }
                }
                _ => {}
            }
        }
        if let Some(merge) = self.git_merging() {
            let conflicted: Vec<String> = std::iter::once(&self.document)
                .chain(&self.background)
                .filter_map(|d| {
                    let path = d.path()?;
                    let buffer = d.buffer();
                    // Most files have no markers: no need to read them line by line.
                    if !buffer.text().contains("<<<<<<<") {
                        return None;
                    }
                    let text: Vec<String> = (0..buffer.line_count())
                        .map(|row| buffer.line(row).into_owned())
                        .collect();
                    let count = crate::git::conflicts(text.iter().map(String::as_str)).len();
                    (count > 0).then(|| format!("{} ({count})", shown(path)))
                })
                .collect();
            let first = merge.lines().next().unwrap_or("a merge");
            if conflicted.is_empty() {
                notes.push(format!(
                    "Merging ({first}): no conflicts left in the open files"
                ));
            } else {
                notes.push(format!(
                    "Merging ({first}): conflicts in {}",
                    conflicted.join(", ")
                ));
            }
        }
        notes
    }

    /// One line on the terminal panel's shell, if there is one: whether
    /// it's shown, and what it's doing.
    fn agent_terminal_note(&self) -> Option<String> {
        let terminal = self.terminal.as_ref()?;
        let shown = if self.terminal_panel().is_some() {
            "shown"
        } else {
            "hidden"
        };
        let doing = match (terminal.exited(), terminal.running_command()) {
            (Some(why), _) => format!("the shell has {why}"),
            (None, Some(command)) => format!("running `{command}`"),
            (None, None) => "at the shell's prompt".into(),
        };
        let title = terminal.title();
        Some(format!(
            "Terminal panel ({shown}, titled \"{title}\"): {doing}"
        ))
    }

    /// The `buffer_changes` tool: a unified diff of a file as the editor has
    /// it against the saved file (`against` "saved", the default) or against
    /// git's staged text ("git", what the gutter shows). With no `path`,
    /// every open file that has such changes.
    pub fn agent_buffer_changes(
        &self,
        path: Option<&str>,
        against: Option<&str>,
    ) -> Result<String, String> {
        let git = match against.unwrap_or("saved") {
            "saved" => false,
            "git" => true,
            other => return Err(format!("against is \"saved\" or \"git\", not \"{other}\"")),
        };
        // The files (none for a buffer with no name), with their text as
        // saving would write it (final newline, line endings).
        let as_saved = |d: &super::Document| d.format().encode(&d.buffer().text());
        let files: Vec<(Option<PathBuf>, String)> = match path {
            Some(path) => {
                let full = absolute(Path::new(path));
                let text = match self
                    .find_open_path(&full)
                    .and_then(|id| self.document_by_id(id))
                {
                    Some(document) => as_saved(document),
                    None => std::fs::read_to_string(&full).map_err(|e| match e.kind() {
                        std::io::ErrorKind::NotFound => format!("No such file: {path}"),
                        _ => format!("Can't read {path}: {e}"),
                    })?,
                };
                vec![(Some(full), text)]
            }
            None => std::iter::once(&self.document)
                .chain(&self.background)
                .filter(|d| {
                    if git {
                        d.path().is_some()
                    } else {
                        d.is_modified() && (d.path().is_some() || !d.buffer().text().is_empty())
                    }
                })
                .map(|d| (d.path().map(Path::to_path_buf), as_saved(d)))
                .collect(),
        };
        let mut out: Vec<String> = Vec::new();
        for (full, text) in &files {
            let Some(full) = full else {
                // A new buffer, never saved: all of it is new.
                out.push(file_diff(
                    "",
                    text,
                    "[unnamed buffer] (not saved yet)",
                    "[unnamed buffer] (editor)",
                    path.is_some(),
                ));
                continue;
            };
            // The path as asked for (not where a link led), or as shown.
            let name = path.map_or_else(|| shown(full), str::to_string);
            let (base, old) = if git {
                match crate::git::staged_text(full) {
                    Some(base) => (base, format!("{name} (git, staged)")),
                    None => {
                        let in_repo = full.parent().and_then(crate::git::repo_root).is_some();
                        if in_repo {
                            // Untracked: all of it is new to git.
                            (String::new(), format!("{name} (untracked)"))
                        } else if path.is_some() {
                            out.push(format!("{name}: not in a git repository"));
                            continue;
                        } else {
                            // Asked about every file: those outside git aren't news.
                            continue;
                        }
                    }
                }
            } else {
                match std::fs::read_to_string(full) {
                    Ok(base) => (base, format!("{name} (saved)")),
                    // Not saved yet: all of it is new.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        (String::new(), format!("{name} (not saved yet)"))
                    }
                    Err(e) => {
                        out.push(format!("{name}: can't read the saved file ({e})"));
                        continue;
                    }
                }
            };
            if base == *text {
                continue;
            }
            out.push(file_diff(
                &base,
                text,
                &old,
                &format!("{name} (editor)"),
                path.is_some(),
            ));
        }
        if out.is_empty() {
            return Ok(match (path, git) {
                (Some(_), false) => "No unsaved changes".into(),
                (Some(_), true) => "No changes from git's staged text".into(),
                (None, false) => "No open file has unsaved changes".into(),
                (None, true) => "No open file has changes from git's staged text".into(),
            });
        }
        Ok(out.join("\n\n"))
    }

    /// The `show_locations` tool: puts places in the panel's Locations tab
    /// for the user to step through (`↵` goes to one), each with a note
    /// (or its line's text). Focus stays where it is.
    pub fn agent_show_locations(
        &mut self,
        title: &str,
        places: &[serde_json::Value],
    ) -> Result<String, String> {
        if places.is_empty() {
            return Err("No locations given".into());
        }
        if places.len() > MOST_LOCATIONS {
            return Err(format!("At most {MOST_LOCATIONS} locations"));
        }
        let mut items = Vec::new();
        for (i, place) in places.iter().enumerate() {
            let which = format!("location {}", i + 1);
            let path = place["path"]
                .as_str()
                .filter(|p| !p.is_empty())
                .ok_or(format!("{which}: no path"))?;
            let full = absolute(Path::new(path));
            if full.is_dir() {
                return Err(format!("{which}: {path} is a folder, not a file"));
            }
            let text = match self
                .find_open_path(&full)
                .and_then(|id| self.document_by_id(id))
            {
                Some(document) => document.buffer().text(),
                None => std::fs::read_to_string(&full)
                    .map_err(|_| format!("{which}: no such file: {path}"))?,
            };
            let line = match place["line"].as_u64() {
                Some(0) => return Err(format!("{which}: lines start at 1, not 0")),
                Some(line) => line as usize,
                None => return Err(format!("{which}: no line")),
            };
            let Some(line_text) = text.lines().nth(line - 1) else {
                return Err(format!(
                    "{which}: {path} has {} lines, not {line}",
                    text.lines().count()
                ));
            };
            let column = place["column"].as_u64().unwrap_or(1).max(1) as usize;
            let column = column.min(line_text.chars().count() + 1);
            let note = place["note"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(line_text.trim())
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            items.push(super::panel::LocationItem {
                path: full,
                position: Position::new(line - 1, column - 1),
                note,
            });
        }
        let count = items.len();
        let mut files: Vec<&PathBuf> = items.iter().map(|i| &i.path).collect();
        files.sort();
        files.dedup();
        let files = files.len();
        let title = title.trim();
        let title = if title.is_empty() { "Locations" } else { title };
        self.panel_show_locations(super::panel::LocationList {
            title: title.to_string(),
            items,
        });
        Ok(format!(
            "Showing {count} place{} in {files} file{} in the panel's Locations tab \
             (the user steps through them with j/k and ↵)",
            if count == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" },
        ))
    }

    /// The `move_file` tool: checks the move can be made, then asks the
    /// file's language server for the edits it needs (imports), through
    /// `LspFeature::MoveFile`; [`Editor::agent_finish_move`] makes them and
    /// the move. With language servers off, the move's made now (`Some`).
    pub fn agent_move_file(
        &mut self,
        call: u64,
        from: &str,
        to: &str,
    ) -> Result<Option<(String, Vec<DocumentId>)>, String> {
        let source = absolute(Path::new(from));
        let target = absolute(Path::new(to));
        if source.is_dir() {
            return Err(format!("{from} is a folder: move_file moves files"));
        }
        if !source.is_file() {
            return Err(format!("No such file: {from}"));
        }
        if target.exists() {
            return Err(format!("{to} already exists: nothing was moved"));
        }
        if let Some(file) = target.ancestors().skip(1).find(|p| p.is_file()) {
            return Err(format!(
                "{} is a file, not a folder: nothing was moved",
                shown(file)
            ));
        }
        let id = self.agent_buffer(&source)?;
        if !self.options.lsp {
            let before = self.agent_unsaved();
            return self
                .agent_finish_move(id, &target, &before, None, false)
                .map(Some);
        }
        let buffer = self.document_by_id(id).ok_or("No such buffer")?.buffer();
        self.lsp_requests.push(LspRequest {
            query: LspQuery::MoveFile {
                to: target.to_path_buf(),
            },
            document: id,
            position: Position::default(),
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            reply: Reply::Agent(call),
        });
        Ok(None)
    }

    /// Moves buffer `id`'s file to `to` (its buffer, and any other open in
    /// it, following), after the language server's edits for it were made
    /// (`edits`: what they were; `asked`: whether a server was asked); saves
    /// what the agent changed (as against
    /// `before`). The answer, and the buffers changed.
    pub fn agent_finish_move(
        &mut self,
        id: DocumentId,
        to: &Path,
        before: &HashMap<DocumentId, (u64, bool)>,
        edits: Option<String>,
        asked: bool,
    ) -> Result<(String, Vec<DocumentId>), String> {
        let from = self
            .document_by_id(id)
            .and_then(|d| d.path())
            .map(Path::to_path_buf)
            .ok_or("The file's buffer is gone")?;
        let (saved, mut changed) = self.agent_save_changed(before);
        if let Err(error) = self.move_path(&from, to) {
            return Err(match edits {
                Some(edits) => format!(
                    "{error}. The file wasn't moved, but its imports were already changed \
                     ({edits}): undo them (u in those files) or move it some other way"
                ),
                None => format!("{error}. Nothing was moved"),
            });
        }
        if !changed.contains(&id) {
            changed.push(id);
        }
        let updated = match (edits, asked) {
            (Some(edits), _) => format!("; updated for it: {edits}"),
            (None, true) => "; the language server made no edits for it, so check what \
                             refers to it (imports, mod lines): get_diagnostics shows what \
                             broke once the server's caught up"
                .to_string(),
            (None, false) => String::new(),
        };
        Ok((
            format!("Moved {} to {}{updated}{saved}", shown(&from), shown(to)),
            changed,
        ))
    }

    /// The `language_servers` tool: each server and how it is (starting,
    /// running, busy with something, stopped and why), the ones not found,
    /// and the Output tab's last lines (what they've logged).
    pub fn agent_language_servers(&self) -> String {
        if !self.options.lsp {
            return "Language servers are off in this editor (lsp = false)".into();
        }
        let mut out = Vec::new();
        let status = self.lsp_status();
        if status.is_empty() {
            out.push("No language servers: none is set for the open files' languages".to_string());
        } else {
            out.push("Language servers:".to_string());
            for line in status {
                match line.strip_prefix("PATH=") {
                    Some(path) => out.push(format!("The PATH ferra started with: {path}")),
                    None => out.push(format!("- {line}")),
                }
            }
        }
        // The Output tab's last lines (an entry can be several), newest last.
        let mut shown: Vec<String> = Vec::new();
        for entry in self.panel.output.iter().rev() {
            let mark = if entry.error { "error: " } else { "" };
            let mut lines: Vec<String> = entry
                .text
                .lines()
                .enumerate()
                .map(|(i, l)| {
                    if i == 0 {
                        format!("{} {mark}{l}", entry.time)
                    } else {
                        format!("         {l}")
                    }
                })
                .collect();
            if shown.len() + lines.len() > LOG_LINES {
                // Its last lines, if it's the first and too long by itself.
                if shown.is_empty() {
                    let from = lines.len() - LOG_LINES;
                    shown = lines.split_off(from);
                    shown.reverse();
                }
                break;
            }
            lines.reverse();
            shown.extend(lines);
        }
        shown.reverse();
        if !shown.is_empty() {
            let all: usize = self
                .panel
                .output
                .iter()
                .map(|e| e.text.lines().count().max(1))
                .sum();
            out.push(format!(
                "The Output tab (what they've logged), last {} of {all} lines:",
                shown.len()
            ));
            out.push("```".into());
            out.extend(shown);
            out.push("```".into());
        }
        out.join("\n")
    }

    /// The `read_terminal` tool: the terminal panel's last `lines` lines
    /// (scrollback included), and what its shell's doing.
    pub fn agent_read_terminal(&self, lines: Option<usize>) -> Result<String, String> {
        let terminal = self
            .terminal
            .as_ref()
            .ok_or("The terminal panel has no shell (the user hasn't opened it)")?;
        let note = self.agent_terminal_note().unwrap_or_default();
        let (text, redraws) = without_redraws(terminal.text_lines());
        if text.is_empty() {
            return Ok(format!("{note}\nNothing in it yet."));
        }
        let wanted = lines.unwrap_or(TERMINAL_LINES).max(1);
        let from = text.len().saturating_sub(wanted);
        let which = if from == 0 {
            format!("All {} lines", text.len())
        } else {
            format!(
                "Lines {}-{} of {} (ask for more with lines)",
                from + 1,
                text.len(),
                text.len()
            )
        };
        let left_out = match redraws {
            0 => String::new(),
            1 => " (1 progress redraw left out, in all)".into(),
            n => format!(" ({n} progress redraws left out, in all)"),
        };
        Ok(format!(
            "{note}\n{which}{left_out}:\n```\n{}\n```",
            text[from..].join("\n")
        ))
    }

    /// The `get_diagnostics` tool: the language servers' errors and
    /// warnings, for `path` or every file they've reported on.
    pub fn agent_diagnostics(&self, path: Option<&str>) -> String {
        let wanted = path.map(|p| absolute(Path::new(p)));
        let mut found: Vec<(PathBuf, &super::diagnostics::Diagnostic)> = Vec::new();
        for document in std::iter::once(&self.document).chain(&self.background) {
            let Some(path) = document.path() else {
                continue;
            };
            for d in document.diagnostics() {
                found.push((absolute(path), d));
            }
        }
        for (path, list) in &self.file_diagnostics {
            if self.find_open_path(path).is_none() {
                found.extend(list.iter().map(|d| (absolute(path), d)));
            }
        }
        if let Some(wanted) = &wanted {
            found.retain(|(path, _)| path == wanted);
        }
        found.sort_by(|(pa, a), (pb, b)| (a.severity, pa, a.start).cmp(&(b.severity, pb, b.start)));
        if found.is_empty() {
            return match path {
                Some(path) => format!("No diagnostics for {path}"),
                None => "No diagnostics".into(),
            };
        }
        found
            .iter()
            .take(200)
            .map(|(path, d)| {
                format!(
                    "{}:{}:{}  {}",
                    shown(path),
                    d.start.row + 1,
                    d.start.col + 1,
                    full_message(d)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The `open_file` tool: shows the user `path` (at `line`, from 1).
    pub fn agent_open_file(&mut self, path: &str, line: Option<usize>) -> Result<String, String> {
        let location = super::lsp::Location {
            path: absolute(Path::new(path)),
            position: Position::new(line.unwrap_or(1).saturating_sub(1), 0),
            line: String::new(),
        };
        if !location.path.is_file() {
            return Err(format!("{path} isn't a file"));
        }
        self.goto_location(&location)?;
        Ok(format!("Opened {path} for the user"))
    }

    /// The `read_buffer` tool: a file's text as the editor has it (unsaved
    /// changes and all), its lines numbered, from `line` (from 1) for
    /// `limit` lines.
    pub fn agent_read_buffer(
        &self,
        path: &str,
        line: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String, String> {
        let full = absolute(Path::new(path));
        let (text, unsaved) = match self
            .find_open_path(&full)
            .and_then(|id| self.document_by_id(id))
        {
            Some(document) => (document.buffer().text(), document.is_modified()),
            None => (
                std::fs::read_to_string(&full).map_err(|e| format!("{path}: {e}"))?,
                false,
            ),
        };
        let first = line.unwrap_or(1).max(1) - 1;
        let limit = limit.unwrap_or(READ_LINES);
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return Ok("(empty file)".into());
        }
        if first >= lines.len() {
            return Err(format!("{path} has {} lines", lines.len()));
        }
        let mut out = String::new();
        if unsaved {
            out.push_str("(Open in the editor with unsaved changes: this is the editor's text.)\n");
        }
        for (i, line) in lines.iter().enumerate().skip(first).take(limit) {
            out.push_str(&format!("{:>6}\t{line}\n", i + 1));
        }
        let shown = first + limit;
        if shown < lines.len() {
            out.push_str(&format!(
                "… {} more lines (read from line {})",
                lines.len() - shown,
                shown + 1
            ));
        }
        Ok(out)
    }

    /// The `edit_buffer` tool: each `(old, new, all)` replacement made in
    /// the file's buffer (opened if it isn't), together as one undo step,
    /// shown as it happens. Each `old` must be there once (or `all`
    /// replaces every one), checked before any is made, and they mustn't
    /// overlap. The file's saved if it had no unsaved changes, so the disk
    /// keeps up. Returns what was done and the buffer, for its diagnostics.
    pub fn agent_edit_buffer(
        &mut self,
        path: &str,
        replacements: &[(String, String, bool)],
    ) -> Result<(String, Vec<DocumentId>), String> {
        if replacements.is_empty() {
            return Err("No edits given".into());
        }
        let full = absolute(Path::new(path));
        if !full.is_file() {
            return Err(format!("{path} isn't a file (create_file makes a new one)"));
        }
        let id = self.agent_buffer(&full)?;
        let document = self.document_by_id(id).ok_or("No such buffer")?;
        let buffer = document.buffer();
        // The file's last newline isn't in the buffer (it's put back when
        // it's saved), but the agent sees it in the file: matched too.
        let text = format!("{}\n", buffer.text());
        let end = text.len() - 1;
        let several = replacements.len() > 1;
        // Each match: where, its new text, and which edit (from 0).
        let mut spans: Vec<(usize, usize, String, usize)> = Vec::new();
        for (i, (old, new, all)) in replacements.iter().enumerate() {
            let which = if several {
                format!("edit {}: ", i + 1)
            } else {
                String::new()
            };
            if old.is_empty() {
                return Err(format!(
                    "{which}old_text is empty: give the text to replace"
                ));
            }
            if old == new {
                return Err(format!("{which}old_text and new_text are the same"));
            }
            let found: Vec<usize> = text.match_indices(old.as_str()).map(|(at, _)| at).collect();
            match found.len() {
                0 => {
                    return Err(format!(
                        "{which}old_text isn't in {path} (as the editor has it: read_buffer \
                         shows it, whitespace and all). Nothing was changed."
                    ));
                }
                n if n > 1 && !all => {
                    return Err(format!(
                        "{which}old_text is in {path} {n} times: give more of the text around \
                         it, or replace_all. Nothing was changed."
                    ));
                }
                _ => {}
            }
            for at in found {
                let mut to = at + old.len();
                let mut new = new.clone();
                // Through the last newline: it stays.
                if to > end {
                    to = end;
                    if new.ends_with('\n') {
                        new.pop();
                    }
                }
                spans.push((at.min(end), to, new, i));
            }
        }
        spans.sort_by_key(|&(from, ..)| from);
        if let Some(pair) = spans.windows(2).find(|w| w[0].1 > w[1].0) {
            let (a, b) = (pair[0].3.min(pair[1].3), pair[0].3.max(pair[1].3));
            return Err(if a == b {
                format!("Edit {}'s places overlap: nothing was changed", a + 1)
            } else {
                format!("Edits {} and {} overlap: nothing was changed", a + 1, b + 1)
            });
        }
        let count = spans.len();
        // Edits that touch become one: trimmed on their own, each could
        // shrink to the same place, and one would land in the other's text.
        let mut merged: Vec<(usize, usize, String, usize)> = Vec::with_capacity(spans.len());
        for span in spans {
            match merged.last_mut() {
                Some(last) if last.1 == span.0 => {
                    last.1 = span.1;
                    last.2.push_str(&span.2);
                }
                _ => merged.push(span),
            }
        }
        let edits = merged
            .into_iter()
            .map(|(from, to, new, _)| {
                // Only what changes is replaced (the text the old and new
                // share at either end is left), so diagnostics and marks in
                // it stay put. (Overlaps were checked on the whole spans.)
                let (from, to, new) = trimmed(&text, from, to, &new);
                (
                    buffer.position_at_byte(from),
                    buffer.position_at_byte(to),
                    new,
                )
            })
            .collect();
        let before = self.agent_unsaved();
        self.edit_document(id, edits);
        let (saved, _) = self.agent_save_changed(&before);
        let what = if count == 1 {
            "1 replacement".to_string()
        } else {
            format!("{count} replacements")
        };
        Ok((format!("Edited {}: {what}{saved}", named(path)), vec![id]))
    }

    /// The `create_file` tool: a new file with `text` (its folders made),
    /// opened in a buffer (for its language server), not shown.
    pub fn agent_create_file(
        &mut self,
        path: &str,
        text: &str,
    ) -> Result<(String, Vec<DocumentId>), String> {
        let full = absolute(Path::new(path));
        if full.exists() {
            return Err(format!("{path} already exists: edit_buffer changes it"));
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{path}: {e}"))?;
        }
        std::fs::write(&full, text).map_err(|e| format!("{path}: {e}"))?;
        let id = self.agent_buffer(&absolute(&full))?;
        Ok((format!("Created {}", named(path)), vec![id]))
    }

    /// The `close_file` tool: closes `path`'s buffer (not one with unsaved
    /// changes, or shown in a split other than the one in use).
    pub fn agent_close_file(&mut self, path: &str) -> Result<String, String> {
        let full = absolute(Path::new(path));
        let Some(id) = self.find_open_path(&full) else {
            return Ok(format!("{path} isn't open"));
        };
        let document = self.document_by_id(id).ok_or("No such buffer")?;
        if document.is_modified() {
            return Err(format!(
                "{path} has unsaved changes: left open for the user"
            ));
        }
        if id == self.document.id() {
            if self.in_other_window(id) {
                return Err(format!(
                    "{path} is in another split too: left open for the user"
                ));
            }
            if self.background.is_empty() {
                return Err(format!(
                    "{path} is the only file open: left open for the user"
                ));
            }
            self.close_buffer();
        } else {
            if self.shown_in_a_window(id) {
                return Err(format!(
                    "{path} is shown in a split: left open for the user"
                ));
            }
            self.forget_tab(id);
            self.close_hidden_buffer(id);
        }
        self.agent.opened.remove(&id);
        Ok(format!("Closed {}", named(path)))
    }

    /// `path`'s buffer, opened (and noted as the agent's, to be closed when
    /// its turn ends) if it isn't open.
    fn agent_buffer(&mut self, path: &Path) -> Result<DocumentId, String> {
        if let Some(id) = self.find_open_path(path) {
            return Ok(id);
        }
        let id = self.buffer_for_path(path)?;
        self.agent.opened.insert(id);
        Ok(id)
    }

    /// The agent's turn ended: the buffers it opened to read or ask about
    /// go, unless they're shown, in a tab, or have unsaved changes.
    pub fn agent_close_its_buffers(&mut self) {
        for id in std::mem::take(&mut self.agent.opened) {
            let clean = self.document_by_id(id).is_some_and(|d| !d.is_modified());
            if clean {
                self.close_hidden_buffer(id);
            }
        }
    }

    /// The problems the language servers now report in the buffers the
    /// agent changed: errors and warnings, or that there are none.
    pub fn agent_problems(&self, ids: &[DocumentId]) -> String {
        let mut out = String::new();
        for &id in ids {
            let Some(document) = self.document_by_id(id) else {
                continue;
            };
            let name = document.path().map(shown).unwrap_or_default();
            let all = document.diagnostics();
            let unsaved = document.is_modified();
            // rust-analyzer's own guess where rustc says the same, dropped.
            let said_by_rustc = |d: &super::diagnostics::Diagnostic| {
                d.source.as_deref() == Some("rust-analyzer")
                    && all
                        .iter()
                        .any(|o| o.start == d.start && o.source.as_deref() == Some("rustc"))
            };
            let mut seen = Vec::new();
            let problems: Vec<String> = all
                .iter()
                .filter(|d| d.severity <= super::diagnostics::Severity::Warning)
                .filter(|d| !said_by_rustc(d))
                // The same problem twice (rust-analyzer can), once.
                .filter(|d| {
                    let key = (d.start, &d.message, &d.source);
                    let new = !seen.contains(&key);
                    seen.push(key);
                    new
                })
                .take(20)
                .map(|d| {
                    let mut line = format!(
                        "  {}:{}  {}",
                        d.start.row + 1,
                        d.start.col + 1,
                        full_message(d)
                    );
                    // Unsaved: what cargo check says is about the saved file.
                    if unsaved && on_save(d) {
                        line.truncate(line.len() - 1);
                        line.push_str(", from the saved file)");
                    }
                    line
                })
                .collect();
            if problems.is_empty() {
                out.push_str(&format!("\nNo errors or warnings in {name}."));
            } else {
                out.push_str(&format!("\nProblems in {name}:\n{}", problems.join("\n")));
            }
            // Not saved: checks run on save (cargo check) still describe
            // the file on disk.
            if unsaved && all.iter().any(on_save) {
                out.push_str(&format!(
                    "\n({name} isn't saved: those marked \"from the saved file\" come from a \
                     check that runs on save, like cargo check, so they may not match the editor's \
                     text.)"
                ));
            }
        }
        out
    }

    /// How many times buffer `id`'s diagnostics have been set (they're
    /// waited for after the agent's edit).
    pub fn diagnostics_version(&self, id: DocumentId) -> u64 {
        self.diagnostics_versions.get(&id).map_or(0, |v| v.0)
    }

    /// What a language server's formatting is told about buffer `id`'s
    /// indentation (`tabSize`, `insertSpaces`): its own (chosen or as the
    /// file has it), else its settings'.
    pub fn format_indent(&self, id: DocumentId) -> (usize, bool) {
        let options = self.options_for(self.windows.active(), id);
        let Some(document) = self.document_by_id(id) else {
            return (options.tab_width, options.expand_tab);
        };
        let unit = document
            .chosen_indent()
            .or_else(|| document.detected_indent().filter(|_| options.detect_indent));
        match unit {
            Some(crate::core::indent::IndentUnit::Spaces(n)) => (n.max(1), true),
            Some(crate::core::indent::IndentUnit::Tabs) => (options.tab_width, false),
            None => (options.tab_width, options.expand_tab),
        }
    }

    /// Whether buffer `id`'s language has a language server set up (one
    /// may be about to open it).
    pub fn has_language_server(&self, id: DocumentId) -> bool {
        self.options.lsp
            && self
                .document_by_id(id)
                .and_then(|d| self.language_of(d))
                .is_some_and(|language| self.languages.server(language).is_some())
    }

    /// When buffer `id`'s diagnostics were last set.
    pub fn diagnostics_set_at(&self, id: DocumentId) -> Option<std::time::Instant> {
        self.diagnostics_versions.get(&id).map(|v| v.1)
    }

    /// The language servers' state, from the app: the ready server each
    /// buffer's open in, and what it's busy with.
    pub fn set_lsp_checks(&mut self, checks: HashMap<DocumentId, (String, Option<String>)>) {
        self.lsp_checks = checks;
    }

    /// The ready language server buffer `id` is open in (it reports its
    /// problems), and what it's busy with, if anything.
    pub fn lsp_check(&self, id: DocumentId) -> Option<(&str, Option<&str>)> {
        self.lsp_checks
            .get(&id)
            .map(|(name, busy)| (name.as_str(), busy.as_deref()))
    }

    /// Each buffer's text (its edit count) and whether it had unsaved
    /// changes, before the agent changes any.
    pub fn agent_unsaved(&self) -> HashMap<DocumentId, (u64, bool)> {
        std::iter::once(&self.document)
            .chain(&self.background)
            .map(|d| (d.id(), (d.buffer().edit_seq(), d.is_modified())))
            .collect()
    }

    /// An agent change: makes `change`, then saves the files it changed
    /// that had no unsaved changes before, leaving the user's unsaved work
    /// as it is. `opened`: buffers already opened for it (to hold a
    /// server's edits), which are the agent's, as those it opens are.
    /// Returns what `change` did, the note on what was saved, and the
    /// documents changed.
    pub fn agent_change<R>(
        &mut self,
        opened: &[DocumentId],
        change: impl FnOnce(&mut Self) -> R,
    ) -> (R, String, Vec<DocumentId>) {
        let mut before = self.agent_unsaved();
        for id in opened {
            before.remove(id);
        }
        let done = change(self);
        let (saved, changed) = self.agent_save_changed(&before);
        (done, saved, changed)
    }

    /// After the agent's changes: the files they changed that had no
    /// unsaved changes before are saved (so what the agent runs sees
    /// them); the others are left to the user. Says which, for the agent.
    pub fn agent_save_changed(
        &mut self,
        before: &HashMap<DocumentId, (u64, bool)>,
    ) -> (String, Vec<DocumentId>) {
        // Buffers its edits opened (a rename's, a move's other files) are
        // the agent's, to close when its turn ends.
        let opened: Vec<DocumentId> = std::iter::once(&self.document)
            .chain(&self.background)
            .map(|d| d.id())
            .filter(|id| !before.contains_key(id))
            .collect();
        self.agent.opened.extend(opened);
        // Those the agent changed (opened for it, or edited since).
        let changed: Vec<(DocumentId, bool)> = std::iter::once(&self.document)
            .chain(&self.background)
            .filter(|d| d.is_modified() && d.path().is_some())
            .filter_map(|d| match before.get(&d.id()) {
                Some(&(seq, _)) if seq == d.buffer().edit_seq() => None,
                Some(&(_, unsaved)) => Some((d.id(), unsaved)),
                None => Some((d.id(), false)),
            })
            .collect();
        let ids: Vec<DocumentId> = changed.iter().map(|&(id, _)| id).collect();
        let ids_count = ids.len();
        let mut left = Vec::new();
        let mut failed = Vec::new();
        for (id, unsaved) in changed {
            let Some(document) = self.document_by_id_mut(id) else {
                continue;
            };
            let name = document.path().map(shown).unwrap_or_default();
            if unsaved {
                left.push(name);
            } else if let Err(error) = document.save(None, false) {
                failed.push(format!("{name} ({error})"));
            }
        }
        let mut note = String::new();
        if left.len() == ids_count && ids_count > 0 {
            note.push_str(". Not saved, as the user has unsaved changes there too");
        } else if !left.is_empty() {
            note.push_str(&format!(
                ". Not saved, as the user has unsaved changes there too: {}",
                left.join(", ")
            ));
        }
        if !failed.is_empty() {
            note.push_str(&format!(". Couldn't save: {}", failed.join(", ")));
        }
        (note, ids)
    }

    /// Buffers' files, named for the agent (`src/a.rs, src/b.rs`).
    pub fn agent_names(&self, ids: &[DocumentId]) -> String {
        let names: Vec<String> = ids
            .iter()
            .filter_map(|&id| self.document_by_id(id)?.path().map(shown))
            .collect();
        names.join(", ")
    }

    /// The `lsp_apply_code_action` tool: action `number` (from 1) of the
    /// list `list` that `lsp_code_actions` gave, for `path`.
    pub fn agent_apply_code_action(
        &mut self,
        call: u64,
        path: &str,
        list: u64,
        number: usize,
    ) -> Result<(), String> {
        if !self.options.lsp {
            return Err("Language servers are off in this editor (lsp = false)".into());
        }
        let full = absolute(Path::new(path));
        self.agent_buffer(&full)?;
        let kept = self
            .agent
            .code_actions
            .iter()
            .find(|l| l.number == list)
            .cloned();
        let index = number.saturating_sub(1);
        if let Err(why) = self.make_code_action(kept.as_ref(), index, Reply::Agent(call)) {
            // Answered as the server's answer would be.
            self.agent_lsp_answer(call, Err(why.into()));
        }
        Ok(())
    }

    /// Keeps a list of code actions given to the agent, to make one: the
    /// last few.
    pub(super) fn keep_agent_code_actions(&mut self, list: super::lsp::ActionList) {
        const KEPT: usize = 4;
        let lists = &mut self.agent.code_actions;
        if lists.len() == KEPT {
            lists.pop_front();
        }
        lists.push_back(list);
    }

    /// A language server question for tool call `call`: `feature` at
    /// `path`'s `line` (from 1), at `symbol` on it (or `column`, from 1, or
    /// the line's first word); `query` for the project's symbols. The answer
    /// comes back through [`Editor::agent_lsp_answer`].
    pub fn agent_ask_lsp(
        &mut self,
        call: u64,
        feature: LspFeature,
        path: Option<&str>,
        place: (Option<usize>, Option<&str>, Option<usize>, Option<usize>),
        query: Option<&str>,
    ) -> Result<(), String> {
        if !self.options.lsp {
            return Err("Language servers are off in this editor (lsp = false)".into());
        }
        let id = match path {
            Some(path) => {
                let path = absolute(Path::new(path));
                if !path.is_file() {
                    return Err(format!("{} isn't a file", path.display()));
                }
                self.agent_buffer(&path)?
            }
            // The project's symbols: asked of the server for any open file.
            None => std::iter::once(&self.document)
                .chain(&self.background)
                .find(|d| d.path().is_some())
                .map(|d| d.id())
                .ok_or("Open a file first: the language server is found by its language")?,
        };
        let document = self.document_by_id(id).ok_or("No such buffer")?;
        let buffer = document.buffer();
        let (line, symbol, column, occurrence) = place;
        // Code actions with no line: the file-wide ones, for all of it.
        if feature == LspFeature::CodeAction && line.is_none() {
            let last = buffer.line_count().saturating_sub(1);
            self.lsp_requests.push(LspRequest {
                query: LspQuery::CodeActions {
                    start: Position::new(0, 0),
                    end: buffer.line_end(last),
                    file_wide: true,
                    with_line: false,
                },
                document: id,
                position: Position::new(0, 0),
                buffer_id: buffer.id(),
                edit_seq: buffer.edit_seq(),
                reply: Reply::Agent(call),
            });
            return Ok(());
        }
        let row = line.unwrap_or(1).saturating_sub(1);
        if row >= buffer.line_count() {
            return Err(format!(
                "Line {} is past the end ({} lines)",
                row + 1,
                buffer.line_count()
            ));
        }
        let text = buffer.line(row).to_string();
        let col = match (symbol, column) {
            (Some(symbol), _) => {
                let nth = occurrence.unwrap_or(1).max(1);
                let byte = find_word(&text, symbol, nth).ok_or_else(|| {
                    let times = text.matches(symbol).count();
                    if times == 0 {
                        format!("\"{symbol}\" isn't on line {}", row + 1)
                    } else if times == 1 {
                        format!("\"{symbol}\" is on line {} only once", row + 1)
                    } else {
                        format!("\"{symbol}\" is on line {} only {times} times", row + 1)
                    }
                })?;
                text[..byte].chars().count()
            }
            (None, Some(column)) => {
                let width = text.trim_end_matches(['\n', '\r']).chars().count();
                if column == 0 || column > width + 1 {
                    return Err(format!(
                        "Column {column} is past the end of line {} ({width} columns)",
                        row + 1
                    ));
                }
                column - 1
            }
            (None, None) => text.chars().take_while(|c| c.is_whitespace()).count(),
        };
        let position = Position::new(row, col);
        let text = |q: Option<&str>| q.unwrap_or_default().to_string();
        let lsp_query = match feature {
            LspFeature::Rename => LspQuery::Rename { name: text(query) },
            LspFeature::WorkspaceSymbols => LspQuery::WorkspaceSymbols { query: text(query) },
            // At the symbol (its own refactors), and for the whole line.
            LspFeature::CodeAction => LspQuery::CodeActions {
                start: position,
                end: position,
                file_wide: false,
                with_line: true,
            },
            feature => LspQuery::at(feature)
                .ok_or_else(|| format!("Language servers can't be asked for {}", feature.noun()))?,
        };
        let request = LspRequest {
            query: lsp_query,
            document: id,
            position,
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            reply: Reply::Agent(call),
        };
        self.lsp_requests.push(request);
        Ok(())
    }

    /// A language server's answer to the agent's tool call `call`.
    pub fn agent_lsp_answer(&mut self, call: u64, answer: Result<String, String>) {
        self.agent_lsp_answers.push(AgentAnswer {
            call,
            answer,
            changed: Vec::new(),
        });
    }

    /// The same, for an answer that changed buffers `changed`.
    pub fn agent_lsp_changed(&mut self, call: u64, text: String, changed: Vec<DocumentId>) {
        self.agent_lsp_answers.push(AgentAnswer {
            call,
            answer: Ok(text),
            changed,
        });
    }

    /// The answers that came, for the app to send the agent.
    pub fn take_agent_lsp_answers(&mut self) -> Vec<AgentAnswer> {
        std::mem::take(&mut self.agent_lsp_answers)
    }
}

/// Lines (both sides together) below which a file's diff is always given,
/// even with nothing in common.
const SHORT_FILE: usize = 40;

/// A unified diff of `old` to `new` (three lines of context, headed by
/// their names), cut at `DIFF_LINES`. When they've no line in common, just
/// that, and how long each is: the lines themselves say nothing more.
fn file_diff(old: &str, new: &str, old_name: &str, new_name: &str, asked: bool) -> String {
    let diff = similar::TextDiff::from_lines(old, new);
    let common = diff
        .iter_all_changes()
        .any(|c| c.tag() == similar::ChangeTag::Equal);
    let (old_lines, new_lines) = (old.lines().count(), new.lines().count());
    // Emptied (a saved empty buffer is still a line ending).
    if old_lines > 0 && new.trim().is_empty() {
        return format!("{new_name}: empty, all {old_lines} lines of {old_name} deleted");
    }
    // A short file's diff is short anyway: given as it is.
    if !common && old_lines > 0 && new_lines > 0 && old_lines + new_lines > SHORT_FILE {
        return format!(
            "{new_name}: the whole file differs from {old_name} ({old_lines} lines there, \
             {new_lines} in the editor; read_buffer gives the editor's)"
        );
    }
    let text = diff
        .unified_diff()
        .context_radius(3)
        .header(old_name, new_name)
        .to_string();
    let lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.len() <= DIFF_LINES {
        return lines.join("\n");
    }
    let more = lines.len() - DIFF_LINES;
    let hint = if asked {
        "too long to give whole: read_buffer gives the file as it is now"
    } else {
        "ask for this file with path, or read_buffer for it as it is now"
    };
    format!(
        "{}\n… {more} more lines of this file's diff ({hint})",
        lines[..DIFF_LINES].join("\n")
    )
}

/// The longest group of lines a progress display redraws (`❯ file 3/88`
/// lines, one per worker, and the totals under them) that's looked for.
const REDRAW_LINES: usize = 32;
/// How far on an old status line's newer copy (or its file's ✓ line) is
/// looked for, past the output printed in between.
const NEAR_LINES: usize = 64;

/// A terminal's lines without what progress displays left behind: a
/// program that redraws its status in place (Vitest, cargo, npm...) leaves
/// old copies in the scrollback when its status is taller than the screen.
/// Status lines are ones with a counter (`3/88`, `40%`) or a status mark
/// (`❯`, a spinner) in front. Left out: a group of lines with a status line
/// in it that comes again straight after itself (its counters and `[queued]`
/// aside); then a status line that comes again close by, with other output
/// between, or whose file's ✓ (or ×) line follows (or came just before,
/// if its counter's full: `❯ file 5/5`); and the blank lines beside those. Returns the lines and how many redraws went.
fn without_redraws(lines: Vec<String>) -> (Vec<String>, usize) {
    let mut kept: Vec<(String, String)> = Vec::with_capacity(lines.len());
    let mut dropped = 0;
    for line in lines {
        kept.push((redraw_key(&line), line));
        'again: loop {
            let n = kept.len();
            for size in 1..=REDRAW_LINES.min(n / 2) {
                let (before, after) = kept[n - 2 * size..].split_at(size);
                if before.iter().zip(after).all(|(a, b)| a.0 == b.0)
                    && after.iter().any(|(_, line)| is_status(line))
                {
                    kept.drain(n - 2 * size..n - size);
                    dropped += 1;
                    continue 'again;
                }
            }
            break;
        }
    }

    // Old status lines with output in between.
    let n = kept.len();
    let mut stale = vec![false; n];
    for i in 0..n {
        let line = &kept[i].1;
        if !is_status(line) {
            continue;
        }
        let subject = status_subject(line);
        let later = &kept[i + 1..(i + 1 + NEAR_LINES).min(n)];
        let earlier = &kept[i.saturating_sub(NEAR_LINES)..i];
        let finished =
            |(_, other): &(String, String)| subject.as_ref().is_some_and(|s| finishes(other, s));
        if later
            .iter()
            .any(|item| item.0 == kept[i].0 || finished(item))
            || counter_full(line) && earlier.iter().any(finished)
        {
            stale[i] = true;
            dropped += 1;
        }
    }
    let beside_stale =
        |i: usize| (i > 0 && stale[i - 1]) || stale.get(i + 1).copied().unwrap_or(false);
    let lines = kept
        .into_iter()
        .enumerate()
        .filter(|(i, (_, line))| !(stale[*i] || line.trim().is_empty() && beside_stale(*i)))
        .map(|(_, (_, line))| line)
        .collect();
    (lines, dropped)
}

/// Whether `line` is a status line: a counter in it, or a status mark (`❯`,
/// `›`, a braille spinner) in front.
fn is_status(line: &str) -> bool {
    has_counter(line)
        || line
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| matches!(c, '❯' | '›' | '\u{2801}'..='\u{28FF}'))
}

/// What a `❯ file 3/5` status line is about: its words after the mark,
/// without a last counter or `[queued]`.
fn status_subject(line: &str) -> Option<Vec<&str>> {
    let mut words: Vec<&str> = line.split_whitespace().collect();
    let mark = words.first()?.chars().next()?;
    if !matches!(mark, '❯' | '›' | '\u{2801}'..='\u{28FF}') {
        return None;
    }
    words.remove(0);
    if words
        .last()
        .is_some_and(|w| has_counter(w) || (w.starts_with('[') && w.ends_with(']')))
    {
        words.pop();
    }
    (!words.is_empty()).then_some(words)
}

/// Whether `line` says `subject` is done: `✓` (or `×`) and the same words.
fn finishes(line: &str, subject: &[&str]) -> bool {
    let mut words = line.split_whitespace();
    let done = words
        .next()
        .and_then(|w| w.chars().next())
        .is_some_and(|c| matches!(c, '✓' | '✔' | '×' | '✗' | '✘'));
    done && subject.iter().all(|s| words.next() == Some(*s))
}

/// What a line's compared by, for [`without_redraws`]: the line with its
/// counters' numbers (`3/88`, `40%`) left out, and a status line's
/// `[queued]` at the end. Other numbers stay, so `w1 3/88` and `w2 3/88`
/// are different lines.
fn redraw_key(line: &str) -> String {
    let line = match line.trim_end().rsplit_once(' ') {
        Some((before, last)) if last.starts_with('[') && last.ends_with(']') && is_status(line) => {
            before
        }
        _ => line,
    };
    let chars: Vec<char> = line.chars().collect();
    let mut key = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            key.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let next = chars.get(i).copied();
        let after = chars.get(i + 1).is_some_and(char::is_ascii_digit);
        let before = start >= 2 && chars[start - 1] == '/' && chars[start - 2].is_ascii_digit();
        let counter = next == Some('%') || (next == Some('/') && after) || before;
        if counter {
            key.push('#');
        } else {
            key.extend(&chars[start..i]);
        }
    }
    key
}

/// Whether `line` ends with a full counter (`5/5`, `100%`): what it counts
/// is done.
fn counter_full(line: &str) -> bool {
    let Some(last) = line.split_whitespace().last() else {
        return false;
    };
    if let Some((done, total)) = last.split_once('/') {
        let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        return number(done) && done == total;
    }
    last == "100%"
}

/// Whether there's a counter in `line`: `3/88` or `40%`.
fn has_counter(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        let digit_before = i > 0 && bytes[i - 1].is_ascii_digit();
        match b {
            b'%' => digit_before,
            b'/' => digit_before && bytes.get(i + 1).is_some_and(u8::is_ascii_digit),
            _ => false,
        }
    })
}

/// The replacement of `text[from..to]` by `new`, without the text they
/// share at the start and the end.
fn trimmed(text: &str, from: usize, to: usize, new: &str) -> (usize, usize, String) {
    let old = &text[from..to];
    let start: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let (old_rest, new_rest) = (&old[start..], &new[start..]);
    let end: usize = old_rest
        .chars()
        .rev()
        .zip(new_rest.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    (
        from + start,
        to - end,
        new_rest[..new_rest.len() - end].to_string(),
    )
}

/// Where the `nth` (from 1) `word` is in `line` as a whole word (or, if
/// it's nowhere whole, as part of one): its byte offset.
fn find_word(line: &str, word: &str, nth: usize) -> Option<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let whole: Vec<usize> = line
        .match_indices(word)
        .filter(|&(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + word.len()..].chars().next();
            !before.is_some_and(is_word) && !after.is_some_and(is_word)
        })
        .map(|(at, _)| at)
        .collect();
    if whole.is_empty() {
        return line.match_indices(word).nth(nth - 1).map(|(at, _)| at);
    }
    whole.get(nth - 1).copied()
}

/// A path made absolute (from the current folder), as servers give them.
fn absolute(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    if let Ok(real) = std::fs::canonicalize(&path) {
        return real;
    }
    // Not there yet (a file to create, a move's target): its nearest folder
    // that is, as the file system resolves it (`/tmp` is `/private/tmp` on
    // macOS), and the rest as given, so it's written as files that exist are.
    let mut rest = Vec::new();
    let mut at = path.as_path();
    while let Some(parent) = at.parent() {
        rest.push(
            at.file_name()
                .map(std::ffi::OsStr::to_os_string)
                .unwrap_or_default(),
        );
        if let Ok(real) = std::fs::canonicalize(parent) {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        at = parent;
    }
    path
}

/// A path the agent gave, as it's told back: relative to the project if
/// it's in it, else as given (not as the file system resolves it).
fn named(given: &str) -> String {
    let full = absolute(Path::new(given));
    let relative = shown(&full);
    if Path::new(&relative).is_absolute() {
        given.to_string()
    } else {
        relative
    }
}

/// A path as the agent's told it: relative to the project, if it's in it.
fn shown(path: &Path) -> String {
    super::agent::short_path(&absolute(path).to_string_lossy())
}

/// Whether a diagnostic comes from a check that runs on save (cargo
/// check's rustc and clippy), so it's about the saved file.
fn on_save(d: &super::diagnostics::Diagnostic) -> bool {
    matches!(d.source.as_deref(), Some("rustc" | "clippy" | "cargo"))
}

/// A diagnostic, its message's first two lines joined (rustc's second
/// says what was expected and found), cut short if long; lines like
/// `#[warn(dead_code)] on by default` left out.
fn full_message(d: &super::diagnostics::Diagnostic) -> String {
    const MOST: usize = 200;
    let lines: Vec<&str> = d
        .message
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.ends_with("on by default"))
        .collect();
    let mut message = lines
        .iter()
        .take(2)
        .copied()
        .collect::<Vec<_>>()
        .join(" / ");
    let cut = lines.len() > 2 || message.chars().count() > MOST;
    if let Some((at, _)) = message.char_indices().nth(MOST) {
        message.truncate(at);
    }
    if cut {
        message.push_str(" …");
    }
    match &d.source {
        Some(source) => format!("{}: {message} ({source})", d.severity.id()),
        None => format!("{}: {message}", d.severity.id()),
    }
}

fn clipped(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!("{}\n… (cut short)", &text[..at]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn edits_that_touch_land_where_they_were_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "alpha\n2. x\ngamma\n").unwrap();
        let mut e = Editor::default();
        let edits = [
            ("2. x".to_string(), "1. x".to_string(), false),
            (
                "alpha\n".to_string(),
                "alpha\n   (note)\n".to_string(),
                false,
            ),
        ];
        let (said, _) = e.agent_edit_buffer(path.to_str().unwrap(), &edits).unwrap();
        assert!(said.contains("2 replacements"), "{said}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "alpha\n   (note)\n1. x\ngamma\n"
        );
    }

    #[test]
    fn language_servers_say_how_they_are_and_what_they_logged() {
        let mut e = Editor::default();
        e.options.lsp = false;
        assert!(e.agent_language_servers().contains("lsp = false"));
        e.options.lsp = true;
        assert!(
            e.agent_language_servers()
                .starts_with("No language servers")
        );

        e.set_lsp_status(vec![
            "rust-analyzer (rust): running (indexing 3/9) in /p, 2 files open".into(),
            "pyright-langserver: not found on PATH".into(),
            "PATH=/usr/bin:/bin".into(),
        ]);
        let output = |e: &mut Editor, text: &str, error: bool| {
            e.panel.output.push(super::super::panel::OutputLine {
                time: "09:30:05".into(),
                error,
                text: text.into(),
            });
        };
        for n in 0..45 {
            output(&mut e, &format!("rust-analyzer: line {n}"), false);
        }
        // One entry of 8 lines: counted as 8.
        output(
            &mut e,
            "rust-analyzer: cargo check failed\nerror: duplicate key\n a\n b\n c\n d\n e\n f",
            true,
        );
        let said = e.agent_language_servers();
        assert!(
            said.contains("- rust-analyzer (rust): running (indexing 3/9) in /p, 2 files open"),
            "{said}"
        );
        assert!(
            said.contains("- pyright-langserver: not found on PATH"),
            "{said}"
        );
        assert!(
            said.contains("\nThe PATH ferra started with: /usr/bin:/bin"),
            "{said}"
        );
        assert!(!said.contains("Busy:"), "the state says it: {said}");
        assert!(said.contains("last 50 of 53 lines"), "{said}");
        assert!(
            !said.contains("line 2\n") && said.contains("line 3\n"),
            "{said}"
        );
        assert!(
            said.contains(
                "09:30:05 error: rust-analyzer: cargo check failed\n         error: duplicate key"
            ),
            "{said}"
        );
        assert!(said.contains("         f\n```"), "{said}");

        // An entry too long by itself: its last lines.
        let mut e = Editor::default();
        let long: Vec<String> = (0..70).map(|n| format!("l{n}")).collect();
        output(&mut e, &long.join("\n"), false);
        let said = e.agent_language_servers();
        assert!(said.contains("last 50 of 70 lines"), "{said}");
        assert!(
            said.contains("\n         l20\n") && said.contains("l69\n```"),
            "{said}"
        );
        assert!(
            said.find("l20").unwrap() < said.find("l69").unwrap(),
            "in order: {said}"
        );
    }

    #[test]
    fn progress_redraws_are_left_out_of_the_terminals_text() {
        // Vitest's leftovers: one status line and a blank, again and again.
        let (kept, dropped) = without_redraws(lines(&[
            "RUN v3",
            " ❯ a.test.ts 0/88",
            "",
            " ❯ a.test.ts 0/88",
            "",
            " ❯ a.test.ts 17/88",
            "",
            " ✓ a.test.ts (88 tests)",
        ]));
        // The last copy goes too, once the file's ✓ line is there.
        assert_eq!(kept, lines(&["RUN v3", " ✓ a.test.ts (88 tests)"]));
        assert_eq!(dropped, 3);

        // A taller status (a line per worker) goes a group at a time.
        let (kept, dropped) = without_redraws(lines(&[
            "w1 a.ts 1/9",
            "w2 b.ts 3/5",
            "w1 a.ts 2/9",
            "w2 b.ts 4/5",
            "w1 a.ts 9/9",
            "w2 b.ts 5/5",
        ]));
        assert_eq!(kept, lines(&["w1 a.ts 9/9", "w2 b.ts 5/5"]));
        assert_eq!(dropped, 2);

        // Repeats without a counter are real output: kept.
        let same = lines(&["warning: x", "warning: x", "line1", "line2", "line3"]);
        assert_eq!(without_redraws(same.clone()), (same, 0));
        // Neither are paths or ordinary numbers counters.
        assert!(!has_counter("server/resources/a1.ts line 12"));
        assert!(has_counter("Tests 40%") && has_counter("3/88"));
        assert_eq!(redraw_key("w12 3/88 40% 7s"), "w12 #/# #% 7s");
    }

    #[test]
    fn queued_status_lines_go_like_counted_ones() {
        let q = " ❯  client  components/Editor.test.tsx [queued]";
        let (kept, dropped) = without_redraws(lines(&["RUN", q, "", q, "", q, "", " ✓ x"]));
        assert_eq!(kept, lines(&["RUN", q, "", " ✓ x"]));
        assert_eq!(dropped, 2);
        // [running] after [queued] is the same line, redrawn.
        let running = " ❯  client  components/Editor.test.tsx [running]";
        let (kept, _) = without_redraws(lines(&[q, running]));
        assert_eq!(kept, lines(&[running]));
    }

    #[test]
    fn old_status_lines_between_output_go() {
        let (kept, dropped) = without_redraws(lines(&[
            " ❯  client  components/AssessmentEditor.test.tsx 2/5",
            " ✓  client  pages/Page.test.tsx (35 tests) 879ms",
            "",
            " ❯  client  components/AssessmentEditor.test.tsx 2/5",
            " ✓  client  layouts/AdminLayout.test.tsx (31 tests) 1123ms",
            "",
            " ❯  client  components/AssessmentEditor.test.tsx 3/5",
            " ✓  client  lib/bundle.test.ts (230 tests) 395ms",
            " ✓  client  components/AssessmentEditor.test.tsx (5 tests) 481ms",
            "",
            " Test Files  4 passed (4)",
        ]));
        assert_eq!(
            kept,
            lines(&[
                " ✓  client  pages/Page.test.tsx (35 tests) 879ms",
                " ✓  client  layouts/AdminLayout.test.tsx (31 tests) 1123ms",
                " ✓  client  lib/bundle.test.ts (230 tests) 395ms",
                " ✓  client  components/AssessmentEditor.test.tsx (5 tests) 481ms",
                "",
                " Test Files  4 passed (4)",
            ])
        );
        assert_eq!(dropped, 3);

        // Two files at 3/5 are two lines; a status still showing stays.
        let (kept, dropped) = without_redraws(lines(&[
            " ❯ a.test.ts 3/5",
            " ❯ b.test.ts 3/5",
            " ✓ c.test.ts (2 tests)",
        ]));
        assert_eq!(kept.len(), 3, "{kept:?}");
        assert_eq!(dropped, 0);
        // And real output that repeats with things between stays.
        let real = lines(&["warning: x", "compiling y", "warning: x", "done"]);
        assert_eq!(without_redraws(real.clone()), (real, 0));
    }

    #[test]
    fn full_status_lines_after_their_file_is_done_go() {
        // The last redraw can land after the file's ✓ line (seen in Vitest).
        let (kept, dropped) = without_redraws(lines(&[
            " ✓  client  lib/search.test.ts (2 tests) 2ms",
            " ✓  client  lib/format.test.ts (2 tests) 1ms",
            "",
            " ❯  client  lib/search.test.ts 2/2",
            " ✓  client  components/Form.test.tsx (32 tests) 4035ms",
        ]));
        assert_eq!(
            kept,
            lines(&[
                " ✓  client  lib/search.test.ts (2 tests) 2ms",
                " ✓  client  lib/format.test.ts (2 tests) 1ms",
                " ✓  client  components/Form.test.tsx (32 tests) 4035ms",
            ])
        );
        assert_eq!(dropped, 1);

        // A file running again after it was done (watch mode) still shows.
        let rerun = lines(&[" ✓ a.test.ts (5 tests)", "", " ❯ a.test.ts 2/5"]);
        assert_eq!(without_redraws(rerun.clone()), (rerun, 0));
    }

    #[test]
    fn words_are_found_whole() {
        assert_eq!(find_word("let area = area_of(c);", "area", 1), Some(4));
        assert_eq!(find_word("a + a", "a", 2), Some(4));
        assert_eq!(find_word("a + a", "a", 3), None);
        assert_eq!(find_word("fn area_of()", "area", 1), Some(3));
        assert_eq!(find_word("x", "y", 1), None);
    }

    #[test]
    fn diagnostics_are_told_in_two_lines_at_most() {
        use super::super::diagnostics::{Diagnostic, Severity};
        let d = |message: &str| Diagnostic {
            start: Position::new(0, 0),
            end: Position::new(0, 1),
            severity: Severity::Error,
            message: message.into(),
            source: Some("rustc".into()),
        };
        assert_eq!(
            full_message(&d("mismatched types\nexpected `String`, found `u32`")),
            "error: mismatched types / expected `String`, found `u32` (rustc)"
        );
        assert_eq!(
            full_message(&d(
                "cannot multiply\nthe trait is not implemented\n`&u32` implements Mul"
            )),
            "error: cannot multiply / the trait is not implemented … (rustc)"
        );
        assert_eq!(
            full_message(&d(
                "never used\n#[warn(dead_code)] (part of #[warn(unused)]) on by default"
            )),
            "error: never used (rustc)"
        );
    }

    #[test]
    fn only_what_changes_is_replaced() {
        let text = " */\nfn area";
        assert_eq!(
            trimmed(text, 0, text.len(), " */\n// probe\nfn area"),
            (4, 4, "// probe\n".into())
        );
        assert_eq!(trimmed("w * h", 0, 5, "w * hh"), (5, 5, "h".into()));
        assert_eq!(trimmed("é1é", 0, 5, "é2é"), (2, 3, "2".into()));
    }

    #[test]
    fn long_selections_are_cut_short() {
        assert_eq!(clipped("abcdef", 3), "abc\n… (cut short)");
        assert_eq!(clipped("abc", 3), "abc");
    }
}
