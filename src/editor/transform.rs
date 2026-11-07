//! Text transformations that aren't deletes or yanks: indenting, changing
//! case, joining lines, and working out the indentation for a new line.
//!
//! Indentation follows the language's rules where it has them
//! (`syntax::indent`, from the syntax tree): `=` re-indents lines, Enter,
//! `o` and `O` start a line at the right depth, and typing a closing
//! bracket first on a line puts it where it belongs. Without rules (Python,
//! YAML, plain text, or `auto-indent` off for new lines), a new line copies
//! the indentation of the one it was opened from, one level more after an
//! opening bracket (or `:` in Python).

use std::ops::RangeInclusive;

use super::Editor;
use crate::core::{Position, Range, motion};
use crate::syntax::indent::{self, IndentStyle};

/// How `gu`, `gU` and `g~` change letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseChange {
    Lower,
    Upper,
    Toggle,
}

impl CaseChange {
    pub fn apply(self, text: &str) -> String {
        match self {
            CaseChange::Lower => text.to_lowercase(),
            CaseChange::Upper => text.to_uppercase(),
            CaseChange::Toggle => text
                .chars()
                .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
                    if c.is_lowercase() {
                        Box::new(c.to_uppercase())
                    } else {
                        Box::new(c.to_lowercase())
                    }
                })
                .collect(),
        }
    }
}

impl Editor {
    /// One level of indentation: a tab, or `tab-width` spaces with
    /// `expand-tab`.
    pub fn indent_unit(&self) -> String {
        if self.options.expand_tab {
            " ".repeat(self.options.tab_width)
        } else {
            "\t".to_string()
        }
    }

    /// `>` and `<`: indents (or outdents) every line of `range` by one level.
    ///
    /// Like Vim, the indentation is measured and rebuilt rather than having
    /// characters added or removed, so a line's indent is always made the
    /// configured way (spaces with `expand-tab`, otherwise tabs then spaces)
    /// and mixed indentation is tidied up. Empty lines aren't indented.
    pub fn shift_lines(&mut self, range: Range, indent: bool) {
        let (first, last) = range.rows();
        let tab_width = self.options.tab_width.max(1);
        let expand_tab = self.options.expand_tab;
        let doc = &mut self.document;
        for row in first..=last {
            let line = doc.buffer().line(row).into_owned();
            if line.is_empty() {
                continue;
            }
            let old_len = line.chars().take_while(|c| c.is_whitespace()).count();
            let width = line.chars().take(old_len).fold(0, |col, c| {
                if c == '\t' {
                    col - col % tab_width + tab_width
                } else {
                    col + 1
                }
            });
            let width = if indent {
                width + tab_width
            } else {
                width.saturating_sub(tab_width)
            };
            let new_indent = if expand_tab {
                " ".repeat(width)
            } else {
                "\t".repeat(width / tab_width) + &" ".repeat(width % tab_width)
            };
            if line.chars().take(old_len).collect::<String>() != new_indent {
                doc.delete(Position::new(row, 0), Position::new(row, old_len));
                doc.insert(Position::new(row, 0), &new_indent);
            }
        }
        let col = motion::first_non_blank(doc.buffer(), first);
        doc.set_cursor(Position::new(first, col));
    }

    /// `gu`, `gU`, `g~`: changes the case of the text in `range`, leaving the
    /// cursor at its start.
    pub fn change_case(&mut self, range: Range, change: CaseChange) {
        let doc = &mut self.document;
        let (start, end) = match range {
            Range::Chars { start, end } => (start, end),
            Range::Lines { first, last } => (Position::new(first, 0), doc.buffer().line_end(last)),
        };
        let text = doc.buffer().text_range(start, end);
        let changed = change.apply(&text);
        if changed != text {
            doc.delete(start, end);
            doc.insert(start, &changed);
        }
        let cursor = match range {
            Range::Chars { start, .. } => start,
            Range::Lines { first, .. } => Position::new(first, doc.cursor().col),
        };
        doc.set_cursor(cursor);
    }

    /// `J`: joins line `row` with the next one, Vim-style: the next line's
    /// indentation is replaced by one space (none if either side is empty,
    /// the line already ends in a space, or the next starts with `)`).
    /// Returns `false` if `row` is the last line.
    pub fn join_line(&mut self, row: usize) -> bool {
        self.join_line_with(row, true)
    }

    /// `gJ`: joins line `row` and the next exactly as they are, without
    /// removing indentation or adding a space.
    pub fn join_line_raw(&mut self, row: usize) -> bool {
        self.join_line_with(row, false)
    }

