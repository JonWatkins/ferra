//! `:s/pattern/replacement/c`: asking before each replacement.
//!
//! The session moves from match to match. The command layer asks for each
//! answer as the next key (`y`, `n`, `a`, `q`, `l`), so there's no special
//! mode. While a session is open the undo step stays open, so the whole
//! substitution undoes at once.

use std::collections::BTreeSet;

use super::{Editor, Message};
use crate::core::Position;
use crate::core::search::SearchPattern;

#[derive(Debug, Clone)]
pub struct ConfirmSession {
    pattern: SearchPattern,
    /// In `regex` syntax.
    replacement: String,
    /// `g`: every match on a line, not just the first.
    global: bool,
    last_row: usize,
    /// Where to look for the next match.
    from: Position,
    /// The match being asked about: row, start and end columns, and the
    /// text that would replace it.
    current: Option<(usize, usize, usize, String)>,
    replaced: usize,
    lines: BTreeSet<usize>,
}

impl Editor {
    /// Starts asking about each match in `first..=last`. Fails if there
    /// are none.
    pub fn begin_confirm(
        &mut self,
        pattern: SearchPattern,
        replacement: String,
        global: bool,
        (first, last): (usize, usize),
    ) -> Result<(), String> {
        let source = pattern.source().to_string();
        self.set_last_search(pattern.clone());
        let mut session = ConfirmSession {
            pattern,
            replacement,
            global,
            last_row: last,
            from: Position::new(first, 0),
            current: None,
            replaced: 0,
            lines: BTreeSet::new(),
        };
        if !self.find_next_confirm(&mut session) {
            return Err(format!("Pattern not found: {source}"));
        }
        self.confirm = Some(session);
        self.show_confirm_question();
        Ok(())
    }

    pub fn is_confirming(&self) -> bool {
        self.confirm.is_some()
    }

    /// Handles an answer. Returns whether there's another match to ask
    /// about. Keys other than the five answers ask again.
    pub fn answer_confirm(&mut self, answer: char) -> bool {
        let Some(mut session) = self.confirm.take() else {
            return false;
        };
        let more = match answer {
            'y' => {
                self.replace_current(&mut session);
                self.find_next_confirm(&mut session)
            }
            'n' => {
                self.skip_current(&mut session);
                self.find_next_confirm(&mut session)
            }
            'a' => {
                loop {
                    self.replace_current(&mut session);
                    if !self.find_next_confirm(&mut session) {
                        break;
                    }
                }
                false
            }
            'l' => {
                self.replace_current(&mut session);
                false
            }
            'q' => false,
            _ => true,
        };
        if more {
            self.confirm = Some(session);
            self.show_confirm_question();
        } else {
            self.finish_confirm(&session);
        }
        more
    }

    /// Stops asking (`<Esc>`), keeping what was replaced so far.
    pub fn cancel_confirm(&mut self) {
        if let Some(session) = self.confirm.take() {
            self.finish_confirm(&session);
        }
    }

    fn show_confirm_question(&mut self) {
        let Some(session) = &self.confirm else {
            return;
        };
        let Some((row, start, _, text)) = &session.current else {
            return;
        };
        let shown = text.replace('\n', "⏎");
        let question = format!("replace with {shown} (y/n/a/q/l)?");
        self.document.set_cursor(Position::new(*row, *start));
        self.set_message(Message::info(question));
    }

    /// Finds the next match from `session.from`, within the range.
    fn find_next_confirm(&self, session: &mut ConfirmSession) -> bool {
        session.current = None;
        let buffer = self.document.buffer();
        let last = session.last_row.min(buffer.line_count() - 1);
        let mut from = session.from;
        while from.row <= last {
            let line = buffer.line(from.row);
            if let Some((start, end, text)) =
                session
                    .pattern
                    .next_replacement(&line, from.col, &session.replacement)
            {
                session.current = Some((from.row, start, end, text));
                return true;
            }
            from = Position::new(from.row + 1, 0);
        }
        false
    }

    fn replace_current(&mut self, session: &mut ConfirmSession) {
        let Some((row, start, end, text)) = session.current.take() else {
            return;
        };
        let doc = &mut self.document;
        doc.delete(Position::new(row, start), Position::new(row, end));
        let after = doc.insert(Position::new(row, start), &text);
        session.last_row += after.row - row;
        session.replaced += 1;
        session.lines.insert(row);
        session.from = if !session.global {
            Position::new(after.row + 1, 0)
        } else if start == end && text.is_empty() {
            // An empty match replaced with nothing: step past it.
            after.with_col(after.col + 1)
        } else {
            after
        };
    }

