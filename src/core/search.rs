//! Text search: compiling a pattern and finding matches in a [`Buffer`].
//!
//! Patterns use Rust's `regex` syntax, plus Vim's `\<` and `\>` word
//! boundaries (both become `\b`).
//!
//! A pattern that can match a line break spans lines (`spans_lines`): one
//! with a line break in it, or `\n`, `\r` or `\W` (VS Code's rule, so
//! `\s` and `[^x]` don't start matching across lines). Those are matched
//! against the text of whole rows (`matches_between`, `find`), with `^` and
//! `$` at each line's start and end; any other pattern line by line, which
//! needs no copy of the text. `:s` replaces line by line, so a pattern that
//! spans lines finds nothing there.

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

/// A match: chars `start..end`, on one row or running across several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: Position,
    pub end: Position,
}

impl Match {
    /// A match of chars `start..end` on `row`.
    pub fn on_row(row: usize, start: usize, end: usize) -> Self {
        Self {
            start: Position::new(row, start),
            end: Position::new(row, end),
        }
    }

    pub fn start_position(&self) -> Position {
        self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Its chars on `row` (`len` chars long), if it's on it: running on to
    /// `len + 1` (the line break) on a row it carries on past.
    pub fn cols_on(&self, row: usize, len: usize) -> Option<(usize, usize)> {
        if row < self.start.row || row > self.end.row {
            return None;
        }
        let from = if row == self.start.row {
            self.start.col
        } else {
            0
        };
        let to = if row == self.end.row {
            self.end.col
        } else {
            len + 1
        };
        Some((from, to))
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
    /// Whether it can match a line break.
    spans_lines: bool,
}

impl SearchPattern {
    pub fn new(pattern: &str, case: CaseMode) -> Result<Self, String> {
        let ignore_case = match case {
            CaseMode::Sensitive => false,
            CaseMode::Insensitive => true,
            CaseMode::Smart => !pattern.chars().any(char::is_uppercase),
        };
        let translated = pattern.replace(r"\<", r"\b").replace(r"\>", r"\b");
        Self::from_regex(&translated, pattern, ignore_case)
    }

    /// A pattern from a regex in `regex` syntax as it is (no `\<` `\>`
    /// translation), shown as `source`: the find box builds these.
    pub fn from_regex(regex: &str, source: &str, ignore_case: bool) -> Result<Self, String> {
        let spans_lines = can_match_line_break(regex);
        let regex = RegexBuilder::new(regex)
            .case_insensitive(ignore_case)
            .multi_line(true)
            .build()
            .map_err(|e| format!("Invalid pattern: {}", last_line(&e.to_string())))?;
        Ok(Self {
            source: source.to_string(),
            regex,
            spans_lines,
        })
    }

    /// Whether it's matched across lines (see the module docs).
    pub fn spans_lines(&self) -> bool {
        self.spans_lines
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

    /// The first match in `line` starting at or after char column `from`
    /// (empty matches included), as `(start, end)` char columns, with
    /// `replacement` (`regex` syntax) expanded for it. Anchors like `^`
    /// still see the whole line.
    pub fn next_replacement(
        &self,
        line: &str,
        from: usize,
        replacement: &str,
    ) -> Option<(usize, usize, String)> {
        let byte_from = match line.char_indices().nth(from) {
            Some((i, _)) => i,
            None if from == line.chars().count() => line.len(),
            None => return None,
        };
        let captures = self.regex.captures_at(line, byte_from)?;
        let found = captures.get(0)?;
        let mut text = String::new();
        captures.expand(replacement, &mut text);
        let start = line[..found.start()].chars().count();
        let end = start + found.as_str().chars().count();
        Some((start, end, text))
    }

    /// Whether `line` has a match.
    pub fn is_match(&self, line: &str) -> bool {
        self.regex.is_match(line)
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

    /// Every match (empty ones too) starting in rows `rows`: in the text of
    /// those rows if the pattern spans lines (so a match running past the
    /// last one is cut short), else line by line.
    pub fn matches_between(&self, buffer: &Buffer, rows: std::ops::Range<usize>) -> Vec<Match> {
        let rows = rows.start.min(buffer.line_count())..rows.end.min(buffer.line_count());
        if rows.is_empty() {
            return Vec::new();
        }
        if !self.spans_lines {
            return rows
                .flat_map(|row| {
                    self.match_starts(&buffer.line(row))
                        .into_iter()
                        .map(move |(start, end)| Match::on_row(row, start, end))
                })
                .collect();
        }
        let text = buffer.lines_text(rows.start, rows.end - 1);
        let base = buffer.byte_of(Position::new(rows.start, 0));
        let at = |byte: usize| buffer.position_at_byte(base + byte);
        self.regex
            .find_iter(&text)
            .map(|m| Match {
                start: at(m.start()),
                end: at(m.end()),
            })
            .collect()
    }

    /// The first match in `text` starting at or after byte `from` (empty
    /// ones too), as a byte range, with `replacement` expanded for it.
    pub fn replacement_at(
        &self,
        text: &str,
        from: usize,
        replacement: &str,
    ) -> Option<(std::ops::Range<usize>, String)> {
        let captures = self.regex.captures_at(text, from)?;
        let found = captures.get(0)?;
        let mut expanded = String::new();
        captures.expand(replacement, &mut expanded);
        Some((found.range(), expanded))
    }

    /// Replaces the first match in `line` (or every match, with `all`),
    /// with `replacement` in `regex` syntax (`$0`, `${1}`). Returns the new
    /// text and how many matches were replaced.
    pub fn replace(&self, line: &str, replacement: &str, all: bool) -> (String, usize) {
        self.replace_mapped(line, replacement, all, |_, text| text)
    }

    /// As `replace`, with each match's expanded replacement passed through
    /// `map` (given the text matched) first: the find box's preserve case.
    pub fn replace_mapped(
        &self,
        line: &str,
        replacement: &str,
        all: bool,
        mut map: impl FnMut(&str, String) -> String,
    ) -> (String, usize) {
        let limit = if all { 0 } else { 1 };
        let count = self
            .regex
            .find_iter(line)
            .take(if all { usize::MAX } else { 1 })
            .count();
        let replaced = self
            .regex
            .replacen(line, limit, |captures: &regex::Captures| {
                let mut text = String::new();
                captures.expand(replacement, &mut text);
                map(&captures[0], text)
            })
            .into_owned();
        (replaced, count)
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
        if self.spans_lines {
            let matches = self.matches_between(buffer, 0..buffer.line_count());
            let found = if forward {
                matches.iter().find(|m| m.start > from)
            } else {
                matches.iter().rev().find(|m| m.start < from)
            };
            return match found {
                Some(&found) => Some(Found {
                    found,
                    wrapped: false,
                }),
                None if !wrap => None,
                None => {
                    let found = if forward {
                        matches.first()
                    } else {
                        matches.last()
                    };
                    found.map(|&found| Found {
                        found,
                        wrapped: true,
                    })
                }
            };
        }
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
                    found: Match::on_row(row, start, end),
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

/// `replacement` cased like `matched`, as VS Code's preserve case does: all
/// capitals, all small, or (mixed) its first letter's case; parts between
/// `-` or `_` one by one when both have as many. A match with no cased
/// letters leaves it as it is.
pub fn preserve_case(matched: &str, replacement: &str) -> String {
    for separator in ['-', '_'] {
        if matched.contains(separator) && replacement.contains(separator) {
            let matched: Vec<&str> = matched.split(separator).collect();
            let replacement: Vec<&str> = replacement.split(separator).collect();
            if matched.len() == replacement.len() {
                let parts: Vec<String> = matched
                    .iter()
                    .zip(&replacement)
                    .map(|(m, r)| preserve_case(m, r))
                    .collect();
                return parts.join(&separator.to_string());
            }
        }
    }
    let upper = matched.chars().any(char::is_uppercase);
    let lower = matched.chars().any(char::is_lowercase);
    match (upper, lower) {
        (false, false) => replacement.to_string(),
        (true, false) => replacement.to_uppercase(),
        (false, true) => replacement.to_lowercase(),
        (true, true) => {
            let first = matched
                .chars()
                .find(|c| c.is_uppercase() || c.is_lowercase());
            let mut rest = replacement.chars();
            match (first, rest.next()) {
                (Some(m), Some(r)) if m.is_uppercase() => r.to_uppercase().chain(rest).collect(),
                (Some(_), Some(r)) => r.to_lowercase().chain(rest).collect(),
                _ => replacement.to_string(),
            }
        }
    }
}

/// Whether `regex` can match a line break: it has one in it, or `\n`,
/// `\r` or `\W` (VS Code's rule).
fn can_match_line_break(regex: &str) -> bool {
    let mut chars = regex.chars();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\r' => return true,
            '\\' if matches!(chars.next(), Some('n' | 'r' | 'W')) => return true,
            _ => {}
        }
    }
    false
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
        found.map(|f| (f.found.start.row, f.found.start.col, f.wrapped))
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
    fn replace_first_or_all() {
        let p = pattern("o");
        assert_eq!(p.replace("foo boo", "0", false), ("f0o boo".into(), 1));
        assert_eq!(p.replace("foo boo", "0", true), ("f00 b00".into(), 4));
        assert_eq!(p.replace("xyz", "0", true), ("xyz".into(), 0));
        let groups = pattern(r"(\w+)@(\w+)");
        assert_eq!(groups.replace("me@host", "${2}:${1}", false).0, "host:me");
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
    fn preserve_case_follows_the_match() {
        let cases = [
            ("foo", "bar", "bar"),
            ("FOO", "bar", "BAR"),
            ("foo", "BAR", "bar"),
            ("Foo", "bar", "Bar"),
            ("fOO", "Bar", "bar"),
            ("Foo", "barBaz", "BarBaz"),
            ("fooBar", "BazQux", "bazQux"),
            // Parts between separators, when both have as many.
            ("Foo-BAR", "baz-qux", "Baz-QUX"),
            ("foo_BAR", "baz_qux", "baz_QUX"),
            ("FOO-bar", "baz", "Baz"),
            // Nothing to follow.
            ("123", "abc", "abc"),
            ("Foo", "", ""),
            ("Éclair", "über", "Über"),
        ];
        for (matched, replacement, expected) in cases {
            assert_eq!(
                preserve_case(matched, replacement),
                expected,
                "{matched} -> {replacement}"
            );
        }
    }

    #[test]
    fn replace_mapped_maps_each_expanded_match() {
        let p = SearchPattern::from_regex(r"(\w)oo", "", true).unwrap();
        let (text, count) =
            p.replace_mapped("Foo moo GOO", "${1}ar", true, |m, r| preserve_case(m, &r));
        assert_eq!((text.as_str(), count), ("Far mar GAR", 3));
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

    #[test]
    fn patterns_with_a_line_break_span_lines() {
        assert!(pattern(r"a\nb").spans_lines());
        assert!(pattern("a\nb").spans_lines(), "a line break in it");
        assert!(pattern(r"a\Wb").spans_lines());
        assert!(!pattern(r"a\sb").spans_lines(), "as in VS Code");
        assert!(!pattern(r"a\\nb").spans_lines(), "an escaped backslash");
        assert!(!pattern(r"[^x]").spans_lines());
    }

    #[test]
    fn matches_run_across_rows() {
        let b = Buffer::from_text("one\ntwo\nthree two\nfour");
        let p = pattern(r"two\n\w+");
        assert_eq!(
            p.matches_between(&b, 0..4),
            vec![
                Match {
                    start: pos(1, 0),
                    end: pos(2, 5)
                },
                Match {
                    start: pos(2, 6),
                    end: pos(3, 4)
                },
            ]
        );
        assert_eq!(p.matches_between(&b, 2..3).len(), 0, "cut at the last row");
        assert_eq!(
            pattern(r"o$\n^[tf]").matches_between(&b, 0..4).len(),
            2,
            "^ and $ at each line"
        );
    }

    #[test]
    fn find_across_lines_wraps() {
        let b = Buffer::from_text("a\nb a\nb");
        let p = pattern(r"a\nb");
        assert_eq!(
            found_at(p.find(&b, pos(0, 0), true, true)),
            Some((1, 2, false))
        );
        assert_eq!(
            found_at(p.find(&b, pos(1, 2), true, true)),
            Some((0, 0, true))
        );
        assert_eq!(p.find(&b, pos(1, 2), true, false), None);
        assert_eq!(
            found_at(p.find(&b, pos(1, 2), false, true)),
            Some((0, 0, false))
        );
    }

    #[test]
    fn a_match_s_columns_on_each_row() {
        let m = Match {
            start: pos(1, 2),
            end: pos(3, 1),
        };
        assert_eq!(m.cols_on(0, 5), None);
        assert_eq!(m.cols_on(1, 5), Some((2, 6)), "on past the line break");
        assert_eq!(m.cols_on(2, 0), Some((0, 1)));
        assert_eq!(m.cols_on(3, 5), Some((0, 1)));
    }
}