    fn join_line_with(&mut self, row: usize, tidy: bool) -> bool {
        let doc = &mut self.document;
        if row + 1 >= doc.buffer().line_count() {
            return false;
        }
        if !tidy {
            let join_at = doc.buffer().line_end(row);
            doc.delete(join_at, Position::new(row + 1, 0));
            doc.set_cursor(join_at);
            return true;
        }
        let line = doc.buffer().line(row).into_owned();
        let next = doc.buffer().line(row + 1).into_owned();
        let indent = next.chars().take_while(|c| c.is_whitespace()).count();
        let rest = &next[next
            .char_indices()
            .nth(indent)
            .map_or(next.len(), |(i, _)| i)..];

        let join_at = doc.buffer().line_end(row);
        doc.delete(join_at, Position::new(row + 1, indent));
        let needs_space = !line.is_empty()
            && !line.ends_with(char::is_whitespace)
            && !rest.is_empty()
            && !rest.starts_with(')');
        if needs_space {
            doc.insert(join_at, " ");
        }
        doc.set_cursor(join_at);
        true
    }

    /// Typing a closing bracket on a line that so far holds only
    /// indentation removes one level, so `}` lines up with its `{` line.
    pub(super) fn dedent_for_closing_bracket(&mut self, c: char) {
        if !self.options.auto_indent || !matches!(c, '}' | ')' | ']') {
            return;
        }
        let cursor = self.document.cursor();
        let line = self.document.buffer().line(cursor.row);
        let before: Vec<char> = line.chars().take(cursor.col).collect();
        if before.is_empty() || !before.iter().all(|c| c.is_whitespace()) {
            return;
        }
        let remove = if before.ends_with(&['\t']) {
            1
        } else {
            let tab_width = self.options.tab_width.max(1);
            before
                .iter()
                .rev()
                .take(tab_width)
                .take_while(|&&c| c == ' ')
                .count()
        };
        let start = cursor.with_col(cursor.col - remove);
        self.document.delete(start, cursor);
        self.document.set_cursor(start);
    }

    /// Indentation `width` display columns wide, made the configured way.
    pub fn indent_string(&self, width: usize) -> String {
        let tab = self.options.tab_width.max(1);
        whitespace(0, width, tab, self.options.expand_tab)
    }

    /// The width each line of `rows` should be indented to by the
    /// language's rules (see [`indent::compute`]), or `None` without rules.
    fn rule_indents(&self, rows: RangeInclusive<usize>) -> Option<Vec<Option<usize>>> {
        let language = self.language_of(&self.document)?;
        let buffer = self.document.buffer();
        if buffer.len_chars() > crate::syntax::MAX_HIGHLIGHT_CHARS {
            return None;
        }
        let tab = self.options.tab_width.max(1);
        let style = IndentStyle {
            tab_width: tab,
            unit: tab,
        };
        indent::compute(language, &buffer.text(), rows, style)
    }

    /// One line's indentation by the rules.
    fn rule_indent(&self, row: usize) -> Option<usize> {
        self.rule_indents(row..=row)?.first().copied().flatten()
    }

    /// `=`: re-indents lines `first` to `last` by the language's rules.
    /// Blank lines are emptied; lines inside a multi-line string or comment
    /// are left alone.
    pub fn reindent_lines(&mut self, first: usize, last: usize) -> Result<(), String> {
        let language = self
            .language_of(&self.document)
            .ok_or("Can't re-indent: no language for this file")?
            .to_string();
        if !indent::has_rules(&language) {
            return Err(format!(
                "Can't re-indent: no indentation rules for {language}"
            ));
        }
        let widths = self
            .rule_indents(first..=last)
            .ok_or("Can't re-indent: the file is too large")?;
        for (row, width) in (first..=last).zip(widths) {
            if let Some(width) = width {
                self.set_line_indent(row, width);
            }
        }
        let col = motion::first_non_blank(self.document.buffer(), first);
        self.document.set_cursor(Position::new(first, col));
        Ok(())
    }

    /// Replaces line `row`'s leading whitespace with indentation `width`
    /// wide (or none, if the line is blank).
    fn set_line_indent(&mut self, row: usize, width: usize) {
        let line = self.document.buffer().line(row).into_owned();
        let blanks = line.chars().take_while(|c| matches!(c, ' ' | '\t')).count();
        let new = if blanks == line.chars().count() {
            String::new()
        } else {
            self.indent_string(width)
        };
        if line.chars().take(blanks).ne(new.chars()) {
            self.document
                .delete(Position::new(row, 0), Position::new(row, blanks));
            self.document.insert(Position::new(row, 0), &new);
        }
    }

