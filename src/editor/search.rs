//! Searching from the editor: the `/` and `?` prompts with live preview,
//! `n`/`N`, `*`/`#`, and which pattern to highlight. A search typed after
//! an operator (`d/foo<CR>`) is that operator's motion.

use super::{Editor, Message, Mode, PendingOperator, Prompt, Selection};
use crate::core::Position;
use crate::core::search::{Found, SearchPattern};

#[derive(Debug, Default)]
pub(super) struct SearchState {
    last: Option<LastSearch>,
    /// Where the cursor was when the prompt opened; restored on cancel.
    origin: Option<Position>,
    /// The pattern being typed, compiled, for live highlighting.
    preview: Option<SearchPattern>,
    /// Whether the last search is highlighted (`:noh` turns it off until the
    /// next search).
    highlight: bool,
    /// The operator waiting for the search being typed (`d` of `d/foo`).
    operator: Option<PendingOperator>,
    /// The visual selection the search extends (`v/foo`), as it was when
    /// the prompt opened.
    visual: Option<Selection>,
}

/// Where a search (`n`, `/foo<CR>`) lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub position: Position,
    pub wrapped: bool,
    pub forward: bool,
}

/// What finishing a search prompt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchDone {
    /// The cursor moved to the match.
    Moved,
    /// An operator was waiting: it should now act up to `target`.
    Operator {
        operator: PendingOperator,
        target: Position,
    },
}

#[derive(Debug, Clone)]
struct LastSearch {
    pattern: SearchPattern,
    forward: bool,
}

impl Editor {
    /// `:`: opens the command prompt.
    pub fn open_command_line(&mut self) {
        self.set_mode(Mode::Command);
        self.command_line.open(Prompt::Command);
    }

    /// `/` or `?`: opens a search prompt. A pending operator waits for it;
    /// a visual selection stays, and grows to the match.
    pub fn open_search(&mut self, forward: bool) {
        let operator = self.take_pending_operator();
        let visual = self.selection().filter(|_| self.mode() == Mode::Visual);
        let origin = self.document.cursor();
        self.set_mode(Mode::Command);
        // Still drawn while typing, following the previewed match.
        self.selection = visual;
        self.search.visual = visual;
        let prompt = if forward {
            Prompt::SearchForward
        } else {
            Prompt::SearchBackward
        };
        self.command_line.open(prompt);
        self.search.origin = Some(origin);
        self.search.preview = None;
        self.search.operator = operator;
    }

    /// Leaves a prompt without running it (`<Esc>`): back to visual mode
    /// with the selection as it was if the search started there, or else to
    /// normal mode. The cursor goes back to where the search began.
    pub fn leave_command_line(&mut self) {
        self.return_from_search();
    }

    /// Closes the prompt, back in visual mode if the search began there.
    fn return_from_search(&mut self) {
        match self.search.visual.take() {
            Some(selection) => {
                self.set_mode(Mode::Visual);
                self.selection = Some(selection);
            }
            None => self.set_mode(Mode::Normal),
        }
    }

    /// The operator waiting for the search prompt, if any (for display).
    pub fn search_operator(&self) -> Option<PendingOperator> {
        self.search.operator
    }

    /// Re-runs the search being typed: moves the cursor to the first match
    /// and highlights all matches, or returns to where the search started.
    pub fn update_search_preview(&mut self) {
        let prompt = self.command_line.prompt();
        let Some(origin) = self.search.origin.filter(|_| prompt.is_search()) else {
            return;
        };
        let input = self.command_line.input();
        self.search.preview = (!input.is_empty())
            .then(|| SearchPattern::new(input, self.options.search.case.into()).ok())
            .flatten();

        let target = self.search.preview.as_ref().and_then(|pattern| {
            let forward = prompt == Prompt::SearchForward;
            pattern.find(
                self.document.buffer(),
                origin,
                forward,
                self.options.search.wrap,
            )
        });
        let cursor = target.map_or(origin, |t| t.found.start_position());
        self.document.set_cursor(cursor);
    }

