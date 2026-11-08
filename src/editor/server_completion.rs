//! Completion from a language server: asked for as you type (with
//! `auto-complete`: two letters of a word, or a character the server
//! completes after, like `.`) or with `<C-Space>`, and shown in the same
//! menu as word completion, by the cursor.
//!
//! Unlike word completion nothing goes in until it's chosen: `<C-n>` and
//! `<C-p>` put the items in turn (Vim-style), `<C-y>` or `<Tab>` takes the
//! chosen one (or the first) and makes its other edits (an import), and
//! typing more of the word narrows the list, fuzzily. Anything else (a
//! space, moving away, `<Esc>`) closes it, keeping what's in the text.

use super::completion::{WordCompletion, is_word_char};
use super::lsp::{LspFeature, LspRequest};
use super::picker::fuzzy_match;
use super::{Editor, Mode};
use crate::core::Position;

/// How many letters of a word start completing as you type.
const TRIGGER_LENGTH: usize = 2;

/// The most items shown, of those that match.
const MAX_SHOWN: usize = 200;

/// One of the server's completions, made ready for the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerItem {
    /// What the menu shows (`area(…)`).
    pub label: String,
    /// What goes in, in place of the word typed (`area`).
    pub text: String,
    /// What typing is matched against, and the server's order.
    pub filter: String,
    pub sort: String,
    /// Shown beside it: a type or signature, or the kind of thing it is.
    pub detail: String,
    /// Other edits it comes with (an import at the top): start, end, text,
    /// in the buffer's chars as it was when they were asked for.
    pub other_edits: Vec<(Position, Position, String)>,
    /// When the server gives the other edits only on asking (resolving):
    /// which answer and item it was, to ask about once it's taken.
    pub resolve: Option<(u64, usize)>,
}

/// A server's completion in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCompletion {
    pub all: Vec<ServerItem>,
    /// Which of `all` each shown item is.
    pub shown: Vec<usize>,
    /// The server has more than it sent: ask again as typing goes on.
    pub incomplete: bool,
}

impl Editor {
    /// Where the word before the cursor starts, and the cursor.
    fn word_before_cursor(&self) -> (Position, Position) {
        let cursor = self.document.cursor();
        let line = self.document.buffer().line(cursor.row);
        let before: Vec<char> = line.chars().take(cursor.col).collect();
        let len = before
            .iter()
            .rev()
            .take_while(|c| is_word_char(**c))
            .count();
        (cursor.with_col(cursor.col - len), cursor)
    }

    /// `<C-Space>` / `<C-x><C-o>`: asks the server for completions here.
    pub fn request_completion(&mut self) -> Result<(), String> {
        self.completion_used = true;
        self.queue_completion(true, None)
    }

    /// After a key that changed the text in insert mode: asks the server,
    /// if what was typed calls for it.
    pub(super) fn auto_complete(&mut self) {
        if !(self.options.lsp && self.options.auto_complete) || self.mode() != Mode::Insert {
            return;
        }
        if self.completion.as_ref().is_some_and(|c| c.server.is_none()) {
            return;
        }
        let (start, cursor) = self.word_before_cursor();
        let showing = self.completion.as_ref().and_then(|c| c.server.as_ref());
        if let Some(server) = showing {
            // The list narrows by itself, unless the server has more.
            if server.incomplete {
                let _ = self.queue_completion(false, None);
            }
            return;
        }
        if cursor.col - start.col >= TRIGGER_LENGTH {
            let _ = self.queue_completion(false, None);
        } else if cursor.col == start.col && cursor.col > 0 {
            // Maybe one the server completes after (`.`, `::`); it decides.
            let before = self
                .document
                .buffer()
                .char_at(cursor.with_col(cursor.col - 1));
            if let Some(c) = before.filter(|c| !c.is_whitespace()) {
                let _ = self.queue_completion(false, Some(c));
            }
        }
    }

    fn queue_completion(&mut self, manual: bool, trigger: Option<char>) -> Result<(), String> {
        if !self.options.lsp {
            return Err("Language servers are off (lsp = false)".into());
        }
        if self.document.path().is_none() {
            return Err("No language server: the buffer has no file".into());
        }
        let (start, cursor) = self.word_before_cursor();
        let buffer = self.document.buffer();
        self.lsp_requests.push(LspRequest {
            feature: LspFeature::Completion,
            document: self.document.id(),
            position: cursor,
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            manual,
            trigger,
            word_start: start,
            item: None,
        });
        Ok(())
    }

    /// Whether the editor is still completing the word `request` was
    /// about (though more of it may have been typed since).
    pub fn still_completing(&self, request: &LspRequest) -> bool {
        let (start, cursor) = self.word_before_cursor();
        self.document.id() == request.document
            && self.mode() == Mode::Insert
            && start == request.word_start
            && cursor.row == request.position.row
            && self.completion.as_ref().is_none_or(|c| c.server.is_some())
    }