    /// The indentation for line `row`, just opened by Enter or `o` (or by
    /// `O`, with `above`): by the language's rules, or else the indentation
    /// of the line it was opened from, one level more after an opening
    /// bracket. Nothing with `auto-indent` off.
    pub fn indent_for_opened_line(&self, row: usize, above: bool) -> String {
        if !self.options.auto_indent {
            return String::new();
        }
        if let Some(width) = self.rule_indent(row) {
            return self.indent_string(width);
        }
        let buffer = self.document.buffer();
        let from = if above {
            row + 1
        } else {
            row.saturating_sub(1)
        };
        if from >= buffer.line_count() || from == row {
            return String::new();
        }
        let line = buffer.line(from);
        let mut indent: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let python = self.language_of(&self.document) == Some("python");
        let trimmed = line.trim_end();
        if !above && (trimmed.ends_with(['{', '(', '[']) || (python && trimmed.ends_with(':'))) {
            indent.push_str(&self.indent_unit());
        }
        indent
    }

    /// Enter in insert mode: breaks the line and indents the new one.
    /// Between a pair of brackets (`{|}`), the closing one goes on a line
    /// of its own too, with the cursor on an indented line between.
    pub fn insert_line_break(&mut self) {
        let cursor = self.document.cursor();
        let line = self.document.buffer().line(cursor.row).into_owned();
        let before = cursor
            .col
            .checked_sub(1)
            .and_then(|col| line.chars().nth(col));
        let after = line.chars().nth(cursor.col);
        let between = self.options.auto_indent
            && matches!(
                (before, after),
                (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
            );
        let row = cursor.row + 1;
        if !between {
            self.document.insert_str("\n");
            let indent = self.indent_for_opened_line(row, false);
            self.document.insert_str(&indent);
            return;
        }
        let own_indent: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        self.document.insert_str("\n\n");
        let inner = self.indent_for_opened_line(row, false);
        let closing = match self.rule_indent(row + 1) {
            Some(width) => self.indent_string(width),
            None => own_indent,
        };
        self.document.insert(Position::new(row + 1, 0), &closing);
        self.document.insert(Position::new(row, 0), &inner);
        self.document
            .set_cursor(Position::new(row, inner.chars().count()));
    }

    /// Called as `c` is typed in insert mode. A closing bracket typed first
    /// on a line re-indents the line by the language's rules, or without
    /// rules, takes off one level (see [`Editor::dedent_for_closing_bracket`]).
    pub(super) fn type_with_indent(&mut self, c: char) {
        let cursor = self.document.cursor();
        let first_on_line = self
            .document
            .buffer()
            .line(cursor.row)
            .chars()
            .take(cursor.col)
            .all(|c| matches!(c, ' ' | '\t'));
        let by_rules = self.options.auto_indent
            && CLOSERS.contains(&c)
            && first_on_line
            && self
                .language_of(&self.document)
                .is_some_and(indent::has_rules);
        if !by_rules {
            self.dedent_for_closing_bracket(c);
            self.document.insert_char(c);
            return;
        }
        self.document.insert_char(c);
        if let Some(width) = self.rule_indent(cursor.row) {
            self.set_line_indent(cursor.row, width);
            let col = self.indent_string(width).chars().count() + 1;
            self.document.set_cursor(Position::new(cursor.row, col));
        }
    }
}

/// Whitespace running from display column `from` to `to`: spaces, or tabs
/// to each tab stop then spaces.
pub(super) fn whitespace(from: usize, to: usize, tab: usize, expand_tab: bool) -> String {
    if expand_tab {
        return " ".repeat(to - from);
    }
    let mut text = String::new();
    let mut col = from;
    loop {
        let stop = col - col % tab + tab;
        if stop > to {
            break;
        }
        text.push('\t');
        col = stop;
    }
    text + &" ".repeat(to - col)
}

/// The characters a line can be re-indented after typing, when they're the
/// first thing on it: closing brackets.
const CLOSERS: [char; 3] = ['}', ')', ']'];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn editor(text: &str) -> Editor {
        Editor::with_document(Document::with_text(text))
    }

    fn text(e: &Editor) -> String {
        e.document.buffer().text()
    }

    fn lines(first: usize, last: usize) -> Range {
        Range::Lines { first, last }
    }

    #[test]
    fn case_changes() {
        assert_eq!(CaseChange::Lower.apply("MiXeD"), "mixed");
        assert_eq!(CaseChange::Upper.apply("MiXeD"), "MIXED");
        assert_eq!(CaseChange::Toggle.apply("MiXeD 1"), "mIxEd 1");
        assert_eq!(
            CaseChange::Upper.apply("straße"),
            "STRASSE",
            "can change length"
        );
    }

    #[test]
    fn indent_and_outdent_with_spaces() {
        let mut e = editor("a\n\n  b");
        e.options.tab_width = 2;
        e.shift_lines(lines(0, 2), true);
        assert_eq!(text(&e), "  a\n\n    b", "empty line left alone");
        assert_eq!(e.document.cursor(), Position::new(0, 2));
        e.shift_lines(lines(0, 2), false);
        e.shift_lines(lines(0, 2), false);
        assert_eq!(text(&e), "a\n\nb");
    }

