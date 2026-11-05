//! Text search: compiling a pattern and finding matches in a [`Buffer`].
//!
//! Patterns use Rust's `regex` syntax, plus Vim's `\<` and `\>` word
//! boundaries (both become `\b`). Matches don't span lines.

use regex::{Regex, RegexBuilder};

use super::{Buffer, Position};

/// How letter case is treated when matching.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    /// Case-insensitive unless the pattern contains an uppercase letter.
    #[default]
    Smart,
    Sensitive,
    Insensitive,
}

/// A match on one line: columns `start..end` (in chars) of `row`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

impl Match {
    pub fn start_position(&self) -> Position {
        Position::new(self.row, self.start)
    }
}

/// A search result: the match, and whether the search wrapped past the
/// end (or start) of the buffer to find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub found: Match,
    pub wrapped: bool,
}

#[derive(Debug, Clone)]
pub struct SearchPattern {
    source: String,
    regex: Regex,
}

impl SearchPattern {
    pub fn new(pattern: &str, case: CaseMode) -> Result<Self, String> {
        let ignore_case = match case {
            CaseMode::Sensitive => false,
            CaseMode::Insensitive => true,
            CaseMode::Smart => !pattern.chars().any(char::is_uppercase),
        };
        let translated = pattern.replace(r"\<", r"\b").replace(r"\>", r"\b");
        let regex = RegexBuilder::new(&translated)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|e| format!("Invalid pattern: {}", last_line(&e.to_string())))?;
        Ok(Self {
            source: pattern.to_string(),
            regex,
        })
    }

    /// A pattern matching `word` exactly, as a whole word (for `*` and `#`).
    pub fn whole_word(word: &str) -> Self {
        let pattern = format!(r"\<{}\>", regex::escape(word));
        Self::new(&pattern, CaseMode::Sensitive).expect("escaped word is a valid pattern")
    }

    /// The pattern as typed, for messages and history.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Non-empty matches in `line`, as `(start, end)` char columns.
    pub fn matches_in_line(&self, line: &str) -> Vec<(usize, usize)> {
        let mut cols = CharCols::new(line);
        self.regex
            .find_iter(line)
            .filter(|m| !m.is_empty())
            .map(|m| (cols.col(m.start()), cols.col(m.end())))
            .collect()
    }

    /// The next match after `from` (or before it, searching backwards),
    /// wrapping around the buffer if `wrap` is set.
    pub fn find(
        &self,
        buffer: &Buffer,
        from: Position,
        forward: bool,
        wrap: bool,
    ) -> Option<Found> {
        let rows = buffer.line_count();
        // Visit every row once, starting at the cursor's, then the cursor's
        // row again for matches on the far side of the cursor after wrapping.
        for step in 0..=rows {
            let (row, wrapped) = if forward {
                let row = from.row + step;
                (row % rows, row >= rows)
            } else {
                let row = from.row as isize - step as isize;
                (row.rem_euclid(rows as isize) as usize, row < 0)
            };
            if wrapped && !wrap {
                return None;
            }

            let starts = self.match_starts(&buffer.line(row));
            let candidate = if step == 0 {
                if forward {
                    starts.into_iter().find(|&(s, _)| s > from.col)
                } else {
                    starts.into_iter().rev().find(|&(s, _)| s < from.col)
                }
            } else if forward {
                starts.into_iter().next()
            } else {
                starts.into_iter().next_back()
            };

            if let Some((start, end)) = candidate {
                return Some(Found {
                    found: Match { row, start, end },
                    wrapped,
                });
            }
        }
        None
    }

    /// All match starts on a line, including empty matches (so `^` works).
    fn match_starts(&self, line: &str) -> Vec<(usize, usize)> {
        let mut cols = CharCols::new(line);
        self.regex
            .find_iter(line)
            .map(|m| (cols.col(m.start()), cols.col(m.end())))
            .collect()
    }
}

/// Converts increasing byte offsets in a line to char columns.
struct CharCols<'a> {
    line: &'a str,
    byte: usize,
    col: usize,
}

impl<'a> CharCols<'a> {
    fn new(line: &'a str) -> Self {
        Self {
            line,
            byte: 0,
            col: 0,
        }
    }

