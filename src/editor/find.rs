//! The find box (`<Space>/`, `<Space>h` with the replace field), as in
//! VS Code: it floats at the top right of the window, finds as you type
//! (the cursor goes to the first match from where it opened, every match
//! highlighted), counts the matches, and replaces one or all of them.
//!
//! What's typed is found as it is; the toggles make it match case (`Aa`,
//! `<A-c>`), whole words (`W`, `<A-w>`) or a regex (`.*`, `<A-r>`, Rust's
//! `regex` syntax, with `$1` in the replacement). It has focus in
//! `Mode::Find`, with its own `[keys.find]`. Closing it leaves its search
//! as the last one, so `n`/`N` carry on and the matches stay highlighted.

use std::cell::RefCell;

use super::text_field::TextField;
use super::{DocumentId, Editor, Message, Mode};
use crate::core::search::{Match, SearchPattern};
use crate::core::{Buffer, Position};

/// The most matches counted (the box shows `10000+`).
pub const MAX_COUNTED: usize = 10_000;

/// Which field has focus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FindField {
    #[default]
    Find,
    Replace,
}

#[derive(Debug, Default)]
pub struct FindState {
    pub find: TextField,
    pub replace: TextField,
    pub focus: FindField,
    /// Whether the replace field is shown.
    pub replacing: bool,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Where finding as you type starts from.
    origin: Position,
    /// The pattern typed (or why it isn't one).
    pattern: Option<Result<SearchPattern, String>>,
    /// The matches in the buffer, counted for the box (kept until the
    /// pattern or the text changes).
    counted: RefCell<Option<Counted>>,
}

#[derive(Debug, Clone)]
struct Counted {
    document: DocumentId,
    edits: u64,
    matches: Vec<Match>,
    /// Whether there are more than `MAX_COUNTED`.
    more: bool,
}

/// What the box shows after the fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindCount {
    /// Nothing typed.
    Empty,
    /// The regex doesn't compile.
    Invalid,
    /// The match at the cursor (1-based, if it's on one), out of `total`,
    /// and whether there are more than that.
    Found {
        current: Option<usize>,
        total: usize,
        more: bool,
    },
}

impl FindState {
    pub fn pattern(&self) -> Option<&SearchPattern> {
        self.pattern.as_ref().and_then(|p| p.as_ref().ok())
    }

    /// Why what's typed isn't a pattern (a regex that doesn't compile).
    pub fn error(&self) -> Option<&str> {
        self.pattern
            .as_ref()
            .and_then(|p| p.as_ref().err())
            .map(String::as_str)
    }

    /// Compiles what's typed, with the toggles.
    fn rebuild(&mut self) {
        *self.counted.get_mut() = None;
        let text = self.find.text();
        if text.is_empty() {
            self.pattern = None;
            return;
        }
        let body = if self.regex {
            text.to_string()
        } else {
            regex::escape(text)
        };
        let body = if self.whole_word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        self.pattern = Some(SearchPattern::from_regex(&body, text, !self.match_case));
    }

    /// The field with focus.
    pub fn field(&mut self) -> &mut TextField {
        match self.focus {
            FindField::Find => &mut self.find,
            FindField::Replace => &mut self.replace,
        }
    }

    /// The replacement for one match: as typed, or with a regex, expanded
    /// (`$1`) by the caller.
    fn literal_replacement(&self) -> String {
        if self.regex {
            self.replace.text().to_string()
        } else {
            // `regex` replacement syntax, so `$` is written `$$`.
            self.replace.text().replace('$', "$$")
        }
    }

