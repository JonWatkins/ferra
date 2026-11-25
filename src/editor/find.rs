//! The find box (`<Space>/`, `<Space>h` with the replace field), as in
//! VS Code: it floats at the top right of the window, finds as you type
//! (the cursor goes to the first match from where it opened, every match
//! highlighted), counts the matches, and replaces one or all of them.
//!
//! What's typed is found as it is; the toggles make it match case (`Aa`,
//! `<A-c>`), whole words (`W`, `<A-w>`) or a regex (`.*`, `<A-r>`, Rust's
//! `regex` syntax, with `$1` in the replacement), and preserve case (`AB`,
//! `<A-p>`) cases each replacement like what it replaces
//! (`core::search::preserve_case`, VS Code's rules). It has focus in
//! `Mode::Find`, with its own `[keys.find]`. Closing it leaves its search
//! as the last one, so `n`/`N` carry on; its highlights go with it, as in
//! VS Code, and come back with the next `n`/`N`.
//!
//! `editor/find.rs` holds the find box's state (`FindState`): two `TextField`s
//! (`editor/text_field.rs`, a one-line field whose text can start selected, for
//! reuse by later boxes), which has focus, the toggles, and where finding as
//! you type starts. Every change to the find field or a toggle compiles a
//! pattern with `SearchPattern::from_regex` (the text escaped unless regex is
//! on, wrapped in `\b(?:...)\b` for whole words, no Vim translation) and moves
//! the cursor to the first match at or after that start; `<CR>` moves the start
//! along. `find_match` walks lines for the next non-empty match either way,
//! wrapping, so the box never lands on an empty regex match.
//!
//! The box has focus in `Mode::Find` (its own `[keys.find]`, text typed into
//! the field). `search_highlight` returns its pattern while it's open, and
//! `find_current_match` the match at the cursor, which the editor pane draws
//! with `ui.search.current`. The count is made the first time it's drawn after
//! the pattern or the text changes (keyed by the document's id and edit count,
//! so drawing doesn't search again), up to 10,000. Replacing goes through
//! `SearchPattern::next_replacement` / `replace` (a literal replacement has `$`
//! doubled); each command is an undo step as usual, so replace all is one.
//! What a match becomes is a `Replacement` (`FindState::replacement`), which
//! the Search view replaces with too; preserve case maps each expanded match
//! through `SearchPattern::replace_mapped`. Flipping it doesn't find again.
//! Closing hands the pattern to `set_last_search` and clears the highlight
//! (`clear_search_highlight`), as VS Code does; `n`/`N` turn it back on. The
//! app treats keys in find mode like the `:` line: replacements aren't repeated
//! by `.`.
//!
//! In selection (`≡`, `<A-l>`) finds only in a `FindScope`: the chars of
//! each row of a selection, so lines, chars across lines and a block are
//! all one shape. Opening the box from any selection but one line of chars
//! (which seeds the find field instead) makes one and turns it on; the
//! toggle otherwise takes the last selection (`gv`'s). `find_match`, the
//! count, both replaces and the editor pane's highlights skip matches
//! outside it, and the pane tints it (`ui.find.scope`). Replacing never
//! adds or removes lines, so the scope only needs each row's end moved by
//! what a replacement changed (`FindScope::replaced`). Closing the box
//! forgets it.
//!
//! `<Up>`/`<Down>` browse what was searched for, newest first, whatever was
//! typed (VS Code's way, not Vim's by prefix), and in the replace field what
//! was replaced with. What was found is the command line's search history,
//! shared with `/` and `?`: closing the box adds what's in the find field,
//! and replacing adds both fields. `<Down>` past the newest comes back to
//! what was typed; typing stops browsing.
//!
//! The keys the box shows come from the keymap (`Keymap::hint_keys`: the
//! binding without Alt if there is one, since macOS terminals don't send Option
//! as Alt by default, then the shortest, then the fewest modifiers), labelled
//! by `ui::find::key_label` (`⌃A` on macOS, `C-a` elsewhere). The app passes
//! them to `ui::draw` in `KeyHints`, with the splash screen's.

