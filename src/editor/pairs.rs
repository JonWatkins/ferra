//! Auto-closing brackets and quotes (`auto-pairs`), as VS Code has them:
//! typing `(` puts in `)` after the cursor, typing `)` with one already
//! there steps over it, `Backspace` inside an empty pair takes both, and
//! (with VS Code's keys) typing one over a selection wraps it.

use super::Editor;
use crate::core::Position;

/// The closing character for `c`, if it opens a pair.
fn closer(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' | '\'' | '`' => Some(c),
        _ => None,
    }
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

impl Editor {
    /// The characters before and after the cursor on its line.
    fn around_cursor(&self) -> (Option<char>, Option<char>) {
        let at = self.document.cursor();
        let line: Vec<char> = self.document.buffer().line(at.row).chars().collect();
        let before = at.col.checked_sub(1).and_then(|i| line.get(i)).copied();
        (before, line.get(at.col).copied())
    }

    /// Typing `c` in insert mode, before it's put in: steps over a closer
    /// that's already next (returns `true`: nothing more to type), or, for
    /// an opener in the right place, types it and its closer (also `true`).
    pub(super) fn auto_pair(&mut self, c: char) -> bool {
        if !self.options.auto_pairs || self.replacing.is_some() {
            return false;
        }
        // `/**`: closed with ` */`.
        if self.close_doc_comment(c) {
            return true;
        }
        let (before, after) = self.around_cursor();
        // `)` (or a quote) with the same next: over it.
        if after == Some(c) && (matches!(c, ')' | ']' | '}') || is_quote(c)) {
            let at = self.document.cursor();
            self.document.set_cursor(at.with_col(at.col + 1));
            return true;
        }
        let Some(close) = closer(c) else {
            return false;
        };
        // Only before space, the line's end or what closes something.
        let room = after.is_none_or(|a| a.is_whitespace() || ";:.,=)]}>".contains(a));
        // A quote isn't paired after a word (`don't`), or a Rust lifetime
        // (`&'a`, `<'a>`).
        let quote_ok = !is_quote(c)
            || before.is_none_or(|b| {
                !(b.is_alphanumeric() || b == '_' || (c == '\'' && matches!(b, '&' | '<')))
            });
        if !room || !quote_ok {
            return false;
        }
        self.type_with_indent(c);
        let at = self.document.cursor();
        self.document.insert(at, &close.to_string());
        true
    }

    /// `Backspace` between an empty pair (`(|)`): takes both. Returns
    /// whether it did.
    pub fn delete_empty_pair(&mut self) -> bool {
        if !self.options.auto_pairs {
            return false;
        }
        let (Some(before), Some(after)) = self.around_cursor() else {
            return false;
        };
        if closer(before) != Some(after) {
            return false;
        }
        let at = self.document.cursor();
        let start = at.with_col(at.col - 1);
        self.document.delete(start, at.with_col(at.col + 1));
        self.document.set_cursor(start);
        true
    }

    /// An opener typed over a selection (VS Code's keys): the selection
    /// wrapped in the pair, and still selected. Returns whether it did.
    pub(super) fn wrap_selection(&mut self, c: char) -> bool {
        let Some(close) = closer(c).filter(|_| self.options.auto_pairs) else {
            return false;
        };
        let Some(crate::core::Range::Chars { start, end }) = self.selection_range() else {
            return false;
        };
        if start == end {
            return false;
        }
        let cursor = self.document.cursor();
        let anchor = self.selection.map(|s| s.anchor).unwrap_or(start);
        self.document.insert(end, &close.to_string());
        self.document.insert(start, &c.to_string());
        // Both ends move one along where they're on the first line.
        let shift = |p: Position| {
            if p.row == start.row {
                p.with_col(p.col + 1)
            } else {
                p
            }
        };
        if let Some(selection) = &mut self.selection {
            selection.anchor = shift(anchor);
        }
        self.document.set_cursor(shift(cursor));
        true
    }
}
