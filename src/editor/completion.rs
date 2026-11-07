//! Insert-mode word completion: `<C-n>` and `<C-p>`, as in Vim.
//!
//! The word before the cursor is completed from words in the buffer
//! (nearest first, looking forward for `<C-n>` and backward for `<C-p>`,
//! wrapping), then in the other open buffers. The first match goes in at
//! once; more presses go through the rest and back round to what was
//! typed. `<C-y>` keeps the choice and `<C-e>` puts back what was typed;
//! any other key keeps the choice and carries on (the app ends completion
//! after a key that didn't use it: [`Editor::end_completion_unless_used`]).

use std::collections::HashSet;

use super::Editor;
use crate::core::{Buffer, Position};

/// The most matches gathered, so a huge file stays quick.
const MAX_ITEMS: usize = 500;

/// A completion in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordCompletion {
    /// Where the word being completed starts.
    pub start: Position,
    /// What was typed of it.
    pub typed: String,
    /// The matches, nearest first in the direction completion started.
    pub items: Vec<String>,
    /// The match in the text, or `None` for what was typed.
    pub selected: Option<usize>,
}

fn is_word_char(c: char) -> bool {
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
        if items.is_empty() {
            return Err(if typed.is_empty() {
                "No words to complete with".into()
            } else {
                format!("No completions for \"{typed}\"")
            });
        }
        self.completion = Some(WordCompletion {
            start,
            typed,
            items,
            selected: Some(0),
        });
        self.show_completion_choice();
        Ok(())
    }

    /// `<C-y>`: keeps the match in the text and stops completing.
    pub fn accept_completion(&mut self) {
        self.completion_used = true;
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

    /// Called before each key; see [`Editor::end_completion_unless_used`].
    pub fn note_key_for_completion(&mut self) {
        self.completion_used = false;
    }

    /// Called after each key: a key that didn't use the completion (typing
    /// on, moving, `<Esc>`) keeps the choice and ends it.
    pub fn end_completion_unless_used(&mut self) {
        if !self.completion_used {
            self.completion = None;
        }
    }

    /// Puts the selected match (or what was typed) in place of the word.
    fn show_completion_choice(&mut self) {
        let Some(completion) = &self.completion else {
            return;
        };
        let text = match completion.selected {
            Some(i) => completion.items[i].clone(),
            None => completion.typed.clone(),
        };
        let start = completion.start;
        let cursor = self.document.cursor();
        self.document.delete(start, cursor);
        let end = self.document.insert(start, &text);
        self.document.set_cursor(end);
        self.note_typed_replacing(cursor.col - start.col, &text);
    }

    /// Words starting with `prefix` (and longer than it): the current
    /// buffer's, nearest the cursor first in the direction asked, then the
    /// other buffers'.
    fn gather_words(&self, prefix: &str, cursor: Position, forward: bool) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut take = |word: &str| {
            if word.len() > prefix.len()
                && word.starts_with(prefix)
                && items.len() < MAX_ITEMS
                && seen.insert(word.to_string())
            {
                items.push(word.to_string());
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
