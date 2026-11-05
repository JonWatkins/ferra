//! Text transformations that aren't deletes or yanks: indenting, changing
//! case, joining lines, and working out the indentation for a new line.

use super::Editor;
use crate::core::{Position, Range, motion};

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
        let doc = &mut self.document;
        if row + 1 >= doc.buffer().line_count() {
            return false;
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

    /// The indentation for a new line opened after `row`, with the cursor at
    /// `col` (where Enter split it): the line's own indentation, plus a
    /// level if the text before the split ends with an opening bracket.
    pub fn indent_for_new_line(&self, row: usize, col: usize) -> String {
        if !self.options.auto_indent {
            return String::new();
        }
        let line = self.document.buffer().line(row);
        let mut indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let before: String = line.chars().take(col).collect();
        if before.trim_end().ends_with(['{', '(', '[']) {
            indent.push_str(&self.indent_unit());
        }
        indent
    }
}

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
    fn new_line_indent() {
        let mut e = editor("    let x = foo(\n  y");
        e.options.tab_width = 4;
        assert_eq!(
            e.indent_for_new_line(0, 16),
            "        ",
            "after an opening bracket"
        );
        assert_eq!(
            e.indent_for_new_line(0, 8),
            "    ",
            "split before the bracket"
        );
        assert_eq!(e.indent_for_new_line(1, 3), "  ");
        e.options.auto_indent = false;
        assert_eq!(e.indent_for_new_line(0, 16), "");
    }
}