use std::cell::RefCell;

use super::command_line::History;
use super::text_field::TextField;
use super::{DocumentId, Editor, Message, Mode};
use crate::core::search::{Match, SearchPattern, preserve_case};
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
    /// Whether a replacement takes each match's case (`AB`).
    pub preserve_case: bool,
    /// Whether it finds only in `scope` (`≡`).
    pub in_selection: bool,
    /// The selection it finds in, kept while the toggle's off so it comes
    /// back on.
    scope: Option<FindScope>,
    /// What was replaced with (what was found is in the command line's
    /// search history, shared with `/`).
    replace_history: History,
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

/// The selection the box finds in, with "in selection" on: the chars of
/// each of its rows, as a half-open range (`usize::MAX` running to the
/// line's end), so a block is a column of them. Replacing in a row moves
/// its span's end with the text; a replacement never adds or removes a
/// line (matches don't span lines), so the rows stay put.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindScope {
    first: usize,
    spans: Vec<(usize, usize)>,
}

impl FindScope {
    /// The chars of `range`.
    fn of_range(range: crate::core::Range) -> Self {
        use crate::core::Range;
        match range {
            Range::Lines { first, last } => Self {
                first,
                spans: vec![(0, usize::MAX); last - first + 1],
            },
            Range::Chars { start, end } if start.row == end.row => Self {
                first: start.row,
                spans: vec![(start.col, end.col)],
            },
            Range::Chars { start, end } => {
                let mut spans = vec![(0, usize::MAX); end.row - start.row + 1];
                spans[0].0 = start.col;
                spans[end.row - start.row].1 = end.col;
                Self {
                    first: start.row,
                    spans,
                }
            }
        }
    }

    pub fn rows(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.spans.len()
    }

    /// Row `row`'s span, if it's in the scope.
    pub fn span(&self, row: usize) -> Option<(usize, usize)> {
        self.spans.get(row.checked_sub(self.first)?).copied()
    }

    /// Whether chars `start..end` of row `row` are all inside.
    pub fn contains(&self, row: usize, start: usize, end: usize) -> bool {
        self.span(row).is_some_and(|(s, e)| s <= start && end <= e)
    }

    /// Where the scope starts.
    fn start(&self) -> Position {
        Position::new(self.first, self.spans[0].0)
    }

    /// `old` chars in row `row` became `new` chars (inside its span).
    fn replaced(&mut self, row: usize, old: usize, new: usize) {
        if let Some(i) = row.checked_sub(self.first)
            && let Some((_, end)) = self.spans.get_mut(i)
            && *end != usize::MAX
        {
            *end = *end + new - old;
        }
    }
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

/// What the find box's (or the Search view's) matches become: the replace
/// field's text in `regex` syntax (`$1`), and whether each takes the case
/// of the text it replaces.
#[derive(Debug, Clone)]
pub(super) struct Replacement {
    text: String,
    preserve_case: bool,
}

impl Replacement {
    /// The first match in `line` at or after char column `from`, as
    /// `(start, end)`, and what it becomes.
    pub(super) fn next(
        &self,
        pattern: &SearchPattern,
        line: &str,
        from: usize,
    ) -> Option<(usize, usize, String)> {
        let (start, end, text) = pattern.next_replacement(line, from, &self.text)?;
        if !self.preserve_case {
            return Some((start, end, text));
        }
        let matched: String = line.chars().skip(start).take(end - start).collect();
        Some((start, end, preserve_case(&matched, &text)))
    }