    /// `<CR>` at a search prompt: searches for the typed pattern, or the last
    /// one if nothing was typed. With an operator waiting, the cursor stays
    /// put and the operator's target is returned instead.
    pub fn finish_search(&mut self) -> Result<SearchDone, String> {
        let forward = self.command_line.prompt() == Prompt::SearchForward;
        let input = self.command_line.take();
        let origin = self.search.origin.take().unwrap_or(self.document.cursor());
        let operator = self.search.operator.take();
        self.return_from_search();
        self.document.set_cursor(origin);

        let pattern = if input.is_empty() {
            self.last_search()?.pattern.clone()
        } else {
            SearchPattern::new(&input, self.options.search.case.into())?
        };
        self.search.last = Some(LastSearch { pattern, forward });
        let Some(operator) = operator else {
            self.search_next(1, false)?;
            return Ok(SearchDone::Moved);
        };
        // `d2/foo` deletes up to the second match.
        let hit = self.find_next(operator.count.unwrap_or(1), false)?;
        self.search.highlight = true;
        self.report_search(&hit);
        Ok(SearchDone::Operator {
            operator,
            target: hit.position,
        })
    }

    /// Makes `pattern` the last search (as `:s` does), so `n` finds it and
    /// its matches are highlighted.
    pub fn set_last_search(&mut self, pattern: SearchPattern) {
        self.search.last = Some(LastSearch {
            pattern,
            forward: true,
        });
        self.search.highlight = true;
    }

    /// The last search pattern, if any.
    pub fn last_search_pattern(&self) -> Option<&SearchPattern> {
        self.search.last.as_ref().map(|l| &l.pattern)
    }

    /// `n` (or `N` with `reverse`): jumps to the `count`th next match of the
    /// last search.
    pub fn search_next(&mut self, count: usize, reverse: bool) -> Result<(), String> {
        let hit = self.find_next(count, reverse)?;
        self.search.highlight = true;
        self.push_jump();
        self.document.set_cursor(hit.position);
        // A match in a closed fold opens it, as in Vim.
        self.reveal_cursor();
        self.report_search(&hit);
        Ok(())
    }

    /// Where `n` (or `N`) would go, `count` matches on, without moving.
    pub fn find_next(&self, count: usize, reverse: bool) -> Result<SearchHit, String> {
        let last = self.last_search()?;
        let forward = last.forward != reverse;
        let mut position = self.document.cursor();
        let mut wrapped = false;
        for _ in 0..count.max(1) {
            let Found { found, wrapped: w } = last
                .pattern
                .find(
                    self.document.buffer(),
                    position,
                    forward,
                    self.options.search.wrap,
                )
                .ok_or_else(|| not_found(&last.pattern, self.options.search.wrap))?;
            position = found.start_position();
            wrapped |= w;
        }
        Ok(SearchHit {
            position,
            wrapped,
            forward,
        })
    }

    /// Shows the pattern searched for, or that the search wrapped.
    pub fn report_search(&mut self, hit: &SearchHit) {
        let Some(last) = self.search.last.as_ref() else {
            return;
        };
        let message = if !hit.wrapped {
            let symbol = if hit.forward { '/' } else { '?' };
            format!("{symbol}{}", last.pattern.source())
        } else if hit.forward {
            "Search hit BOTTOM, continuing at TOP".to_string()
        } else {
            "Search hit TOP, continuing at BOTTOM".to_string()
        };
        self.set_message(Message::info(message));
    }

    /// `*` (or `#` backwards): searches for the whole word under (or after)
    /// the cursor.
    pub fn search_word(&mut self, count: usize, forward: bool) -> Result<(), String> {
        let cursor = self.document.cursor();
        let line: Vec<char> = self.document.buffer().line(cursor.row).chars().collect();
        let is_word = |c: char| c.is_alphanumeric() || c == '_';

        let first = (cursor.col..line.len())
            .find(|&i| is_word(line[i]))
            .ok_or("No word under cursor")?;
        let start = (0..first)
            .rev()
            .take_while(|&i| is_word(line[i]))
            .last()
            .unwrap_or(first);
        let end = (first..line.len())
            .take_while(|&i| is_word(line[i]))
            .last()
            .map_or(first, |i| i + 1);
        let word: String = line[start..end].iter().collect();

        self.search.last = Some(LastSearch {
            pattern: SearchPattern::whole_word(&word),
            forward,
        });
        // Search from the word's start, so `#` skips the word itself.
        self.document.set_cursor(cursor.with_col(start));
        self.search_next(count, false)
    }