    /// Shows the server's answer to `request`: `items`, narrowed to what's
    /// typed of the word by now.
    pub fn show_server_completion(
        &mut self,
        request: &LspRequest,
        items: Vec<ServerItem>,
        incomplete: bool,
    ) {
        if !self.still_completing(request) {
            return;
        }
        // An item chosen with <C-n> stays chosen: a late answer (to a key
        // typed before) doesn't take it away.
        let chosen = self
            .completion
            .as_ref()
            .is_some_and(|c| c.selected.is_some() && !request.manual);
        if chosen {
            return;
        }
        if items.is_empty() {
            if request.manual {
                self.set_message(super::Message::info("No completions here"));
            }
            self.completion = None;
            return;
        }
        self.completion = Some(WordCompletion {
            start: request.word_start,
            typed: String::new(),
            items: Vec::new(),
            selected: None,
            labels: Vec::new(),
            details: Vec::new(),
            server: Some(ServerCompletion {
                all: items,
                shown: Vec::new(),
                incomplete,
            }),
        });
        if !self.refilter_server_completion() {
            self.completion = None;
        }
    }

    /// Narrows the server's completion to what's typed of the word. Returns
    /// false (and the caller ends it) if the cursor has left the word or
    /// nothing matches.
    pub(super) fn refilter_server_completion(&mut self) -> bool {
        let (start, cursor) = self.word_before_cursor();
        let Some(completion) = &mut self.completion else {
            return false;
        };
        if self.mode != Mode::Insert || start != completion.start {
            return false;
        }
        let typed = self.document.buffer().text_range(start, cursor);
        let Some(server) = &mut completion.server else {
            return false;
        };
        // Those starting with what's typed first, then those starting with
        // it in another case, then looser (fuzzy) matches; the server's
        // order within each.
        let lower = typed.to_lowercase();
        let mut matches: Vec<(u8, usize)> = server
            .all
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let tier = if item.filter.starts_with(&typed) {
                    0
                } else if item.filter.to_lowercase().starts_with(&lower) {
                    1
                } else if fuzzy_match(&typed, &item.filter).is_some() {
                    2
                } else {
                    return None;
                };
                Some((tier, i))
            })
            .collect();
        matches.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| server.all[a.1].sort.cmp(&server.all[b.1].sort))
        });
        matches.truncate(MAX_SHOWN);
        server.shown = matches.into_iter().map(|(_, i)| i).collect();
        // Just the word as typed: nothing to offer.
        let only_itself = server.shown.len() == 1 && server.all[server.shown[0]].text == typed;
        if server.shown.is_empty() || only_itself {
            return false;
        }
        completion.items = server
            .shown
            .iter()
            .map(|&i| server.all[i].text.clone())
            .collect();
        completion.labels = server
            .shown
            .iter()
            .map(|&i| server.all[i].label.clone())
            .collect();
        completion.details = server
            .shown
            .iter()
            .map(|&i| server.all[i].detail.clone())
            .collect();
        completion.typed = typed;
        completion.selected = None;
        true
    }

    /// Whether a server's completion menu is up (so `<Tab>` takes from it).
    pub fn server_completion_showing(&self) -> bool {
        self.completion.as_ref().is_some_and(|c| c.server.is_some())
    }

    /// Takes the chosen item (or the first), and makes its other edits.
    pub(super) fn accept_server_completion(&mut self) {
        let Some(completion) = &mut self.completion else {
            return;
        };
        if completion.selected.is_none() {
            completion.selected = Some(0);
            self.show_completion_choice();
        }
        let Some(completion) = &self.completion else {
            return;
        };
        let (Some(i), Some(server)) = (completion.selected, &completion.server) else {
            return;
        };
        let item = &server.all[server.shown[i]];
        if let Some(resolve) = item.resolve {
            // Its import comes when the server answers.
            let buffer = self.document.buffer();
            self.lsp_requests.push(LspRequest {
                feature: LspFeature::ResolveCompletion,
                document: self.document.id(),
                position: self.document.cursor(),
                buffer_id: buffer.id(),
                edit_seq: buffer.edit_seq(),
                manual: false,
                trigger: None,
                word_start: completion.start,
                item: Some(resolve),
            });
        }
        let edits = item.other_edits.clone();
        self.make_other_edits(edits);
    }

    /// Makes a completion's other edits (an import), keeping the cursor on
    /// the text it was on. Edits that overlap the cursor are left out.
    pub fn make_other_edits(&mut self, mut edits: Vec<(Position, Position, String)>) {
        // Last first, so each edit's place is still where it was; the
        // cursor moves with the text edited before it.
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let mut cursor = self.document.cursor();
        for (start, end, text) in edits {
            if end > cursor && start < cursor {
                continue; // Overlaps the word: not safe to make.
            }
            self.document.delete(start, end);
            let new_end = self.document.insert(start, &text);
            if end <= cursor {
                cursor = moved(cursor, end, new_end);
            }
        }
        self.document.set_cursor(cursor);
    }
}

/// Where `cursor` (after `end`) is once the text up to `end` is replaced
/// by text ending at `new_end`.
fn moved(cursor: Position, end: Position, new_end: Position) -> Position {
    if end.row == cursor.row {
        Position::new(new_end.row, new_end.col + (cursor.col - end.col))
    } else {
        let rows = new_end.row as isize - end.row as isize;
        Position::new((cursor.row as isize + rows) as usize, cursor.col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_moves_with_edits_before_it() {
        let at = Position::new;
        // `use a;\n` put in above: one row down.
        assert_eq!(moved(at(3, 4), at(0, 0), at(1, 0)), at(4, 4));
        // On its row, before it: along by the difference (`2..5` to `2..3`).
        assert_eq!(moved(at(0, 10), at(0, 5), at(0, 3)), at(0, 8));
        // A line joined into the one above (`0,3..1,0` deleted).
        assert_eq!(moved(at(1, 4), at(1, 0), at(0, 3)), at(0, 7));
    }
}