    /// `line` with every match inside chars `span` replaced (half-open,
    /// `usize::MAX` to its end), and how many.
    pub(super) fn span(
        &self,
        pattern: &SearchPattern,
        line: &str,
        (from, to): (usize, usize),
    ) -> (String, usize) {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut done = 0;
        let mut at = from.min(chars.len());
        let mut count = 0;
        while let Some((start, end, text)) = self.next(pattern, line, at) {
            if end > to {
                break;
            }
            out.extend(&chars[done..start]);
            out.push_str(&text);
            done = end;
            count += 1;
            if end == chars.len() {
                break;
            }
            // Past an empty match, so it isn't found again.
            at = if end == start { end + 1 } else { end };
        }
        out.extend(&chars[done..]);
        (out, count)
    }

    /// `line` with every match replaced, and how many.
    pub(super) fn line(&self, pattern: &SearchPattern, line: &str) -> (String, usize) {
        pattern.replace_mapped(line, &self.text, true, |matched, text| {
            if self.preserve_case {
                preserve_case(matched, &text)
            } else {
                text
            }
        })
    }
}

impl FindState {
    pub fn pattern(&self) -> Option<&SearchPattern> {
        self.pattern.as_ref().and_then(|p| p.as_ref().ok())
    }

    /// Where it finds, with "in selection" on.
    pub fn scope(&self) -> Option<&FindScope> {
        self.scope.as_ref().filter(|_| self.in_selection)
    }

    /// Why what's typed isn't a pattern (a regex that doesn't compile).
    pub fn error(&self) -> Option<&str> {
        self.pattern
            .as_ref()
            .and_then(|p| p.as_ref().err())
            .map(String::as_str)
    }