    /// The word (or with `big`, the run of non-blanks) under or after the
    /// cursor, for `<C-r><C-w>`. While a search prompt previews matches, the
    /// cursor it means is where the search began.
    pub fn word_under_cursor(&self, big: bool) -> Option<String> {
        let cursor = self.search.origin.unwrap_or(self.document.cursor());
        let line: Vec<char> = self.document.buffer().line(cursor.row).chars().collect();
        let in_word = |c: char| {
            if big {
                !c.is_whitespace()
            } else {
                c.is_alphanumeric() || c == '_'
            }
        };
        let first = (cursor.col..line.len()).find(|&i| in_word(line[i]))?;
        let start = (0..first)
            .rev()
            .take_while(|&i| in_word(line[i]))
            .last()
            .unwrap_or(first);
        let end = (first..line.len())
            .take_while(|&i| in_word(line[i]))
            .last()
            .map_or(first, |i| i + 1);
        Some(line[start..end].iter().collect())
    }

    /// `:noh`: hides search highlights until the next search.
    pub fn clear_search_highlight(&mut self) {
        self.search.highlight = false;
    }

    /// The pattern whose matches should be highlighted, if any: the one being
    /// typed, or else the last search.
    pub fn search_highlight(&self) -> Option<&SearchPattern> {
        if self.mode() == Mode::Command && self.command_line.prompt().is_search() {
            return self.search.preview.as_ref();
        }
        if self.options.search.highlight && self.search.highlight {
            return self.search.last.as_ref().map(|l| &l.pattern);
        }
        None
    }

    /// Called by `set_mode` when leaving command mode: a search prompt that
    /// wasn't completed puts the cursor back where it started.
    pub(super) fn end_search_prompt(&mut self) {
        self.search.preview = None;
        // A cancelled `d/` cancels the `d` too.
        self.search.operator = None;
        self.search.visual = None;
        if let Some(origin) = self.search.origin.take() {
            self.document.set_cursor(origin);
        }
    }

    fn last_search(&self) -> Result<&LastSearch, String> {
        self.search
            .last
            .as_ref()
            .ok_or_else(|| "No previous search pattern".to_string())
    }
}

