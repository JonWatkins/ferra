//! The hover the pointer shows, as VS Code's: rested on a word for a
//! moment (`mouse-hover-delay`), what the language server says about it,
//! under any problems there. The app tells the editor where the pointer
//! rested (`app/mouse.rs`); the popup is the `K` hover's, drawn by the
//! pointer, and goes when the pointer leaves the word (and the popup), on
//! a key or a click. With Ctrl (or ⌥) held, it's the definition's lines
//! instead, as VS Code's peek.

use super::diagnostics::Severity;
use super::lsp::{Location, LspFeature, LspRequest};
use super::{DocumentId, Editor};
use crate::core::Position;

/// The word the pointer rested on: its line and chars `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Word {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

impl Word {
    /// The word around `at` in `line` (a run of letters, digits and `_`),
    /// or the char itself if it's other punctuation; none on a space.
    pub fn at(line: &str, at: Position) -> Option<Word> {
        let chars: Vec<char> = line.chars().collect();
        let c = *chars.get(at.col)?;
        if c.is_whitespace() {
            return None;
        }
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(c) {
            return Some(Word {
                row: at.row,
                start: at.col,
                end: at.col + 1,
            });
        }
        let start = (0..at.col)
            .rev()
            .take_while(|&i| is_word(chars[i]))
            .last()
            .unwrap_or(at.col);
        let end = (at.col..chars.len())
            .take_while(|&i| is_word(chars[i]))
            .last()
            .map_or(at.col + 1, |i| i + 1);
        Some(Word {
            row: at.row,
            start,
            end,
        })
    }
}

/// Where the pointer rested, while its hover is asked for or shown.
#[derive(Debug, Clone)]
pub struct MouseHoverAt {
    document: DocumentId,
    edit_seq: u64,
    at: Position,
    word: Word,
    /// The screen cell, for the popup.
    anchor: (u16, u16),
    /// The problems there, as the hover's Markdown.
    problems: String,
    /// Ctrl was held: the definition's lines.
    peek: bool,
}

/// The most lines a peek shows.
const PEEK_LINES: usize = 15;

