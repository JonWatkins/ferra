//! Block comments as VS Code writes them, in languages with C's comments
//! (`/* */`, with `auto-pairs` and `auto-indent`):
//!
//! - typing `/**` closes it: `/**| */`;
//! - `Enter` between those opens it out, the cursor on a ` * ` line;
//! - `Enter` on a line of the comment (` * text`) starts the next with
//!   ` * ` too;
//! - after the closing ` */`, the next line lines up with the `/**`.
//!
//! ```text
//! /**
//!  * |
//!  */
//! ```

use super::Editor;
use crate::core::Position;

/// How far back to look for the comment a line is in.
const LOOK_BACK: usize = 200;

impl Editor {
    /// Whether the buffer's language has `/* */` comments.
    fn has_c_comments(&self) -> bool {
        self.language_of(&self.document)
            .is_some_and(|language| self.languages.has_c_comments(language))
    }

    /// Whether line `row` starts inside a block comment: the last `/*` or
    /// `*/` before it (within a few hundred lines) opens one.
    fn starts_in_block_comment(&self, row: usize) -> bool {
        let buffer = self.document.buffer();
        for r in (row.saturating_sub(LOOK_BACK)..row).rev() {
            let line = buffer.line(r);
            let (open, close) = (line.rfind("/*"), line.rfind("*/"));
            match (open, close) {
                (Some(o), Some(c)) if o + 1 == c => {}
                (Some(o), Some(c)) => return o > c,
                (Some(_), None) => return true,
                (None, Some(_)) => return false,
                (None, None) => {}
            }
        }
        false
    }

    /// Typing `*` after `/*` (so `/**`) with nothing after the cursor: puts
    /// in ` */` after it. Returns whether it did (and typed the `*`).
    pub(super) fn close_doc_comment(&mut self, c: char) -> bool {
        if c != '*' || !self.has_c_comments() {
            return false;
        }
        let at = self.document.cursor();
        let line = self.document.buffer().line(at.row).into_owned();
        let before: String = line.chars().take(at.col).collect();
        let after: String = line.chars().skip(at.col).collect();
        if !before.trim_start().ends_with("/*")
            || before.ends_with("//*")
            || !after.trim().is_empty()
        {
            return false;
        }
        self.document.insert_char('*');
        let at = self.document.cursor();
        self.document.insert(at, " */");
        self.document.set_cursor(at);
        true
    }

    /// `Enter` in a block comment, as VS Code does it (see the module's
    /// notes). Returns whether it was one of those.
    pub(super) fn doc_comment_line_break(&mut self) -> bool {
        if !self.options.auto_indent || !self.has_c_comments() {
            return false;
        }
        let at = self.document.cursor();
        let line = self.document.buffer().line(at.row).into_owned();
        let before: String = line.chars().take(at.col).collect();
        let after: String = line.chars().skip(at.col).collect();
        let indent: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let trimmed = before.trim_start();

        // `/**| */` (or `/*| */`): opened out.
        if trimmed.starts_with("/*") && !trimmed.contains("*/") && after.trim() == "*/" {
            let end = Position::new(at.row, line.chars().count());
            self.document.delete(at, end);
            self.document.set_cursor(at);
            self.document.insert_str(&format!("\n{indent} * "));
            let cursor = self.document.cursor();
            self.document.insert(cursor, &format!("\n{indent} */"));
            self.document.set_cursor(cursor);
            return true;
        }
        // `/**` and nothing after: the next line starts ` * `.
        if trimmed.starts_with("/**") && !trimmed.contains("*/") && after.trim().is_empty() {
            self.document.insert_str(&format!("\n{indent} * "));
            return true;
        }
        // ` * text` in a comment: ` * ` again (the `*`s lined up).
        if trimmed.starts_with('*')
            && !trimmed.starts_with("*/")
            && self.starts_in_block_comment(at.row)
        {
            let star_at: String = line
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t'))
                .collect();
            let rest = after.trim_start();
            if !rest.is_empty() {
                let gap = after.chars().count() - rest.chars().count();
                let from = at.with_col(at.col);
                self.document.delete(from, from.with_col(from.col + gap));
                self.document.set_cursor(from);
            }
            self.document.insert_str(&format!("\n{star_at}* "));
            return true;
        }
        // Just after the closing ` */`: back in line with the `/**`.
        if trimmed == "*/" && after.trim().is_empty() && indent.ends_with(' ') {
            let mut outer = indent.clone();
            outer.pop();
            self.document.insert_str(&format!("\n{outer}"));
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Document, Mode};

    /// An editor on a Rust buffer with `text` (`|` the cursor).
    fn rust(text: &str) -> Editor {
        let mut document = Document::with_text(&text.replace('|', ""));
        document.set_language(Some("rust".into()));
        let mut editor = Editor::with_document(document);
        let at = text.find('|').unwrap();
        let row = text[..at].matches('\n').count();
        let col = text[..at].rsplit('\n').next().unwrap().chars().count();
        editor.document.set_cursor(Position::new(row, col));
        editor.options.auto_pairs = true;
        editor.set_mode(Mode::Insert);
        editor
    }

    fn shown(editor: &Editor) -> String {
        let mut text = editor.document.buffer().text();
        let at = editor.document.cursor();
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let line: Vec<char> = lines[at.row].chars().collect();
        lines[at.row] = line[..at.col]
            .iter()
            .chain(['|'].iter())
            .chain(line[at.col..].iter())
            .collect();
        text = lines.join("\n");
        text
    }

    #[test]
    fn typing_slash_star_star_and_enter_opens_a_doc_comment() {
        let mut e = rust("    |");
        for c in "/**".chars() {
            e.insert_text_char(c);
        }
        assert_eq!(shown(&e), "    /**| */");
        e.insert_line_break();
        assert_eq!(shown(&e), "    /**\n     * |\n     */");
        for c in "Adds.".chars() {
            e.insert_text_char(c);
        }
        e.insert_line_break();
        assert_eq!(shown(&e), "    /**\n     * Adds.\n     * |\n     */");
    }

    #[test]
    fn after_the_comment_the_next_line_lines_up() {
        let mut e = rust("/**\n * a\n */|");
        e.insert_line_break();
        assert_eq!(shown(&e), "/**\n * a\n */\n|");
    }

    #[test]
    fn a_star_outside_a_comment_is_left_alone() {
        let mut e = rust("let x = a\n    * b|");
        e.insert_line_break();
        assert!(!shown(&e).contains("\n    * |"), "{}", shown(&e));
    }
}