    #[test]
    fn indent_with_tabs() {
        let mut e = editor("a\n\tb");
        e.options.expand_tab = false;
        e.shift_lines(lines(0, 1), true);
        assert_eq!(text(&e), "\ta\n\t\tb");
        e.shift_lines(lines(1, 1), false);
        assert_eq!(text(&e), "\ta\n\tb");
    }

    #[test]
    fn mixed_indentation_is_rebuilt_the_configured_way() {
        // A tab (to column 4) indented once more becomes 8 spaces...
        let mut e = editor("\tx");
        e.options.tab_width = 4;
        e.shift_lines(lines(0, 0), true);
        assert_eq!(text(&e), "        x");

        // ...and with tabs, 6 spaces outdented becomes 2 spaces.
        let mut e = editor("      y");
        e.options.tab_width = 4;
        e.options.expand_tab = false;
        e.shift_lines(lines(0, 0), true);
        assert_eq!(text(&e), "\t\t  y", "10 columns: two tabs and two spaces");
        e.shift_lines(lines(0, 0), false);
        e.shift_lines(lines(0, 0), false);
        assert_eq!(text(&e), "  y");
    }

    #[test]
    fn outdent_removes_at_most_one_level() {
        let mut e = editor("      x");
        e.options.tab_width = 4;
        e.shift_lines(lines(0, 0), false);
        assert_eq!(text(&e), "  x");
    }

    #[test]
    fn change_case_of_chars_and_lines() {
        let mut e = editor("hello world\nagain");
        let word = Range::Chars {
            start: Position::new(0, 6),
            end: Position::new(0, 11),
        };
        e.change_case(word, CaseChange::Upper);
        assert_eq!(text(&e), "hello WORLD\nagain");
        assert_eq!(e.document.cursor(), Position::new(0, 6));
        e.change_case(lines(0, 1), CaseChange::Toggle);
        assert_eq!(text(&e), "HELLO world\nAGAIN");
    }

    #[test]
    fn unchanged_case_records_no_edit() {
        let mut e = editor("abc");
        e.change_case(lines(0, 0), CaseChange::Lower);
        assert!(!e.document.is_modified());
    }

    #[test]
    fn join_line_rules() {
        let cases = [
            ("foo\n    bar", "foo bar", 3),
            ("foo \nbar", "foo bar", 4),
            ("foo\n", "foo", 3),
            ("\nbar", "bar", 0),
            ("call(\n)", "call()", 5),
        ];
        for (before, after, col) in cases {
            let mut e = editor(before);
            assert!(e.join_line(0), "{before:?}");
            assert_eq!(text(&e), after, "{before:?}");
            assert_eq!(e.document.cursor().col, col, "{before:?}");
        }
        assert!(!editor("only").join_line(0));
    }

    #[test]
    fn closing_bracket_on_an_indented_blank_line_dedents() {
        let mut e = editor("        ");
        e.options.tab_width = 4;
        e.document.set_cursor(Position::new(0, 8));
        e.dedent_for_closing_bracket('}');
        assert_eq!(text(&e), "    ");
        assert_eq!(e.document.cursor(), Position::new(0, 4));

        let mut e = editor("    x");
        e.document.set_cursor(Position::new(0, 5));
        e.dedent_for_closing_bracket('}');
        assert_eq!(text(&e), "    x", "text before the cursor: no change");

        let mut e = editor("    ");
        e.document.set_cursor(Position::new(0, 4));
        e.dedent_for_closing_bracket('a');
        assert_eq!(text(&e), "    ", "only closing brackets");
    }

    #[test]
    fn new_line_indent_without_rules() {
        // No language: the line it was opened from, plus a level after an
        // opening bracket.
        let mut e = editor("    let x = foo(\n\n  y\n");
        e.options.tab_width = 4;
        assert_eq!(e.indent_for_opened_line(1, false), "        ");
        assert_eq!(e.indent_for_opened_line(3, false), "  ");
        assert_eq!(e.indent_for_opened_line(1, true), "  ", "O: the line below");
        e.options.auto_indent = false;
        assert_eq!(e.indent_for_opened_line(1, false), "");
    }

    #[test]
    fn enter_between_brackets_opens_a_line_inside() {
        let mut e = editor("  f({})");
        e.document.set_cursor(Position::new(0, 5));
        e.insert_line_break();
        assert_eq!(text(&e), "  f({\n      \n  })");
        assert_eq!(e.document.cursor(), Position::new(1, 6));
    }

    #[test]
    fn whitespace_fills_to_tab_stops() {
        assert_eq!(whitespace(1, 5, 4, true), "    ");
        assert_eq!(whitespace(1, 5, 4, false), "\t ");
        assert_eq!(whitespace(0, 8, 4, false), "\t\t");
        assert_eq!(whitespace(2, 3, 4, false), " ");
    }
}