    /// The count for the box, counting the matches in `buffer` if the
    /// pattern or the text changed.
    fn count(&self, document: DocumentId, edits: u64, buffer: &Buffer, at: Position) -> FindCount {
        let Some(result) = &self.pattern else {
            return FindCount::Empty;
        };
        let Ok(pattern) = result else {
            return FindCount::Invalid;
        };
        let mut counted = self.counted.borrow_mut();
        let stale = counted
            .as_ref()
            .is_none_or(|c| c.document != document || c.edits != edits);
        if stale {
            let mut matches = Vec::new();
            let mut more = false;
            'rows: for row in 0..buffer.line_count() {
                for (start, end) in pattern.matches_in_line(&buffer.line(row)) {
                    if matches.len() == MAX_COUNTED {
                        more = true;
                        break 'rows;
                    }
                    matches.push(Match { row, start, end });
                }
            }
            *counted = Some(Counted {
                document,
                edits,
                matches,
                more,
            });
        }
        let counted = counted.as_ref().expect("just counted");
        let current = counted
            .matches
            .binary_search_by(|m| m.start_position().cmp(&at))
            .ok()
            .map(|i| i + 1);
        FindCount::Found {
            current,
            total: counted.matches.len(),
            more: counted.more,
        }
    }
}

/// The first non-empty match at or after `from` (or with `!forward`, the
/// last before it), wrapping around the buffer. `inclusive` takes one
/// starting at `from` (going forward).
pub fn find_match(
    pattern: &SearchPattern,
    buffer: &Buffer,
    from: Position,
    forward: bool,
    inclusive: bool,
) -> Option<Match> {
    let rows = buffer.line_count();
    for step in 0..=rows {
        let row = if forward {
            (from.row + step) % rows
        } else {
            (from.row + rows * 2 - step) % rows
        };
        let matches = pattern.matches_in_line(&buffer.line(row));
        let found = match (step, forward) {
            (0, true) => matches
                .into_iter()
                .find(|&(s, _)| s > from.col || (inclusive && s == from.col)),
            (0, false) => matches.into_iter().rev().find(|&(s, _)| s < from.col),
            // Back on the first row after going all the way round.
            (_, true) if step == rows => matches.into_iter().find(|&(s, _)| s <= from.col),
            (_, false) if step == rows => matches.into_iter().rev().find(|&(s, _)| s >= from.col),
            (_, true) => matches.into_iter().next(),
            (_, false) => matches.into_iter().next_back(),
        };
        if let Some((start, end)) = found {
            return Some(Match { row, start, end });
        }
    }
    None
}

impl Editor {
    pub fn find_state(&self) -> &FindState {
        &self.find
    }

    /// `<Space>/` (and `<Space>h`, with `replace`): opens the find box with
    /// focus in its find field, seeded with the visual selection (one
    /// line), or the word under the cursor if nothing was found before, or
    /// else what was; selected, so typing replaces it.
    pub fn open_find(&mut self, replace: bool) {
        let selected = self
            .selection_range()
            .filter(|_| self.mode() == Mode::Visual)
            .and_then(|range| match range {
                crate::core::Range::Chars { start, end } if start.row == end.row => {
                    Some(self.document.buffer().text_range(start, end))
                }
                _ => None,
            })
            .filter(|text| !text.is_empty());
        let from_selection = selected.is_some();
        let seed = selected.or_else(|| {
            self.find
                .find
                .text()
                .is_empty()
                .then(|| self.word_under_cursor(false))
                .flatten()
        });
        let seeded_word = !from_selection && seed.is_some();
        match seed {
            Some(text) => self.find.find.set_selected(&text),
            None => self.find.find.select_all(),
        }
        // A selection's start, so its own text is the first match.
        let origin = match self.selection_range() {
            Some(crate::core::Range::Chars { start, .. }) if self.mode() == Mode::Visual => start,
            // The word the cursor's in, so it's the first match.
            _ if seeded_word => self.word_start(),
            _ => self.document.cursor(),
        };
        self.set_mode(Mode::Find);
        self.find.replacing |= replace;
        self.find.focus = FindField::Find;
        self.find.origin = origin;
        self.find_as_typed();
    }

