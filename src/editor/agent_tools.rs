//! What the coding agent can learn from the editor, and ask of it: what's
//! open and selected (sent with each prompt, and as a tool), the
//! diagnostics, and the language servers' answers (the agent's tool calls
//! become `LspRequest`s whose answers come back here, as text, instead of
//! being shown). The tools themselves are served in `agent/tools.rs`.

use std::path::{Path, PathBuf};

use super::Editor;
use super::lsp::{LspFeature, LspRequest};
use crate::core::{Position, Range};

/// At most this much of a selection goes with a prompt.
const SELECTION_CHARS: usize = 4000;

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
        if let Some((first, last, text)) = self.agent_selection() {
            let text = clipped(&text, SELECTION_CHARS);
            note.push_str(&format!(
                "\nSelected, lines {}-{}:\n```\n{text}\n```",
                first + 1,
                last + 1
            ));
        }
        let others: Vec<String> = self
            .background
            .iter()
            .filter_map(|d| d.path().map(shown))
            .take(20)
            .collect();
        if !others.is_empty() {
            note.push_str(&format!("\nAlso open: {}.", others.join(", ")));
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
                let path = d.path()?;
                Some(format!(
                    "- {}{}",
                    shown(path),
                    if d.is_modified() { " (unsaved)" } else { "" }
                ))
            })
            .collect();
        if !open.is_empty() {
            lines.push(format!("Open files:\n{}", open.join("\n")));
        }
        lines.join("\n")
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
                    d.describe()
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

    /// A language server question for tool call `call`: `feature` at
    /// `path`'s `line` (from 1), at `symbol` on it (or `column`, from 1, or
    /// the line's first word); `query` for the project's symbols. The answer
    /// comes back through [`Editor::agent_lsp_answer`].
    pub fn agent_ask_lsp(
        &mut self,
        call: u64,
        feature: LspFeature,
        path: Option<&str>,
        place: (Option<usize>, Option<&str>, Option<usize>),
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
                self.buffer_for_path(&path)?
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
        let (line, symbol, column) = place;
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
                let byte = find_word(&text, symbol)
                    .ok_or_else(|| format!("\"{symbol}\" isn't on line {}", row + 1))?;
                text[..byte].chars().count()
            }
            (None, Some(column)) => column.saturating_sub(1),
            (None, None) => text.chars().take_while(|c| c.is_whitespace()).count(),
        };
        let position = Position::new(row, col);
        self.lsp_requests.push(LspRequest {
            feature,
            document: id,
            position,
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            manual: true,
            trigger: None,
            word_start: position,
            item: None,
            text: query.map(str::to_string),
            end: position,
            agent: Some(call),
        });
        Ok(())
    }

    /// A language server's answer to the agent's tool call `call`.
    pub fn agent_lsp_answer(&mut self, call: u64, answer: Result<String, String>) {
        self.agent_lsp_answers.push((call, answer));
    }

    /// The answers that came, for the app to send the agent.
    pub fn take_agent_lsp_answers(&mut self) -> Vec<(u64, Result<String, String>)> {
        std::mem::take(&mut self.agent_lsp_answers)
    }
}

/// Where `word` is in `line` as a whole word (or, failing that, anywhere):
/// its byte offset.
fn find_word(line: &str, word: &str) -> Option<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let whole = line.match_indices(word).find(|&(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    });
    whole.map(|(at, _)| at).or_else(|| line.find(word))
}

/// A path made absolute (from the current folder), as servers give them.
fn absolute(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// A path as the agent's told it: relative to the project, if it's in it.
fn shown(path: &Path) -> String {
    super::agent::short_path(&absolute(path).to_string_lossy())
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

    #[test]
    fn words_are_found_whole() {
        assert_eq!(find_word("let area = area_of(c);", "area"), Some(4));
        assert_eq!(find_word("fn area_of()", "area"), Some(3));
        assert_eq!(find_word("x", "y"), None);
    }

    #[test]
    fn long_selections_are_cut_short() {
        assert_eq!(clipped("abcdef", 3), "abc\n… (cut short)");
        assert_eq!(clipped("abc", 3), "abc");
    }
}