fn not_found(pattern: &SearchPattern, wrap: bool) -> String {
    if wrap {
        format!("Pattern not found: {}", pattern.source())
    } else {
        format!("Search hit the end without match for: {}", pattern.source())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn editor(text: &str) -> Editor {
        Editor::with_document(Document::with_text(text))
    }

    fn type_search(editor: &mut Editor, forward: bool, text: &str) {
        editor.open_search(forward);
        for c in text.chars() {
            editor.insert_text_char(c);
        }
    }

    fn message(editor: &Editor) -> String {
        editor.message().map(|m| m.text.clone()).unwrap_or_default()
    }

    #[test]
    fn preview_moves_cursor_while_typing() {
        let mut e = editor("alpha beta\ngamma beta");
        type_search(&mut e, true, "b");
        assert_eq!(e.document.cursor(), pos(0, 6));
        assert!(e.search_highlight().is_some());
        e.insert_text_char('x');
        assert_eq!(
            e.document.cursor(),
            pos(0, 0),
            "no match: back at the origin"
        );
    }

    #[test]
    fn cancelling_restores_the_cursor() {
        let mut e = editor("alpha beta");
        type_search(&mut e, true, "beta");
        e.set_mode(Mode::Normal);
        assert_eq!(e.document.cursor(), pos(0, 0));
        assert!(e.search_highlight().is_none());
    }

    #[test]
    fn finishing_moves_and_remembers_the_search() {
        let mut e = editor("one two\ntwo three");
        type_search(&mut e, true, "two");
        e.finish_search().unwrap();
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.document.cursor(), pos(0, 4));
        assert_eq!(message(&e), "/two");

        e.search_next(1, false).unwrap();
        assert_eq!(e.document.cursor(), pos(1, 0));
        e.search_next(1, true).unwrap();
        assert_eq!(e.document.cursor(), pos(0, 4));
    }

    #[test]
    fn backward_search_and_wrap_message() {
        let mut e = editor("x one\none");
        type_search(&mut e, false, "one");
        e.finish_search().unwrap();
        assert_eq!(e.document.cursor(), pos(1, 0));
        assert_eq!(message(&e), "Search hit TOP, continuing at BOTTOM");
    }

    #[test]
    fn empty_input_reuses_last_pattern() {
        let mut e = editor("a b a b a");
        type_search(&mut e, true, "b");
        e.finish_search().unwrap();
        type_search(&mut e, true, "");
        e.finish_search().unwrap();
        assert_eq!(e.document.cursor(), pos(0, 6));
    }

    #[test]
    fn errors_for_missing_pattern_and_no_match() {
        let mut e = editor("abc");
        assert_eq!(
            e.search_next(1, false),
            Err("No previous search pattern".into())
        );

        type_search(&mut e, true, "zzz");
        assert_eq!(e.finish_search(), Err("Pattern not found: zzz".into()));
        assert_eq!(e.document.cursor(), pos(0, 0));

        type_search(&mut e, true, "(");
        assert!(
            e.finish_search()
                .unwrap_err()
                .starts_with("Invalid pattern")
        );
    }

    #[test]
    fn no_wrap_option() {
        let mut e = editor("one\ntwo");
        e.options.search.wrap = false;
        e.document.set_cursor(pos(1, 0));
        type_search(&mut e, true, "one");
        let err = e.finish_search().unwrap_err();
        assert!(err.contains("without match"), "{err}");
    }

    #[test]
    fn count_jumps_several_matches() {
        let mut e = editor("x x x x");
        type_search(&mut e, true, "x");
        e.finish_search().unwrap();
        e.search_next(2, false).unwrap();
        assert_eq!(e.document.cursor(), pos(0, 6));
    }

    #[test]
    fn star_and_hash_search_whole_words() {
        let mut e = editor("foo food foo\nfoo");
        e.document.set_cursor(pos(0, 1));
        e.search_word(1, true).unwrap();
        assert_eq!(e.document.cursor(), pos(0, 9), "skips 'food'");
        e.search_word(1, false).unwrap();
        assert_eq!(e.document.cursor(), pos(0, 0));
    }

    #[test]
    fn star_uses_the_next_word_on_the_line() {
        let mut e = editor("  bar = bar");
        e.search_word(1, true).unwrap();
        assert_eq!(e.document.cursor(), pos(0, 8));
        let mut e = editor("   ");
        assert_eq!(e.search_word(1, true), Err("No word under cursor".into()));
    }

    #[test]
    fn noh_hides_highlight_until_next_search() {
        let mut e = editor("a a");
        type_search(&mut e, true, "a");
        e.finish_search().unwrap();
        assert!(e.search_highlight().is_some());
        e.clear_search_highlight();
        assert!(e.search_highlight().is_none());
        e.search_next(1, false).unwrap();
        assert!(e.search_highlight().is_some());
    }

    #[test]
    fn highlight_option_disables_it() {
        let mut e = editor("a a");
        e.options.search.highlight = false;
        type_search(&mut e, true, "a");
        assert!(
            e.search_highlight().is_some(),
            "still previews while typing"
        );
        e.finish_search().unwrap();
        assert!(e.search_highlight().is_none());
    }

    #[test]
    fn a_waiting_operator_gets_the_target_instead_of_a_move() {
        use crate::editor::Operator;
        let mut e = editor("alpha beta beta");
        e.begin_operator(Operator::Delete, Some(2), None);
        type_search(&mut e, true, "beta");
        assert_eq!(
            e.search_operator().map(|o| o.operator),
            Some(Operator::Delete)
        );
        let done = e.finish_search().unwrap();
        let SearchDone::Operator { operator, target } = done else {
            panic!("expected an operator target, got {done:?}");
        };
        assert_eq!(operator.count, Some(2));
        assert_eq!(target, pos(0, 11), "the second match");
        assert_eq!(e.document.cursor(), pos(0, 0), "the cursor stays put");
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn cancelling_drops_the_waiting_operator() {
        use crate::editor::Operator;
        let mut e = editor("alpha beta");
        e.begin_operator(Operator::Delete, None, None);
        type_search(&mut e, true, "beta");
        e.set_mode(Mode::Normal);
        assert!(e.search_operator().is_none());
        assert!(e.pending_operator().is_none());
    }

    #[test]
    fn find_next_doesnt_move() {
        let mut e = editor("a b a b");
        type_search(&mut e, true, "b");
        e.finish_search().unwrap();
        assert_eq!(e.document.cursor(), pos(0, 2));
        let hit = e.find_next(1, false).unwrap();
        assert_eq!(hit.position, pos(0, 6));
        assert!(hit.forward && !hit.wrapped);
        assert_eq!(e.document.cursor(), pos(0, 2));
    }
}