    /// The start of the word the cursor's in (or the cursor, if it's not
    /// in one).
    fn word_start(&self) -> Position {
        let cursor = self.document.cursor();
        let line: Vec<char> = self.document.buffer().line(cursor.row).chars().collect();
        let word = |c: &char| c.is_alphanumeric() || *c == '_';
        if !line.get(cursor.col).is_some_and(word) {
            return cursor;
        }
        let start = line[..cursor.col]
            .iter()
            .rev()
            .take_while(|c| word(c))
            .count();
        cursor.with_col(cursor.col - start)
    }

    /// `<Esc>`: closes the box, the cursor left at the match. Its search
    /// becomes the last one (for `n`, `N` and the highlight).
    pub fn close_find(&mut self) {
        if let Some(pattern) = self.find.pattern().cloned() {
            self.set_last_search(pattern);
        }
        self.find.replacing = false;
        self.set_mode(Mode::Normal);
    }

    /// The match the cursor's on, while the box has focus (drawn apart
    /// from the others).
    pub fn find_current_match(&self) -> Option<Match> {
        if self.mode() != Mode::Find {
            return None;
        }
        let pattern = self.find.pattern()?;
        let cursor = self.document.cursor();
        find_match(pattern, self.document.buffer(), cursor, true, true)
            .filter(|m| m.start_position() == cursor)
    }

    /// The box's count of matches.
    pub fn find_count(&self) -> FindCount {
        self.find.count(
            self.document.id(),
            self.document.edit_count(),
            self.document.buffer(),
            self.document.cursor(),
        )
    }

    /// After the find field or a toggle changes: goes to the first match
    /// from where the box opened, or back there if there's none.
    fn find_as_typed(&mut self) {
        self.find.rebuild();
        let origin = self.find.origin;
        let target = self
            .find
            .pattern()
            .and_then(|p| find_match(p, self.document.buffer(), origin, true, true));
        self.document
            .set_cursor(target.map_or(origin, |m| m.start_position()));
        self.reveal_cursor();
    }

    /// Edits the field with focus (`edit` is one of `TextField`'s), finding
    /// again if it's the find field.
    pub fn find_edit(&mut self, edit: impl FnOnce(&mut TextField)) {
        edit(self.find.field());
        if self.find.focus == FindField::Find {
            self.find_as_typed();
        }
    }

    /// `<Tab>`: between the find and replace fields (showing the replace
    /// field).
    pub fn find_switch_field(&mut self) {
        self.find.replacing = true;
        self.find.focus = match self.find.focus {
            FindField::Find => FindField::Replace,
            FindField::Replace => FindField::Find,
        };
    }

    /// `<A-c>`, `<A-w>`, `<A-r>`: flips match case, whole word or regex.
    pub fn find_toggle(&mut self, toggle: impl FnOnce(&mut FindState)) {
        toggle(&mut self.find);
        self.find_as_typed();
    }

    /// `<CR>` / `<Down>` (or `<S-CR>` / `<Up>`): the next (or previous)
    /// match; finding as you type carries on from there.
    pub fn find_step(&mut self, forward: bool) -> Result<(), String> {
        let pattern = self.find_pattern()?;
        let cursor = self.document.cursor();
        let found = find_match(&pattern, self.document.buffer(), cursor, forward, false)
            .ok_or_else(|| format!("No results for {}", self.find.find.text()))?;
        self.document.set_cursor(found.start_position());
        self.reveal_cursor();
        self.find.origin = found.start_position();
        Ok(())
    }

    fn find_pattern(&self) -> Result<SearchPattern, String> {
        match &self.find.pattern {
            None => Err("Nothing to find".into()),
            Some(Err(e)) => Err(e.clone()),
            Some(Ok(p)) => Ok(p.clone()),
        }
    }

