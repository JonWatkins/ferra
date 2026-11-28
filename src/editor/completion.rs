//! Insert-mode completion, as in Vim: words (`<C-n>`, `<C-p>`), file
//! names (`<C-x><C-f>`) and whole lines (`<C-x><C-l>`).
//!
//! The word before the cursor is completed from words in the buffer
//! (nearest first, looking forward for `<C-n>` and backward for `<C-p>`,
//! wrapping), then in the other open buffers, those starting with it ahead
//! of fuzzy matches. The first match goes in at
//! once; more presses go through the rest and back round to what was
//! typed. `<C-y>` keeps the choice and `<C-e>` puts back what was typed;
//! any other key keeps the choice and carries on (the app ends completion
//! after a key that didn't use it: [`Editor::end_completion_unless_used`]).
//!
//! `<C-n>`/`<C-p>` in insert mode complete the word before the cursor
//! (`editor/completion.rs`). The first press gathers matches: words starting
//! with what's typed, then starting with it in another case, then fuzzy matches
//! whose first letter starts a word in them (`fm` for `flat_map`, not `0` for
//! `log10`), the tiers a server's completion is narrowed by (`match_tier`).
//! Within each, from the cursor outward through the buffer (forward for
//! `<C-n>`, backward for `<C-p>`, wrapping, the cursor's own line split at the
//! cursor), then the other open buffers, without repeats, up to 500. The
//! nearest goes in at once; later presses step through the matches and what was
//! typed, as a ring. Each step replaces the word in the buffer, so the text is
//! always what you see, and the `".` register's text follows.
//!
//! `<C-x><C-f>` and `<C-x><C-l>` start the same kind of completion with other
//! matches (`start_completion` is shared): the entries of the directory the
//! path before the cursor names (relative to the current file's directory, as
//! completion plugins do, rather than Vim's working directory; `~/` and
//! absolute paths too), only the part after the last `/` replaced, directories
//! with a trailing `/`, hidden ones once a `.` is typed; or the lines (trimmed)
//! that start with the current line's text after its indentation, nearest above
//! first, then below, then other buffers. `<C-n>`/`<C-p>` then step through
//! whichever is open.
//!
//! Completion ends with the first key that doesn't use it: `App::process_key`
//! clears a flag before each key and ends a completion no completion command
//! set it for. That covers typing on, moving, `<Esc>` and pastes, and works the
//! same during macros and `.`. `<C-y>` ends it keeping the match; `<C-e>` puts
//! back what was typed first. The menu (`ui/completion_menu.rs`) is drawn at
//! the cursor in the `ui.popup` colours with a border like the hover's, its
//! text lined up with the word, below its row or above when there's no room,
//! showing up to 10 matches (each label with the letters typed in
//! `ui.popup.match`, and its detail dimmed beside it) and scrolling to the
//! selected one, with `n/total` in the border when they don't all fit. A
//! server's chosen item has its documentation beside it
//! (`editor/server_completion.rs`).
//!
//! `.` puts back the text a completion put in, not the keys that chose it
//! (which would complete again, perhaps to something else now). Each step
//! (`show_completion_choice`, and taking a server's item, which can move the
//! cursor into a snippet) leaves a [`Completed`]: how many characters it took
//! out before the cursor, the text it put in, and where in it the cursor
//! went. The app takes it after each key (`take_completed`) and the `.`
//! recorder swaps the keys of that binding for it (`app/repeat.rs`); replaying
//! it calls [`Editor::put_completed`]. A server item's other edits (an import)
//! aren't repeated: they're made once.

use std::collections::HashSet;

use super::Editor;
use super::server_completion::match_tier;
use crate::core::{Buffer, Position};

/// The position `n` characters before `at` (a line break counting as one),
/// or the buffer's start.
fn chars_before(buffer: &Buffer, mut at: Position, n: usize) -> Position {
    for _ in 0..n {
        if at.col > 0 {
            at.col -= 1;
        } else if at.row > 0 {
            at = Position::new(at.row - 1, buffer.line_len(at.row - 1));
        } else {
            break;
        }
    }
    at
}

/// Where `text` ends if it starts at `start`.
fn position_after(start: Position, text: &str) -> Position {
    text.chars().fold(start, |at, c| {
        if c == '\n' {
            Position::new(at.row + 1, 0)
        } else {
            at.with_col(at.col + 1)
        }
    })
}

/// The most matches gathered, so a huge file stays quick.
const MAX_ITEMS: usize = 500;

