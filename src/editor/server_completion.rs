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
    /// What goes in (`area`), from `start` to the cursor: usually the
    /// start of the word being typed, but some go in from before it (in
    /// place of a `.` typed).
    pub text: String,
    pub start: Position,
    /// Where the cursor goes in `text` (in chars) once it's taken: a
    /// snippet's first tab stop. `None`: after it.
    pub cursor: Option<usize>,
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
    /// Where the word being completed starts. (The completion's `start` is
    /// the earliest of its items' starts.)
    pub word_start: Position,
    /// Which of `all` each shown item is.
    pub shown: Vec<usize>,
    /// The server has more than it sent: ask again as typing goes on.
    pub incomplete: bool,
}

/// How well `typed` matches `filter`: 0 if it starts with it, 1 if it does
/// in another case, 2 if its letters are in it in order (fuzzily) and the
/// first starts a word in it (`fm` for `flat_map`, `ce` for
/// `createElement`, but not `0` for `log10`).
fn match_tier(typed: &str, filter: &str) -> Option<u8> {
    if filter.starts_with(typed) {
        Some(0)
    } else if filter.to_lowercase().starts_with(&typed.to_lowercase()) {
        Some(1)
    } else if starts_a_word(typed.chars().next()?, filter) && fuzzy_match(typed, filter).is_some() {
        Some(2)
    } else {
        None
    }
}

/// Whether `c` (in any case) starts a word in `text`: at its start, after
/// something that isn't a letter or digit, or as a capital after a small
/// letter (camelCase).
fn starts_a_word(c: char, text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.iter().enumerate().any(|(i, &t)| {
        let same = t.to_lowercase().eq(c.to_lowercase());
        let start = i == 0
            || !chars[i - 1].is_alphanumeric()
            || (chars[i - 1].is_lowercase() && t.is_uppercase());
        same && start
    })
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
            text: None,
            end: cursor,
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
        // From the earliest any item goes in, so they all replace the same
        // text and stepping through them works as for words.
        let start = items
            .iter()
            .map(|i| i.start)
            .min()
            .unwrap_or(request.word_start);
        self.completion = Some(WordCompletion {
            start,
            typed: String::new(),
            items: Vec::new(),
            selected: None,
            labels: Vec::new(),
            details: Vec::new(),
            server: Some(ServerCompletion {
                word_start: request.word_start,
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
        let (word_start, cursor) = self.word_before_cursor();
        let buffer = self.document.buffer();
        let Some(completion) = &mut self.completion else {
            return false;
        };
        let Some(server) = &mut completion.server else {
            return false;
        };
        if self.mode != Mode::Insert || word_start != server.word_start {
            return false;
        }
        let start = completion.start;
        let typed = buffer.text_range(start, cursor);
        // What's typed from the word's start, and from each other place
        // items go in (there are only one or two): taken from the buffer
        // once, not for each of hundreds of items.
        let from_word = buffer.text_range(word_start, cursor);
        let mut from_starts: Vec<(Position, String)> = Vec::new();
        for item in &server.all {
            if item.start != word_start && !from_starts.iter().any(|(p, _)| *p == item.start) {
                from_starts.push((item.start, buffer.text_range(item.start, cursor)));
            }
        }
        // Each item is matched against what's typed from where it goes in
        // (`.ap` for one that replaces the `.`). Those starting with it
        // first, then those starting with it in another case, then looser
        // (fuzzy) matches; the server's order within each.
        let mut matches: Vec<(u8, usize)> = server
            .all
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                // Its filter text may include what's before the word (the
                // TypeScript server's `.?.append`) or not (clangd's `x` for
                // `->x`): the better of the two.
                let from_item = from_starts
                    .iter()
                    .find(|(p, _)| *p == item.start)
                    .map_or(from_word.as_str(), |(_, text)| text.as_str());
                let tier = [from_item, from_word.as_str()]
                    .iter()
                    .filter_map(|typed| match_tier(typed, &item.filter))
                    .min()?;
                Some((tier, i))
            })
            .collect();
        matches.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| server.all[a.1].sort.cmp(&server.all[b.1].sort))
        });
        matches.truncate(MAX_SHOWN);
        server.shown = matches.into_iter().map(|(_, i)| i).collect();
        // Each item's text from the completion's start: what's between that
        // and where the item goes in (a `.`), then its own.
        let texts: Vec<String> = server
            .shown
            .iter()
            .map(|&i| {
                let item = &server.all[i];
                if item.start == start {
                    item.text.clone()
                } else {
                    buffer.text_range(start, item.start) + &item.text
                }
            })
            .collect();
        // Just the word as typed: nothing to offer.
        let only_itself = texts.len() == 1 && texts[0] == typed;
        if server.shown.is_empty() || only_itself {
            return false;
        }
        completion.items = texts;
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
                text: None,
                end: self.document.cursor(),
            });
        }
        // A snippet's cursor place: counted from the completion's start
        // (where its text is), past what's before the item's own start.
        if let Some(at) = item.cursor {
            let text = &completion.items[i];
            let before = text.chars().count() - item.text.chars().count();
            let mut place = completion.start;
            for c in text.chars().take(before + at) {
                place = if c == '\n' {
                    Position::new(place.row + 1, 0)
                } else {
                    place.with_col(place.col + 1)
                };
            }
            self.document.set_cursor(place);
        }
        let edits = item.other_edits.clone();
        self.make_other_edits(edits);
    }

    /// Makes a completion's other edits (an import), keeping the cursor on
    /// the text it was on. Edits that overlap the word just put in are left
    /// out.
    pub fn make_other_edits(&mut self, edits: Vec<(Position, Position, String)>) {
        let cursor = self.document.cursor();
        let (start, _) = self.word_before_cursor();
        let edits = edits
            .into_iter()
            .filter(|(from, to, _)| !(*to > start && *from < cursor))
            .collect();
        self.apply_text_edits(edits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rank_prefix_then_other_case_then_fuzzy() {
        assert_eq!(match_tier("ar", "area"), Some(0));
        assert_eq!(match_tier("ar", "ArithmeticError"), Some(1));
        assert_eq!(match_tier("ar", "_parse_args"), Some(2), "`args` is a word");
        assert_eq!(match_tier("fm", "flat_map"), Some(2));
        assert_eq!(match_tier("ce", "createElement"), Some(2));
        assert_eq!(match_tier("el", "createElement"), Some(2), "camelCase word");
        assert_eq!(match_tier("0", "log10"), None, "not at a word's start");
        assert_eq!(match_tier("rt", "parts"), None);
        assert_eq!(match_tier("ar", "xyz"), None);
        assert_eq!(match_tier("", "anything"), Some(0));
    }
}