    /// Compiles what's typed, with the toggles.
    pub(super) fn rebuild(&mut self) {
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

    /// What matches become, from the replace field and the toggles.
    pub(super) fn replacement(&self) -> Replacement {
        Replacement {
            // `regex` replacement syntax, so a literal `$` is written `$$`.
            text: if self.regex {
                self.replace.text().to_string()
            } else {
                self.replace.text().replace('$', "$$")
            },
            preserve_case: self.preserve_case,
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
            let scope = self.scope();
            let rows = scope.map_or(0..buffer.line_count(), |s| {
                s.rows().start.min(buffer.line_count())..s.rows().end.min(buffer.line_count())
            });
            'rows: for row in rows {
                for (start, end) in pattern.matches_in_line(&buffer.line(row)) {
                    if scope.is_some_and(|s| !s.contains(row, start, end)) {
                        continue;
                    }
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
/// starting at `from` (going forward). With a `scope`, only matches inside
/// it count.
pub fn find_match(
    pattern: &SearchPattern,
    buffer: &Buffer,
    scope: Option<&FindScope>,
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
        let mut matches = pattern.matches_in_line(&buffer.line(row));
        if let Some(scope) = scope {
            matches.retain(|&(start, end)| scope.contains(row, start, end));
        }
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

    /// The field with the keys: the find box's, or the Search view's while
    /// it's typed in.
    pub fn find_focus(&self) -> FindField {
        if self.project_search.typing {
            self.project_search.fields.focus
        } else {
            self.find.focus
        }
    }

    /// `<Space>/` (and `<Space>h`, with `replace`): opens the find box with
    /// focus in its find field, seeded with the visual selection (one
    /// line), or the word under the cursor if nothing was found before, or
    /// else what was; selected, so typing replaces it. Any other visual
    /// selection (lines, a block, chars across lines) is where it finds,
    /// with "in selection" on.
    pub fn open_find(&mut self, replace: bool) {
        // The find box's keys are its own again (not the Search view's).
        self.project_search.typing = false;
        let one_line = matches!(
            self.selection_range(),
            Some(crate::core::Range::Chars { start, end }) if start.row == end.row
        );
        self.find.scope = (self.mode() == Mode::Visual && !one_line)
            .then(|| self.selection_scope())
            .flatten();
        self.find.in_selection = self.find.scope.is_some();
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
        // Not the word under the cursor, at a selection's end.
        let scoped = self.find.in_selection;
        let seed = selected.or_else(|| {
            (self.find.find.text().is_empty() && !scoped)
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
            _ if scoped => self.find.scope.as_ref().expect("scoped").start(),
            Some(crate::core::Range::Chars { start, .. }) if self.mode() == Mode::Visual => start,
            // The word the cursor's in, so it's the first match.
            _ if seeded_word => self.word_start(),
            _ => self.document.cursor(),
        };
        self.find_history_reset();
        self.set_mode(Mode::Find);
        self.find.replacing |= replace;
        self.find.focus = FindField::Find;
        self.find.origin = origin;
        self.find_as_typed();
    }

    /// The scope the visual selection makes, or else the last selection
    /// (`gv`'s).
    fn selection_scope(&self) -> Option<FindScope> {
        let (selection, cursor, to_eol) = self.selection_or_last()?;
        if selection.kind != super::SelectionKind::Block {
            return Some(FindScope::of_range(
                selection.range(self.document.buffer(), cursor),
            ));
        }
        let block = crate::core::Block::between(
            (selection.anchor.row, selection.anchor.col),
            (cursor.row, cursor.col),
            to_eol,
            |row| self.line_cells(row),
        );
        let spans = block
            .rows()
            .map(|row| {
                let (start, end) = block.chars_in(&self.line_cells(row));
                (start, if to_eol { usize::MAX } else { end })
            })
            .collect();
        Some(FindScope {
            first: block.first,
            spans,
        })
    }

    /// `<A-l>` (`<C-l>`): finds only in the selection the box opened from
    /// (or the last one, `gv`'s), or everywhere again. In the Search view's
    /// fields it does nothing: that searches files.
    pub fn find_toggle_in_selection(&mut self) -> Result<(), String> {
        if self.project_search.typing {
            return Ok(());
        }
        if !self.find.in_selection && self.find.scope.is_none() {
            self.find.scope = Some(self.selection_scope().ok_or("No selection to find in")?);
        }
        self.find.in_selection = !self.find.in_selection;
        if self.find.in_selection {
            // From its start, so its first match is the one found.
            self.find.origin = self.find.scope.as_ref().expect("just set").start();
        }
        self.find_as_typed();
        Ok(())
    }

    /// The selection the find box finds in, while it's shown with "in
    /// selection" on (drawn behind the text).
    pub fn find_scope(&self) -> Option<&FindScope> {
        if self.mode() != Mode::Find || self.project_search.typing {
            return None;
        }
        self.find.scope()
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

    /// `<Esc>`: closes the box, the cursor left at the match, and its
    /// highlights cleared. Its search becomes the last one, for `n` and
    /// `N` (which highlight it again).
    pub fn close_find(&mut self) {
        if self.project_search.typing {
            self.search_leave();
            return;
        }
        if let Some(pattern) = self.find.pattern().cloned() {
            self.set_last_search(pattern);
        }
        let found = self.find.find.text().to_string();
        self.command_line.search_history_mut().push(&found);
        self.clear_search_highlight();
        self.find.replacing = false;
        self.find.in_selection = false;
        self.find.scope = None;
        self.set_mode(Mode::Normal);
    }

    /// The match the cursor's on, while the box has focus (drawn apart
    /// from the others).
    pub fn find_current_match(&self) -> Option<Match> {
        if self.mode() != Mode::Find || self.project_search.typing {
            return None;
        }
        let pattern = self.find.pattern()?;
        let cursor = self.document.cursor();
        find_match(
            pattern,
            self.document.buffer(),
            self.find.scope(),
            cursor,
            true,
            true,
        )
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
        let target = self.find.pattern().and_then(|p| {
            find_match(
                p,
                self.document.buffer(),
                self.find.scope(),
                origin,
                true,
                true,
            )
        });
        self.document
            .set_cursor(target.map_or(origin, |m| m.start_position()));
        self.reveal_cursor();
    }

    /// Edits the field with focus (`edit` is one of `TextField`'s), finding
    /// again if it's the find field.
    pub fn find_edit(&mut self, edit: impl FnOnce(&mut TextField)) {
        if self.project_search.typing {
            edit(self.project_search.fields.field());
            // The query searches again; the replacement only changes the
            // preview.
            if self.project_search.fields.focus == FindField::Find {
                self.search_changed();
            }
            return;
        }
        self.find_history_reset();
        edit(self.find.field());
        if self.find.focus == FindField::Find {
            self.find_as_typed();
        }
    }

    /// `<Up>` (`older`) / `<Down>`: the field with focus shows an older or
    /// newer entry of its history, the find field finding it. In the
    /// Search view's field, they go to the results.
    pub fn find_history(&mut self, older: bool) {
        if self.project_search.typing {
            self.search_focus_results();
            return;
        }
        let current = self.find.field().text().to_string();
        let history = match self.find.focus {
            FindField::Find => self.command_line.search_history_mut(),
            FindField::Replace => &mut self.find.replace_history,
        };
        let entry = if older {
            history.older_any(&current)
        } else {
            history.newer()
        };
        let Some(entry) = entry else { return };
        self.find.field().set_text(&entry);
        if self.find.focus == FindField::Find {
            self.find_as_typed();
        }
    }

    /// Stops browsing either field's history.
    fn find_history_reset(&mut self) {
        self.command_line.search_history_mut().reset();
        self.find.replace_history.reset();
    }

    /// Replacing: both fields' text into their histories.
    fn find_remember_replace(&mut self) {
        let (found, replaced) = (
            self.find.find.text().to_string(),
            self.find.replace.text().to_string(),
        );
        self.command_line.search_history_mut().push(&found);
        self.find.replace_history.push(&replaced);
    }

    /// A click in a field: focus there, with the cursor at char `col`.
    pub fn find_click_field(&mut self, field: FindField, col: usize) {
        if field == FindField::Replace {
            self.find.replacing = true;
        }
        self.find_history_reset();
        self.find.focus = field;
        self.find.field().set_cursor(col);
    }

    /// `<Tab>`: between the find and replace fields (showing the replace
    /// field).
    pub fn find_switch_field(&mut self) {
        if self.project_search.typing {
            // The query, then the replacement, then the results.
            match self.project_search.fields.focus {
                FindField::Find => self.project_search.fields.focus = FindField::Replace,
                FindField::Replace => {
                    self.project_search.fields.focus = FindField::Find;
                    self.search_focus_results();
                }
            }
            return;
        }
        self.find.replacing = true;
        self.find_history_reset();
        self.find.focus = match self.find.focus {
            FindField::Find => FindField::Replace,
            FindField::Replace => FindField::Find,
        };
    }

    /// `<A-p>` (`<C-t>`): flips preserve case. Only what matches become
    /// changes, so it doesn't find (or search) again.
    pub fn find_toggle_preserve_case(&mut self) {
        let fields = if self.project_search.typing {
            &mut self.project_search.fields
        } else {
            &mut self.find
        };
        fields.preserve_case = !fields.preserve_case;
    }

    /// `<A-c>`, `<A-w>`, `<A-r>`: flips match case, whole word or regex.
    pub fn find_toggle(&mut self, toggle: impl FnOnce(&mut FindState)) {
        if self.project_search.typing {
            toggle(&mut self.project_search.fields);
            self.search_changed();
            return;
        }
        toggle(&mut self.find);
        self.find_as_typed();
    }

    /// `<CR>` / `<Down>` (or `<S-CR>` / `<Up>`): the next (or previous)
    /// match; finding as you type carries on from there.
    pub fn find_step(&mut self, forward: bool) -> Result<(), String> {
        if self.project_search.typing {
            self.search_focus_results();
            return Ok(());
        }
        let pattern = self.find_pattern()?;
        let cursor = self.document.cursor();
        let found = find_match(
            &pattern,
            self.document.buffer(),
            self.find.scope(),
            cursor,
            forward,
            false,
        )
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
        if self.project_search.typing {
            return self.search_ask_replace_all();
        }
        let pattern = self.find_pattern()?;
        let Some(current) = self.find_current_match() else {
            return self.find_step(true);
        };
        self.find_remember_replace();
        let line = self.document.buffer().line(current.row).into_owned();
        let (start, end, text) = self
            .find
            .replacement()
            .next(&pattern, &line, current.start)
            .filter(|&(start, end, _)| {
                self.find
                    .scope()
                    .is_none_or(|s| s.contains(current.row, start, end))
            })
            .filter(|&(start, ..)| start == current.start)
            .ok_or("The match changed")?;
        let at = Position::new(current.row, start);
        self.document.delete(at, Position::new(current.row, end));
        let after = self.document.insert(at, &text);
        match &mut self.find.scope {
            Some(scope) if self.find.in_selection => {
                scope.replaced(current.row, end - start, text.chars().count());
            }
            // Off, it doesn't follow the text, so it goes.
            _ => self.find.scope = None,
        }
        // The next match after what was put in (never inside it).
        let next = find_match(
            &pattern,
            self.document.buffer(),
            self.find.scope(),
            after,
            true,
            true,
        );
        let target = next.map_or(at, |m| m.start_position());
        self.document.set_cursor(target);
        self.find.origin = target;
        self.reveal_cursor();
        Ok(())
    }

    /// `<A-a>`: replaces every match in the buffer (or in the selection,
    /// with "in selection" on), as one undo step.
    pub fn find_replace_all(&mut self) -> Result<(), String> {
        if self.project_search.typing {
            return self.search_ask_replace_all();
        }
        let pattern = self.find_pattern()?;
        self.find_remember_replace();
        let replacement = self.find.replacement();
        let mut scope = self.find.scope().cloned();
        let mut replaced = 0;
        let mut lines = 0;
        let line_count = self.document.buffer().line_count();
        let rows = scope.as_ref().map_or(0..line_count, |s| {
            s.rows().start.min(line_count)..s.rows().end.min(line_count)
        });
        // Bottom-up, so lines put in don't move the rows still to do.
        for row in rows.rev() {
            let line = self.document.buffer().line(row).into_owned();
            let (text, count) = match scope.as_mut() {
                Some(scope) => {
                    let span = scope.span(row).expect("a row of the scope");
                    let (text, count) = replacement.span(&pattern, &line, span);
                    let len = |s: &str| s.chars().count();
                    scope.replaced(row, len(&line), len(&text));
                    (text, count)
                }
                None if pattern.matches_in_line(&line).is_empty() => continue,
                None => replacement.line(&pattern, &line),
            };
            if count == 0 {
                continue;
            }
            if text != line {
                let end = self.document.buffer().line_end(row);
                self.document.delete(Position::new(row, 0), end);
                self.document.insert(Position::new(row, 0), &text);
            }
            replaced += count;
            lines += 1;
        }
        // Off, the kept scope doesn't follow the text, so it goes.
        self.find.scope = scope;
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
    fn up_and_down_browse_what_was_searched_for_shared_with_slash() {
        let mut e = editor("alpha beta gamma");
        e.command_line.search_history_mut().push("gam");
        e.open_find(false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "alp");
        e.close_find();
        e.open_find(false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "b");
        assert_eq!(cursor(&e), (0, 6));
        e.find_history(true);
        assert_eq!(
            e.find_state().find.text(),
            "alp",
            "newest first, any prefix"
        );
        assert_eq!(cursor(&e), (0, 0), "and found");
        e.find_history(true);
        assert_eq!(e.find_state().find.text(), "gam", "a / search too");
        assert_eq!(cursor(&e), (0, 11));
        e.find_history(true);
        assert_eq!(e.find_state().find.text(), "gam", "nothing older");
        e.find_history(false);
        e.find_history(false);
        assert_eq!(e.find_state().find.text(), "b", "back to what was typed");
        assert_eq!(cursor(&e), (0, 6));
        e.close_find();
        e.command_line.open(crate::editor::Prompt::SearchForward);
        assert!(e.command_line.history_older());
        assert_eq!(
            e.command_line.input(),
            "b",
            "the box's searches are /'s too"
        );
    }

    #[test]
    fn the_replace_field_browses_what_was_replaced_with() {
        let mut e = editor("a a a");
        e.open_find(true);
        e.find_switch_field();
        e.find_edit(|f| f.push('x'));
        e.find_replace_one().unwrap();
        e.find_edit(|f| f.delete_to_start());
        e.find_edit(|f| f.push('y'));
        e.find_replace_one().unwrap();
        e.find_edit(|f| f.delete_to_start());
        e.find_history(true);
        assert_eq!(e.find_state().replace.text(), "y");
        e.find_history(true);
        assert_eq!(e.find_state().replace.text(), "x");
        assert_eq!(e.find_state().find.text(), "a", "the find field untouched");
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
    fn preserve_case_replaces_each_match_in_its_case() {
        let mut e = editor("foo Foo FOO foo_bar");
        e.open_find(true);
        e.find_toggle_preserve_case();
        e.find_switch_field();
        e.find_edit(|f| "baz".chars().for_each(|c| f.push(c)));
        assert_eq!(cursor(&e), (0, 0));
        e.find_replace_one().unwrap();
        assert_eq!(e.document.buffer().text(), "baz Foo FOO foo_bar");
        e.find_replace_all().unwrap();
        assert_eq!(e.document.buffer().text(), "baz Baz BAZ baz_bar");

        // Off, the replacement goes in as typed.
        let mut e = editor("Foo FOO");
        e.open_find(true);
        e.find_switch_field();
        e.find_edit(|f| "baz".chars().for_each(|c| f.push(c)));
        e.find_replace_all().unwrap();
        assert_eq!(e.document.buffer().text(), "baz baz");
    }

    /// A visual selection of `kind` from `from` to `to`.
    fn select(
        e: &mut Editor,
        kind: super::super::SelectionKind,
        from: (usize, usize),
        to: (usize, usize),
    ) {
        e.document.set_cursor(Position::new(from.0, from.1));
        e.toggle_visual(kind);
        e.document.set_cursor(Position::new(to.0, to.1));
    }

    fn total(e: &Editor) -> usize {
        match e.find_count() {
            FindCount::Found { total, .. } => total,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_selection_across_lines_is_where_it_finds() {
        use super::super::SelectionKind;
        let mut e = editor("ab ab\nab ab\nab ab\nab");
        e.open_find(false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "ab");
        e.close_find();
        // From the middle of row 0 to the first ab of row 2.
        select(&mut e, SelectionKind::Char, (0, 3), (2, 1));
        e.open_find(true);
        assert!(e.find_state().in_selection, "on by itself");
        assert_eq!(
            e.find_state().find.text(),
            "ab",
            "the last search, not the selection"
        );
        assert_eq!(total(&e), 4);
        assert_eq!(cursor(&e), (0, 3), "its first match");
        assert!(e.find_scope().is_some(), "drawn");
        e.find_step(false).unwrap();
        assert_eq!(cursor(&e), (2, 0), "wraps inside it");

        e.find_switch_field();
        e.find_edit(|f| "xyz".chars().for_each(|c| f.push(c)));
        e.find_replace_all().unwrap();
        assert_eq!(e.document.buffer().text(), "ab xyz\nxyz xyz\nxyz ab\nab");
        e.find_edit(|f| f.delete_to_start());
        assert_eq!(total(&e), 0, "replaced, nothing left inside");
        e.find_toggle_in_selection().unwrap();
        assert_eq!(total(&e), 3, "off: the whole file");
        e.find_toggle_in_selection().unwrap();
        assert_eq!(total(&e), 0, "and back on, the same selection");
        e.close_find();
        e.open_find(false);
        assert!(!e.find_state().in_selection, "off again without one");
    }

    #[test]
    fn replacing_one_keeps_the_scope_with_the_text() {
        use super::super::SelectionKind;
        let mut e = editor("a a a a\nb");
        // `v` over "a a a" (cols 0..=4): the last a is outside.
        select(&mut e, SelectionKind::Char, (0, 0), (0, 4));
        e.open_find(true);
        assert!(!e.find_state().in_selection, "one line: the find text");
        e.find_toggle_in_selection().unwrap();
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "a");
        assert_eq!(total(&e), 3);
        e.find_switch_field();
        e.find_edit(|f| "long".chars().for_each(|c| f.push(c)));
        e.find_replace_one().unwrap();
        assert_eq!(cursor(&e), (0, 5), "the next, moved along");
        e.find_replace_one().unwrap();
        e.find_replace_one().unwrap();
        assert_eq!(e.document.buffer().text(), "long long long a\nb");
        assert!(e.find_replace_one().is_err(), "none left inside");
    }

    #[test]
    fn lines_and_blocks() {
        use super::super::SelectionKind;
        let mut e = editor("x1 x2\nx3 x4\nx5 x6");
        select(&mut e, SelectionKind::Line, (1, 0), (2, 0));
        e.open_find(true);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "x");
        assert_eq!(total(&e), 4);

        let mut e = editor("x1 x2\nx3 x4\nx5 x6");
        // The second column, rows 0 and 1.
        select(&mut e, SelectionKind::Block, (0, 3), (1, 4));
        e.open_find(true);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "x");
        assert_eq!(total(&e), 2);
        e.find_switch_field();
        e.find_edit(|f| f.push('y'));
        e.find_replace_all().unwrap();
        assert_eq!(e.document.buffer().text(), "x1 y2\nx3 y4\nx5 x6");
    }

    #[test]
    fn the_toggle_takes_the_last_selection_or_says_there_is_none() {
        use super::super::SelectionKind;
        let mut e = editor("ab\nab\nab");
        e.open_find(false);
        assert_eq!(
            e.find_toggle_in_selection(),
            Err("No selection to find in".into())
        );
        assert!(!e.find_state().in_selection);
        e.close_find();
        select(&mut e, SelectionKind::Line, (1, 0), (2, 0));
        e.set_mode(Mode::Normal);
        e.document.set_cursor(Position::new(0, 0));
        e.open_find(false);
        e.find_toggle_in_selection().unwrap();
        assert_eq!(total(&e), 2, "gv's");
        assert_eq!(cursor(&e), (1, 0));
    }

    #[test]
    fn closing_clears_the_highlights_but_leaves_the_search_for_n() {
        let mut e = editor("ab ab ab");
        e.open_find(false);
        e.find_edit(|f| f.delete_to_start());
        type_find(&mut e, "ab");
        e.find_step(true).unwrap();
        e.close_find();
        assert_eq!(e.mode(), Mode::Normal);
        assert!(e.search_highlight().is_none(), "gone with the box");
        e.search_next(1, false).unwrap();
        assert_eq!(cursor(&e), (0, 6));
        assert!(e.search_highlight().is_some());
    }

    #[test]
    fn find_match_wraps_and_skips_the_start() {
        let buffer = Buffer::from_text("ab\nxab ab");
        let p = SearchPattern::new("ab", CaseMode::Sensitive).unwrap();
        let at = |r, c| Position::new(r, c);
        let found = |from, fwd, inc| {
            find_match(&p, &buffer, None, from, fwd, inc).map(|m| (m.row, m.start))
        };
        assert_eq!(found(at(0, 0), true, true), Some((0, 0)));
        assert_eq!(found(at(0, 0), true, false), Some((1, 1)));
        assert_eq!(found(at(1, 4), true, false), Some((0, 0)), "wraps");
        assert_eq!(found(at(0, 0), false, false), Some((1, 4)), "wraps back");
        let one = Buffer::from_text("ab");
        assert_eq!(
            find_match(&p, &one, None, at(0, 0), true, false).map(|m| m.start),
            Some(0),
            "the only match, all the way round"
        );
    }
}