/// A completion in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordCompletion {
    /// Where the word being completed starts.
    pub start: Position,
    /// What was typed of it.
    pub typed: String,
    /// The matches, nearest first in the direction completion started:
    /// the text each puts in.
    pub items: Vec<String>,
    /// The match in the text, or `None` for what was typed.
    pub selected: Option<usize>,
    /// What the menu shows for each item, if not its text (a server's
    /// label, `area(…)`), and beside it (`fn(&self) -> f64`). Empty, or
    /// one per item.
    pub labels: Vec<String>,
    pub details: Vec<String>,
    /// For a language server's completion: all it offered, which `items`
    /// are filtered from as typing goes on.
    pub server: Option<super::server_completion::ServerCompletion>,
}

/// What a completion step did to the text before the cursor, for `.` to
/// do again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    /// How many characters it took out before the cursor (what was typed,
    /// or the match before).
    pub removed: usize,
    /// What it put in their place.
    pub text: String,
    /// Where the cursor went, in characters into `text`.
    pub cursor: usize,
}

impl WordCompletion {
    /// What the menu shows for item `i`.
    pub fn label(&self, i: usize) -> &str {
        self.labels.get(i).unwrap_or(&self.items[i])
    }

    /// What the menu shows beside item `i`.
    pub fn detail(&self, i: usize) -> &str {
        self.details.get(i).map_or("", String::as_str)
    }
}

pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    /// The completion in progress, if any.
    pub fn completion(&self) -> Option<&WordCompletion> {
        self.completion.as_ref()
    }

    /// `<C-n>` (`forward`) or `<C-p>`: starts completing the word before the
    /// cursor, or moves to the next (or previous) match.
    pub fn complete(&mut self, forward: bool) -> Result<(), String> {
        self.completion_used = true;
        if let Some(completion) = &mut self.completion {
            let count = completion.items.len();
            // The matches and what was typed form a ring.
            let position = completion.selected.map_or(count, |i| i);
            let next = if forward {
                (position + 1) % (count + 1)
            } else {
                (position + count) % (count + 1)
            };
            completion.selected = (next < count).then_some(next);
            self.show_completion_choice();
            self.completion_choice_changed();
            return Ok(());
        }

        let cursor = self.document.cursor();
        let line = self.document.buffer().line(cursor.row);
        let before: Vec<char> = line.chars().take(cursor.col).collect();
        let word_len = before
            .iter()
            .rev()
            .take_while(|c| is_word_char(**c))
            .count();
        let start = cursor.with_col(cursor.col - word_len);
        let typed: String = before[before.len() - word_len..].iter().collect();
        let items = self.gather_words(&typed, cursor, forward);
        let empty = if typed.is_empty() {
            "No words to complete with".to_string()
        } else {
            format!("No completions for \"{typed}\"")
        };
        self.start_completion(start, typed, items, empty)
    }

    /// `<C-x><C-f>`: completes the file name before the cursor from the
    /// directory its path names, relative to the current file's directory
    /// (or the working directory). Directories end in `/`, so completing
    /// can go on into them; hidden files are listed once a `.` is typed.
    pub fn complete_path(&mut self) -> Result<(), String> {
        self.completion_used = true;
        let cursor = self.document.cursor();
        let line = self.document.buffer().line(cursor.row);
        let before: Vec<char> = line.chars().take(cursor.col).collect();
        let in_path = |c: char| c.is_alphanumeric() || "/._-~+@".contains(c);
        let path_len = before.iter().rev().take_while(|c| in_path(**c)).count();
        let path: String = before[before.len() - path_len..].iter().collect();
        // Only the last part (after the last `/`) is completed.
        let (dir, name) = match path.rfind('/') {
            Some(at) => (&path[..=at], &path[at + 1..]),
            None => ("", path.as_str()),
        };
        let start = cursor.with_col(cursor.col - name.chars().count());
        let base = match dir.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(rest)),
            None if dir.starts_with('/') => Some(std::path::PathBuf::from(dir)),
            None => {
                let here = self
                    .document
                    .path()
                    .and_then(|p| p.parent())
                    .map(std::path::Path::to_path_buf)
                    .or_else(|| std::env::current_dir().ok());
                here.map(|h| h.join(dir))
            }
        };
        let mut items: Vec<String> = base
            .and_then(|base| std::fs::read_dir(base).ok())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let file = entry.file_name().to_string_lossy().into_owned();
                let shown =
                    file.starts_with(name) && (name.starts_with('.') || !file.starts_with('.'));
                if !shown {
                    return None;
                }
                let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                Some(if is_dir { format!("{file}/") } else { file })
            })
            .collect();
        items.sort();
        items.truncate(MAX_ITEMS);
        let empty = format!("No files matching \"{path}\"");
        self.start_completion(start, name.to_string(), items, empty)
    }

    /// `<C-x><C-l>`: completes the whole line from the lines that start with
    /// what's typed on it (after its indentation), the nearest above first,
    /// then below, then in the other buffers.
    pub fn complete_line(&mut self) -> Result<(), String> {
        self.completion_used = true;
        let cursor = self.document.cursor();
        let buffer = self.document.buffer();
        let line = buffer.line(cursor.row);
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        let start = cursor.with_col(indent.min(cursor.col));
        let typed: String = line
            .chars()
            .skip(start.col)
            .take(cursor.col - start.col)
            .collect();
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut take = |candidate: &str| {
            let text = candidate.trim_start().trim_end();
            if text.len() > typed.len()
                && text.starts_with(&typed)
                && items.len() < MAX_ITEMS
                && seen.insert(text.to_string())
            {
                items.push(text.to_string());
            }
        };
        let count = buffer.line_count();
        for row in (0..cursor.row).rev().chain(cursor.row + 1..count) {
            take(&buffer.line(row));
        }
        for document in &self.background {
            let buffer = document.buffer();
            for row in 0..buffer.line_count() {
                take(&buffer.line(row));
            }
        }
        let empty = format!("No lines starting \"{typed}\"");
        self.start_completion(start, typed, items, empty)
    }

    /// Starts a completion of `typed` (from `start` to the cursor) with
    /// `items`, putting in the first; `empty` is the error if there are none.
    fn start_completion(
        &mut self,
        start: Position,
        typed: String,
        items: Vec<String>,
        empty: String,
    ) -> Result<(), String> {
        if items.is_empty() {
            return Err(empty);
        }
        self.completion = Some(WordCompletion {
            start,
            typed,
            items,
            selected: Some(0),
            labels: Vec::new(),
            details: Vec::new(),
            server: None,
        });
        self.show_completion_choice();
        Ok(())
    }

    /// `<C-y>`: keeps the match in the text and stops completing. A
    /// server's completion takes the first item if none is chosen, and
    /// makes the edits that go with it (an import).
    pub fn accept_completion(&mut self) {
        self.completion_used = true;
        if self.completion.as_ref().is_some_and(|c| c.server.is_some()) {
            self.accept_server_completion();
        }
        self.completion = None;
    }

    /// `<C-e>`: puts back what was typed and stops completing.
    pub fn cancel_completion(&mut self) {
        self.completion_used = true;
        if let Some(completion) = &mut self.completion {
            completion.selected = None;
            self.show_completion_choice();
        }
        self.completion = None;
    }

    /// The last completion step since this was last called, for `.`.
    pub fn take_completed(&mut self) -> Option<Completed> {
        self.completed.take()
    }

    /// Does a completion step again (`.`): takes out `removed` characters
    /// before the cursor and puts in `text`, the cursor at its place in it.
    pub fn put_completed(&mut self, completed: &Completed) {
        if self.mode != super::Mode::Insert {
            return;
        }
        self.completion = None;
        let cursor = self.document.cursor();
        let start = chars_before(self.document.buffer(), cursor, completed.removed);
        let removed = self.document.buffer().text_range(start, cursor);
        self.document.delete(start, cursor);
        self.document.insert(start, &completed.text);
        let place: String = completed.text.chars().take(completed.cursor).collect();
        self.document.set_cursor(position_after(start, &place));
        self.note_typed_replacing(removed.chars().count(), &completed.text);
    }

    /// Called before each key; see [`Editor::end_completion_unless_used`].
    pub fn note_key_for_completion(&mut self) {
        self.completion_used = false;
        let buffer = self.document.buffer();
        self.text_before_key = (self.document.id(), buffer.id(), buffer.edit_seq());
    }

    /// Called after each key: a key that didn't use the completion (typing
    /// on, moving, `<Esc>`) keeps the choice and ends it. A server's
    /// completion instead follows the word as it's typed, and typing may
    /// start one (with `auto-complete`).
    pub fn end_completion_unless_used(&mut self) {
        let buffer = self.document.buffer();
        let typed = self.text_before_key != (self.document.id(), buffer.id(), buffer.edit_seq());
        self.signature_after_key(typed);
        if self.completion_used {
            return;
        }
        let server = self.completion.as_ref().is_some_and(|c| c.server.is_some());
        if !(server && self.refilter_server_completion()) {
            self.completion = None;
        }
        if typed {
            self.auto_complete();
        }
    }

    /// Puts the selected match (or what was typed) in place of the word.
    pub(super) fn show_completion_choice(&mut self) {
        let Some(completion) = &self.completion else {
            return;
        };
        let text = match completion.selected {
            Some(i) => completion.items[i].clone(),
            None => completion.typed.clone(),
        };
        let start = completion.start;
        let cursor = self.document.cursor();
        let removed = self
            .document
            .buffer()
            .text_range(start, cursor)
            .chars()
            .count();
        self.document.delete(start, cursor);
        let end = self.document.insert(start, &text);
        self.document.set_cursor(end);
        self.note_typed_replacing(removed, &text);
        let cursor = text.chars().count();
        self.completed = Some(Completed {
            removed,
            text,
            cursor,
        });
    }

    /// Notes that the cursor moved within the completion's text (into a
    /// snippet), for `.`.
    pub(super) fn note_completed_cursor(&mut self) {
        let Some(completion) = &self.completion else {
            return;
        };
        let text = match completion.selected {
            Some(i) => completion.items[i].clone(),
            None => completion.typed.clone(),
        };
        let cursor = self
            .document
            .buffer()
            .text_range(completion.start, self.document.cursor())
            .chars()
            .count();
        // Putting the item in, earlier in the same key, took out more.
        let removed = match &self.completed {
            Some(earlier) => earlier.removed,
            None => text.chars().count(),
        };
        self.completed = Some(Completed {
            removed,
            text,
            cursor,
        });
    }

    /// Words matching `typed` (and not it): those starting with it, then
    /// starting with it in another case, then fuzzy matches whose first
    /// letter starts a word in them (`fm` for `flat_map`), as a server's
    /// completion is narrowed. Within each, the current buffer's, nearest
    /// the cursor first in the direction asked, then the other buffers'.
    fn gather_words(&self, typed: &str, cursor: Position, forward: bool) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut tiers: [Vec<String>; 3] = Default::default();
        let mut take = |word: &str| {
            // Each word is matched once, however often it comes.
            if word == typed || tiers[0].len() >= MAX_ITEMS || !seen.insert(word.to_string()) {
                return;
            }
            let Some(tier) = match_tier(typed, word) else {
                return;
            };
            let tier = &mut tiers[usize::from(tier)];
            if tier.len() < MAX_ITEMS {
                tier.push(word.to_string());
            }
        };

        let buffer = self.document.buffer();
        let count = buffer.line_count();
        // Lines from the cursor's, wrapping round; on the cursor's line, the
        // words after it (or before it, going back) come first and the
        // others last.
        let rows: Vec<usize> = if forward {
            (cursor.row..count).chain(0..=cursor.row).collect()
        } else {
            (0..=cursor.row)
                .rev()
                .chain((cursor.row..count).rev())
                .collect()
        };
        for (i, &row) in rows.iter().enumerate() {
            let words = words_of(buffer, row);
            let at_cursor = row == cursor.row;
            let first_visit = i == 0;
            let mut chosen: Vec<&(usize, String)> = words
                .iter()
                .filter(|(col, word)| {
                    let end = col + word.chars().count();
                    if !at_cursor {
                        true
                    } else if first_visit == forward {
                        *col >= cursor.col
                    } else {
                        end < cursor.col
                    }
                })
                .collect();
            if !forward {
                chosen.reverse();
            }
            for (_, word) in chosen {
                take(word);
            }
        }
        for document in &self.background {
            let buffer = document.buffer();
            for row in 0..buffer.line_count() {
                for (_, word) in words_of(buffer, row) {
                    take(&word);
                }
            }
        }
        let mut items: Vec<String> = tiers.into_iter().flatten().collect();
        items.truncate(MAX_ITEMS);
        items
    }
}

/// The words on `row`, with the char column each starts at.
fn words_of(buffer: &Buffer, row: usize) -> Vec<(usize, String)> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    for (col, c) in buffer.line(row).chars().enumerate() {
        if is_word_char(c) {
            if current.is_empty() {
                start = col;
            }
            current.push(c);
        } else if !current.is_empty() {
            words.push((start, std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        words.push((start, current));
    }
    words
}