    /// `<CR>` in the replace field: replaces the match at the cursor and
    /// goes to the next; if the cursor isn't on one, goes to the next first
    /// (as VS Code does).
    pub fn find_replace_one(&mut self) -> Result<(), String> {
        let pattern = self.find_pattern()?;
        let Some(current) = self.find_current_match() else {
            return self.find_step(true);
        };
        let line = self.document.buffer().line(current.row).into_owned();
        let replacement = self.find.literal_replacement();
        let (start, end, text) = pattern
            .next_replacement(&line, current.start, &replacement)
            .filter(|&(start, ..)| start == current.start)
            .ok_or("The match changed")?;
        let at = Position::new(current.row, start);
        self.document.delete(at, Position::new(current.row, end));
        let after = self.document.insert(at, &text);
        // The next match after what was put in (never inside it).
        let next = find_match(&pattern, self.document.buffer(), after, true, true);
        let target = next.map_or(at, |m| m.start_position());
        self.document.set_cursor(target);
        self.find.origin = target;
        self.reveal_cursor();
        Ok(())
    }

    /// `<A-a>`: replaces every match in the buffer, as one undo step.
    pub fn find_replace_all(&mut self) -> Result<(), String> {
        let pattern = self.find_pattern()?;
        let replacement = self.find.literal_replacement();
        let mut replaced = 0;
        let mut lines = 0;
        // Bottom-up, so lines put in don't move the rows still to do.
        for row in (0..self.document.buffer().line_count()).rev() {
            let line = self.document.buffer().line(row).into_owned();
            if pattern.matches_in_line(&line).is_empty() {
                continue;
            }
            let (text, count) = pattern.replace(&line, &replacement, true);
            if text != line {
                let end = self.document.buffer().line_end(row);
                self.document.delete(Position::new(row, 0), end);
                self.document.insert(Position::new(row, 0), &text);
            }
            replaced += count;
            lines += 1;
        }
        if replaced == 0 {
            return Err(format!("No results for {}", self.find.find.text()));
        }
        let plural = |n: usize| if n == 1 { "" } else { "es" };
        let line_plural = if lines == 1 { "" } else { "s" };
        self.set_message(Message::info(format!(
            "Replaced {replaced} match{} on {lines} line{line_plural}",
            plural(replaced)
        )));
        self.enforce_cursor_rules();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::search::CaseMode;

    fn editor(text: &str) -> Editor {
        let mut editor = Editor::default();
        editor.document.insert(Position::new(0, 0), text);
        editor.document.set_cursor(Position::new(0, 0));
        editor
    }

    fn type_find(editor: &mut Editor, text: &str) {
        for c in text.chars() {
            editor.find_edit(|f| f.push(c));
        }
    }

    fn cursor(editor: &Editor) -> (usize, usize) {
        let c = editor.document.cursor();
        (c.row, c.col)
    }

    #[test]
    fn finds_as_you_type_and_counts() {
        let mut e = editor("one two\nTwo three\ntwo.x");
        e.document.set_cursor(Position::new(0, 5));
        e.open_find(false);
        assert_eq!(e.find_state().find.text(), "two", "seeded with the word");
        assert!(e.find_state().find.is_selected());
        type_find(&mut e, "t");
        assert_eq!(e.find_state().find.text(), "t", "typing replaced it");
        type_find(&mut e, "wo");
        assert_eq!(cursor(&e), (0, 4), "the match at the cursor");
        assert_eq!(
            e.find_count(),
            FindCount::Found {
                current: Some(1),
                total: 3,
                more: false
            },
            "case ignored unless asked"
        );
        e.find_step(true).unwrap();
        assert_eq!(cursor(&e), (1, 0));
        e.find_toggle(|f| f.match_case = true);
        assert_eq!(cursor(&e), (2, 0), "Two no longer matches");
        e.find_step(true).unwrap();
        assert_eq!(cursor(&e), (0, 4), "wraps round");
        e.find_step(false).unwrap();
        assert_eq!(cursor(&e), (2, 0), "and back");
    }

    #[test]
    fn text_is_literal_unless_regex_is_on() {
        let mut e = editor("a.c abc a\\<c");
        e.open_find(false);
        type_find(&mut e, "a.c");
        assert_eq!(
            e.find_count(),
            FindCount::Found {
                current: Some(1),
                total: 1,
                more: false
            }
        );
        e.find_toggle(|f| f.regex = true);
        assert!(matches!(e.find_count(), FindCount::Found { total: 2, .. }));
        type_find(&mut e, "(");
        assert_eq!(e.find_count(), FindCount::Invalid);
        assert_eq!(cursor(&e), (0, 0), "back where it began");

        e.find_toggle(|f| f.regex = false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "a\\<c");
        assert_eq!(cursor(&e), (0, 8), "a literal backslash");
    }

    #[test]
    fn whole_words() {
        let mut e = editor("cat concat cat_x cat");
        e.open_find(false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "cat");
        assert!(matches!(e.find_count(), FindCount::Found { total: 4, .. }));
        e.find_toggle(|f| f.whole_word = true);
        assert!(matches!(e.find_count(), FindCount::Found { total: 2, .. }));
        e.find_step(true).unwrap();
        assert_eq!(cursor(&e), (0, 17));
    }

    #[test]
    fn replaces_one_at_a_time_then_all_as_one_undo_step() {
        let mut e = editor("foo bar foo\nfoo $x");
        e.open_find(true);
        assert!(e.find_state().replacing);
        assert_eq!(cursor(&e), (0, 0));
        e.find_switch_field();
        e.find_edit(|f| "a$b".chars().for_each(|c| f.push(c)));
        assert_eq!(e.find_state().find.text(), "foo");
        e.find_replace_one().unwrap();
        e.finish_command();
        assert_eq!(
            e.document.buffer().text(),
            "a$b bar foo\nfoo $x\n".trim_end()
        );
        assert_eq!(cursor(&e), (0, 8), "on to the next");

        e.find_replace_all().unwrap();
        e.finish_command();
        assert_eq!(e.document.buffer().text(), "a$b bar a$b\na$b $x");
        assert_eq!(
            e.message().map(|m| m.text.as_str()),
            Some("Replaced 2 matches on 2 lines")
        );
        e.document.undo();
        assert_eq!(e.document.buffer().text(), "a$b bar foo\nfoo $x");
    }

    #[test]
    fn regex_replacements_use_captures() {
        let mut e = editor("x=1, y=22");
        e.open_find(true);
        e.find_toggle(|f| f.regex = true);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, r"(\w)=(\d+)");
        e.find_switch_field();
        e.find_edit(|f| "$2:$1".chars().for_each(|c| f.push(c)));
        e.find_replace_all().unwrap();
        assert_eq!(e.document.buffer().text(), "1:x, 22:y");
    }