/// The definition starting at `lines[0]`: it and what's indented under it
/// (with its closing line), at most `PEEK_LINES`, the indent they share
/// taken off.
fn definition_lines(lines: &[String]) -> Vec<String> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let Some(first) = lines.first() else {
        return Vec::new();
    };
    let base = indent(first);
    let mut out = vec![first.clone()];
    for line in &lines[1..] {
        if out.len() >= PEEK_LINES {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || indent(line) > base {
            out.push(line.clone());
            continue;
        }
        // Its closing line (`}`, `end`...), then it's done.
        if trimmed.starts_with(['}', ')', ']']) || trimmed.starts_with("end") {
            out.push(line.clone());
        }
        break;
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    let shared = out
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    out.iter()
        .map(|l| l.get(shared..).unwrap_or("").to_string())
        .collect()
}

impl Editor {
    /// The pointer rested on `at` (in `word`) in the buffer shown, at
    /// screen cell `anchor`: shows the problems there at once, and asks the
    /// language server about it (its answer joins them).
    pub fn mouse_hover_at(&mut self, at: Position, word: Word, anchor: (u16, u16), peek: bool) {
        let problems: Vec<String> = self
            .document
            .diagnostics()
            .iter()
            .filter(|_| !peek)
            .filter(|d| {
                // In its range (an empty range: on its char).
                d.start <= at && (at < d.end || at == d.start)
            })
            .map(|d| {
                let icon = match d.severity {
                    Severity::Error => "⊗",
                    Severity::Warning => "⚠",
                    _ => "ⓘ",
                };
                let source = d
                    .source
                    .as_ref()
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                format!("{icon} {}{source}", d.message.trim())
            })
            .collect();
        let buffer = self.document.buffer();
        let rest = MouseHoverAt {
            document: self.document.id(),
            edit_seq: buffer.edit_seq(),
            at,
            word,
            anchor,
            problems: problems.join("\n\n"),
            peek,
        };
        let request = LspRequest {
            feature: if peek {
                LspFeature::Definition
            } else {
                LspFeature::Hover
            },
            document: self.document.id(),
            position: at,
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            // Not asked for with a key: quiet if there's no answer.
            manual: false,
            trigger: None,
            word_start: at,
            item: None,
            text: None,
            end: at,
            agent: None,
        };
        if !rest.problems.is_empty() {
            let problems = rest.problems.clone();
            self.show_hover_at(&problems, anchor);
        }
        self.mouse_hover = Some(rest);
        if self.options.lsp && self.document.path().is_some() {
            self.lsp_requests.push(request);
        }
    }

    fn show_hover_at(&mut self, markdown: &str, anchor: (u16, u16)) {
        self.show_hover(markdown);
        if let Some(hover) = &mut self.hover {
            hover.anchor = Some(anchor);
        }
    }

    /// Whether a hover the pointer asked for (`request`) is still wanted:
    /// the pointer's still there, and the text's as it was.
    pub fn mouse_hover_wanted(&self, request: &LspRequest) -> bool {
        self.mouse_hover.as_ref().is_some_and(|m| {
            m.document == request.document
                && m.at == request.position
                && m.edit_seq == request.edit_seq
                && self.document.id() == m.document
                && self.document.buffer().edit_seq() == m.edit_seq
        })
    }

    /// The language server's answer for where the pointer rested: shown
    /// under the problems there (or nothing, if there's neither).
    pub fn mouse_hover_answer(&mut self, markdown: &str) {
        let Some(rest) = &self.mouse_hover else {
            return;
        };
        let (problems, anchor) = (rest.problems.clone(), rest.anchor);
        let text = match (problems.is_empty(), markdown.trim().is_empty()) {
            (_, true) => return,
            (true, false) => markdown.trim().to_string(),
            (false, false) => format!("{problems}\n\n---\n\n{}", markdown.trim()),
        };
        self.show_hover_at(&text, anchor);
    }

    /// The definition the pointer peeked at (Ctrl held): its first
    /// lines, in its language, under where it is.
    pub fn mouse_peek_answer(&mut self, locations: &[Location]) {
        let Some(anchor) = self.mouse_hover.as_ref().map(|m| m.anchor) else {
            return;
        };
        let Some(location) = locations.first() else {
            return;
        };
        let row = location.position.row;
        // From the buffer if it's open (unsaved changes and all), else the
        // file.
        let open = self
            .find_open_path(&location.path)
            .and_then(|id| self.document_by_id(id));
        let (lines, language) = match open {
            Some(document) => {
                let buffer = document.buffer();
                let end = buffer.line_count().min(row + PEEK_LINES * 2);
                let lines: Vec<String> =
                    (row..end).map(|r| buffer.line(r).to_string()).collect();
                (lines, self.language_of(document).map(str::to_string))
            }
            None => {
                let Ok(text) = std::fs::read_to_string(&location.path) else {
                    return;
                };
                let first = text.lines().next().unwrap_or_default().to_string();
                let lines: Vec<String> = text
                    .lines()
                    .skip(row)
                    .take(PEEK_LINES * 2)
                    .map(str::to_string)
                    .collect();
                let language = self
                    .languages
                    .detect(Some(&location.path), &first)
                    .map(str::to_string);
                (lines, language)
            }
        };
        let lines = definition_lines(&lines);
        if lines.is_empty() {
            return;
        }
        let here = std::env::current_dir().unwrap_or_default();
        let shown = location
            .path
            .strip_prefix(&here)
            .unwrap_or(&location.path)
            .display();
        let markdown = format!(
            "{shown}:{}\n\n```{}\n{}\n```",
            row + 1,
            language.unwrap_or_default(),
            lines.join("\n")
        );
        self.show_hover_at(&markdown, anchor);
    }

    /// The word the pointer's hover is for, in the buffer shown, and
    /// whether it's a peek.
    pub fn mouse_hover_word(&self) -> Option<(Word, bool)> {
        self.mouse_hover
            .as_ref()
            .filter(|m| m.document == self.document.id())
            .map(|m| (m.word, m.peek))
    }

    /// The pointer left: its hover goes (one `K` showed stays). Returns
    /// whether something went from the screen.
    pub fn end_mouse_hover(&mut self) -> bool {
        self.mouse_hover = None;
        if self.hover.as_ref().is_some_and(|h| h.anchor.is_some()) {
            self.hover = None;
            return true;
        }
        false
    }

    /// A click, or the text scrolled: any hover goes. Returns whether one
    /// was shown.
    pub fn end_any_hover(&mut self) -> bool {
        self.mouse_hover = None;
        self.hover.take().is_some()
    }

    /// The wheel over the hover: scrolls it `delta` rows.
    pub fn scroll_hover_by(&mut self, delta: isize) {
        if let Some(hover) = &mut self.hover {
            let last = hover.total.get().saturating_sub(hover.shown.get());
            hover.scroll = (hover.scroll as isize + delta).clamp(0, last as isize) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(line: &str, col: usize) -> Option<(usize, usize)> {
        Word::at(line, Position::new(0, col)).map(|w| (w.start, w.end))
    }

    #[test]
    fn a_definition_is_its_indented_lines_and_closer() {
        let lines: Vec<String> = [
            "    fn area(&self) -> f64 {",
            "        self.w * self.h",
            "",
            "    }",
            "    fn next() {}",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            definition_lines(&lines),
            ["fn area(&self) -> f64 {", "    self.w * self.h", "", "}"]
        );
        let one: Vec<String> = ["const X: u8 = 1;", "const Y: u8 = 2;"]
            .map(String::from)
            .to_vec();
        assert_eq!(definition_lines(&one), ["const X: u8 = 1;"]);
    }

    #[test]
    fn words_punctuation_and_spaces() {
        assert_eq!(word("let foo_bar = 1;", 6), Some((4, 11)));
        assert_eq!(word("let foo_bar = 1;", 4), Some((4, 11)));
        assert_eq!(word("let foo_bar = 1;", 10), Some((4, 11)));
        assert_eq!(word("let foo_bar = 1;", 12), Some((12, 13)));
        assert_eq!(word("let foo_bar = 1;", 3), None);
        assert_eq!(word("let", 9), None);
    }
}