    fn col(&mut self, byte: usize) -> usize {
        if byte < self.byte {
            self.byte = 0;
            self.col = 0;
        }
        self.col += self.line[self.byte..byte].chars().count();
        self.byte = byte;
        self.col
    }
}

fn last_line(text: &str) -> &str {
    text.lines().last().unwrap_or(text).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn pattern(p: &str) -> SearchPattern {
        SearchPattern::new(p, CaseMode::Smart).unwrap()
    }

    fn found_at(found: Option<Found>) -> Option<(usize, usize, bool)> {
        found.map(|f| (f.found.row, f.found.start, f.wrapped))
    }

    #[test]
    fn smart_case() {
        let b = Buffer::from_text("Foo foo");
        assert_eq!(pattern("foo").matches_in_line(&b.line(0)).len(), 2);
        assert_eq!(pattern("Foo").matches_in_line(&b.line(0)), vec![(0, 3)]);
    }

    #[test]
    fn explicit_case_modes() {
        let p = SearchPattern::new("foo", CaseMode::Sensitive).unwrap();
        assert_eq!(p.matches_in_line("Foo foo"), vec![(4, 7)]);
        let p = SearchPattern::new("Foo", CaseMode::Insensitive).unwrap();
        assert_eq!(p.matches_in_line("Foo foo").len(), 2);
    }

    #[test]
    fn vim_word_boundaries() {
        assert_eq!(
            pattern(r"\<is\>").matches_in_line("this is it"),
            vec![(5, 7)]
        );
    }

    #[test]
    fn whole_word_escapes_regex_characters() {
        let p = SearchPattern::whole_word("a.b");
        assert_eq!(p.matches_in_line("a.b axb"), vec![(0, 3)]);
    }

    #[test]
    fn invalid_patterns_are_errors() {
        let err = SearchPattern::new("(", CaseMode::Smart).unwrap_err();
        assert!(err.starts_with("Invalid pattern"), "{err}");
    }

    #[test]
    fn match_columns_are_chars_not_bytes() {
        assert_eq!(pattern("x").matches_in_line("éé x"), vec![(3, 4)]);
    }

    #[test]
    fn find_forward_skips_match_under_cursor() {
        let b = Buffer::from_text("ab ab\nab");
        let p = pattern("ab");
        assert_eq!(
            found_at(p.find(&b, pos(0, 0), true, true)),
            Some((0, 3, false))
        );
        assert_eq!(
            found_at(p.find(&b, pos(0, 3), true, true)),
            Some((1, 0, false))
        );
    }

    #[test]
    fn find_forward_wraps() {
        let b = Buffer::from_text("ab\nxx");
        let p = pattern("ab");
        assert_eq!(
            found_at(p.find(&b, pos(1, 0), true, true)),
            Some((0, 0, true))
        );
        assert_eq!(
            found_at(p.find(&b, pos(0, 0), true, true)),
            Some((0, 0, true)),
            "only match"
        );
        assert_eq!(p.find(&b, pos(1, 0), true, false), None, "no wrap");
    }

    #[test]
    fn find_backward() {
        let b = Buffer::from_text("ab ab\nab");
        let p = pattern("ab");
        assert_eq!(
            found_at(p.find(&b, pos(0, 3), false, true)),
            Some((0, 0, false))
        );
        assert_eq!(
            found_at(p.find(&b, pos(0, 0), false, true)),
            Some((1, 0, true))
        );
        assert_eq!(
            found_at(p.find(&b, pos(1, 0), false, true)),
            Some((0, 3, false))
        );
    }

    #[test]
    fn no_match_anywhere() {
        let b = Buffer::from_text("abc");
        assert_eq!(pattern("z").find(&b, pos(0, 0), true, true), None);
    }

    #[test]
    fn empty_matches_are_found_but_not_highlighted() {
        let b = Buffer::from_text("a\nb");
        let p = pattern("^");
        assert_eq!(
            found_at(p.find(&b, pos(0, 0), true, true)),
            Some((1, 0, false))
        );
        assert!(p.matches_in_line("a").is_empty());
    }
}