    #[test]
    fn closing_leaves_the_search_for_n() {
        let mut e = editor("ab ab ab");
        e.open_find(false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "ab");
        e.find_step(true).unwrap();
        e.close_find();
        assert_eq!(e.mode(), Mode::Normal);
        e.search_next(1, false).unwrap();
        assert_eq!(cursor(&e), (0, 6));
        assert!(e.search_highlight().is_some());
    }

    #[test]
    fn find_match_wraps_and_skips_the_start() {
        let buffer = Buffer::from_text("ab\nxab ab");
        let p = SearchPattern::new("ab", CaseMode::Sensitive).unwrap();
        let at = |r, c| Position::new(r, c);
        let found =
            |from, fwd, inc| find_match(&p, &buffer, from, fwd, inc).map(|m| (m.row, m.start));
        assert_eq!(found(at(0, 0), true, true), Some((0, 0)));
        assert_eq!(found(at(0, 0), true, false), Some((1, 1)));
        assert_eq!(found(at(1, 4), true, false), Some((0, 0)), "wraps");
        assert_eq!(found(at(0, 0), false, false), Some((1, 4)), "wraps back");
        let one = Buffer::from_text("ab");
        assert_eq!(
            find_match(&p, &one, at(0, 0), true, false).map(|m| m.start),
            Some(0),
            "the only match, all the way round"
        );
    }
}