    fn skip_current(&mut self, session: &mut ConfirmSession) {
        let Some((row, start, end, _)) = session.current.take() else {
            return;
        };
        session.from = if !session.global {
            Position::new(row + 1, 0)
        } else {
            Position::new(row, end.max(start + 1))
        };
    }

    fn finish_confirm(&mut self, session: &ConfirmSession) {
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        self.set_message(Message::info(format!(
            "{} on {}",
            plural(session.replaced, "substitution"),
            plural(session.lines.len(), "line")
        )));
        self.document.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::search::CaseMode;
    use crate::editor::Document;

    fn session(text: &str, pattern: &str, replacement: &str, global: bool) -> Editor {
        let mut e = Editor::with_document(Document::with_text(text));
        let pattern = SearchPattern::new(pattern, CaseMode::Sensitive).unwrap();
        let last = e.document.buffer().line_count() - 1;
        e.begin_confirm(pattern, replacement.into(), global, (0, last))
            .unwrap();
        e
    }

    fn text(e: &Editor) -> String {
        e.document.buffer().text()
    }

    #[test]
    fn yes_and_no_one_at_a_time() {
        let mut e = session("a a\na", "a", "b", true);
        assert_eq!(e.document.cursor(), Position::new(0, 0));
        assert!(e.answer_confirm('y'));
        assert_eq!(e.document.cursor(), Position::new(0, 2));
        assert!(e.answer_confirm('n'));
        assert!(!e.answer_confirm('y'));
        assert_eq!(text(&e), "b a\nb");
        assert!(!e.is_confirming());
        assert_eq!(e.message().unwrap().text, "2 substitutions on 2 lines");
    }

    #[test]
    fn without_g_only_the_first_match_per_line() {
        let mut e = session("a a\na a", "a", "b", false);
        assert!(e.answer_confirm('y'));
        assert_eq!(e.document.cursor(), Position::new(1, 0));
        assert!(!e.answer_confirm('y'));
        assert_eq!(text(&e), "b a\nb a");
    }

    #[test]
    fn all_last_and_quit() {
        let mut e = session("a a a", "a", "b", true);
        assert!(!e.answer_confirm('a'));
        assert_eq!(text(&e), "b b b");

        let mut e = session("a a a", "a", "b", true);
        assert!(!e.answer_confirm('l'));
        assert_eq!(text(&e), "b a a");

        let mut e = session("a a a", "a", "b", true);
        assert!(e.answer_confirm('x'), "other keys ask again");
        assert!(!e.answer_confirm('q'));
        assert_eq!(text(&e), "a a a");
    }

    #[test]
    fn captures_and_line_breaks_in_replacements() {
        let mut e = session("k=v\nx=y", "(\\w)=(\\w)", "${2}:${1}\n", true);
        assert!(e.answer_confirm('y'));
        assert_eq!(
            e.document.cursor(),
            Position::new(2, 0),
            "next line moved down"
        );
        assert!(!e.answer_confirm('y'));
        assert_eq!(text(&e), "v:k\n\ny:x\n");
    }

    #[test]
    fn replacements_containing_the_pattern_are_not_asked_again() {
        let mut e = session("a", "a", "aa", true);
        assert!(!e.answer_confirm('y'));
        assert_eq!(text(&e), "aa");
    }

    #[test]
    fn no_match_is_an_error() {
        let mut e = Editor::with_document(Document::with_text("abc"));
        let pattern = SearchPattern::new("z", CaseMode::Sensitive).unwrap();
        let err = e.begin_confirm(pattern, "y".into(), false, (0, 0));
        assert_eq!(err.unwrap_err(), "Pattern not found: z");
        assert!(!e.is_confirming());
    }

    #[test]
    fn the_session_is_one_undo_step() {
        let mut e = session("a a", "a", "b", true);
        e.answer_confirm('y');
        e.finish_command();
        e.answer_confirm('y');
        e.finish_command();
        assert_eq!(text(&e), "b b");
        e.document.undo();
        assert_eq!(text(&e), "a a");
    }
}
