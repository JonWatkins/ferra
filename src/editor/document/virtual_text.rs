//! Virtual text: text drawn in a line that isn't in the file, such as a
//! language server's inlay hints (`: i32`, `count:`). Each piece sits
//! before a char of the text and is kept there through edits, as
//! diagnostics are.
//!
//! Drawing and layout use a line's *display line*: its text with the
//! virtual text put in, and a map from the real text's columns to the
//! display line's. So wrapping, the cursor and highlights see the extra
//! width, while editing never does.

use super::Document;
use crate::core::Position;

/// A piece of virtual text, before the char at `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualText {
    pub at: Position,
    pub text: String,
    /// The theme scope it's drawn with.
    pub style: String,
    /// What put it there (`"inlay"`), so it can be replaced as a whole.
    pub source: String,
}

/// A line with its virtual text put in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLine {
    pub text: String,
    /// For each char column of the real line (and its end), the column in
    /// `text` of the char, after any virtual text before it...
    to_display: Vec<usize>,
    /// ...and of the gap before that virtual text.
    before: Vec<usize>,
    /// The virtual text's columns in `text`, and its scopes.
    pub virtual_spans: Vec<(usize, usize, String)>,
}

impl DisplayLine {
    /// Real char column `col` in the display line.
    pub fn to_display(&self, col: usize) -> usize {
        match self.to_display.get(col) {
            Some(&c) => c,
            None => {
                let last = self.to_display.len().saturating_sub(1);
                self.to_display[last] + (col - last)
            }
        }
    }

    /// The gap before real column `col` and any virtual text there: where
    /// an insert-mode cursor goes (typing there pushes the virtual text
    /// along), and where a range ending at `col` ends.
    pub fn caret_at(&self, col: usize) -> usize {
        match self.before.get(col) {
            Some(&c) => c,
            None => self.to_display(col),
        }
    }

    /// The real column shown at display column `col` (in virtual text,
    /// the char it's before).
    pub fn to_real(&self, col: usize) -> usize {
        self.to_display
            .iter()
            .position(|&c| c >= col)
            .unwrap_or(self.to_display.len().saturating_sub(1))
    }
}

impl Document {
    /// Replaces the virtual text from `source` with `pieces`.
    pub fn set_virtual_text(&mut self, source: &str, pieces: Vec<VirtualText>) {
        self.virtual_text.retain(|v| v.source != source);
        self.virtual_text.extend(pieces);
        self.virtual_text.sort_by_key(|v| v.at);
    }

    pub fn clear_virtual_text(&mut self, source: &str) {
        self.virtual_text.retain(|v| v.source != source);
    }

    pub fn virtual_text(&self) -> &[VirtualText] {
        &self.virtual_text
    }

    /// Line `row` with its virtual text put in, if it has any.
    pub fn display_line(&self, row: usize) -> Option<DisplayLine> {
        let first = self.virtual_text.partition_point(|v| v.at.row < row);
        let pieces: Vec<&VirtualText> = self.virtual_text[first..]
            .iter()
            .take_while(|v| v.at.row == row)
            .collect();
        if pieces.is_empty() {
            return None;
        }
        let line = self.buffer().line(row);
        let mut text = String::with_capacity(line.len() + 16);
        let mut to_display = Vec::new();
        let mut before = Vec::new();
        let mut virtual_spans = Vec::new();
        let mut next = pieces.iter().peekable();
        // Puts in the pieces before char `at`.
        let mut put_in = |text: &mut String, at: usize| {
            while let Some(piece) = next.next_if(|p| p.at.col <= at) {
                let start = text.chars().count();
                text.push_str(&piece.text);
                virtual_spans.push((start, text.chars().count(), piece.style.clone()));
            }
        };
        for (i, c) in line.chars().enumerate() {
            before.push(text.chars().count());
            put_in(&mut text, i);
            to_display.push(text.chars().count());
            text.push(c);
        }
        // Virtual text at (or past) the line's end.
        before.push(text.chars().count());
        put_in(&mut text, usize::MAX);
        to_display.push(text.chars().count());
        Some(DisplayLine {
            text,
            to_display,
            before,
            virtual_spans,
        })
    }

    /// After `at..end` was inserted: virtual text after it moves on.
    pub(super) fn virtual_text_after_insert(&mut self, at: Position, end: Position) {
        for v in &mut self.virtual_text {
            if v.at > at || (v.at == at && at != end) {
                v.at = if v.at.row == at.row {
                    Position::new(end.row, end.col + (v.at.col - at.col))
                } else {
                    Position::new(v.at.row + (end.row - at.row), v.at.col)
                };
            }
        }
    }

    /// After `start..end` was deleted: virtual text in it goes, and after
    /// it moves back.
    pub(super) fn virtual_text_after_delete(&mut self, start: Position, end: Position) {
        self.virtual_text.retain(|v| !(v.at > start && v.at < end));
        for v in &mut self.virtual_text {
            if v.at >= end {
                v.at = if v.at.row == end.row {
                    Position::new(start.row, start.col + (v.at.col - end.col))
                } else {
                    Position::new(v.at.row - (end.row - start.row), v.at.col)
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(row: usize, col: usize, text: &str) -> VirtualText {
        VirtualText {
            at: Position::new(row, col),
            text: text.into(),
            style: "ui.virtual.inlay-hint".into(),
            source: "inlay".into(),
        }
    }

    #[test]
    fn display_lines_put_virtual_text_in_and_map_columns() {
        let mut doc = Document::with_text("let x = f(1);\nnone");
        doc.set_virtual_text(
            "inlay",
            vec![
                hint(0, 5, ": i32"),
                hint(0, 10, "n: "),
                hint(0, 13, " //end"),
            ],
        );
        let line = doc.display_line(0).unwrap();
        assert_eq!(line.text, "let x: i32 = f(n: 1); //end");
        assert_eq!(line.to_display(4), 4, "x");
        assert_eq!(line.to_display(5), 10, "the space after the hint");
        assert_eq!(line.to_display(10), 18, "1, after `n: `");
        assert_eq!(line.caret_at(13), 21, "the end, before the end's hint");
        assert_eq!(line.caret_at(5), 5, "before `: i32`");
        assert_eq!(line.caret_at(4), 4);
        assert_eq!(
            line.virtual_spans[0],
            (5, 10, "ui.virtual.inlay-hint".into())
        );
        assert_eq!(line.to_real(7), 5, "in a hint: the char after it");
        assert_eq!(line.to_real(18), 10);
        assert_eq!(doc.display_line(1), None, "no virtual text");
    }

    #[test]
    fn virtual_text_follows_edits() {
        let mut doc = Document::with_text("let x = 1;\nlet y = 2;");
        doc.set_virtual_text("inlay", vec![hint(0, 5, ": i32"), hint(1, 5, ": i32")]);
        doc.insert(Position::new(0, 0), "  ");
        assert_eq!(doc.virtual_text()[0].at, Position::new(0, 7));
        doc.insert(Position::new(0, 0), "// note\n");
        assert_eq!(doc.virtual_text()[1].at, Position::new(2, 5));
        // Deleting what a hint was inside drops it.
        doc.delete(Position::new(1, 2), Position::new(1, 9));
        assert_eq!(doc.virtual_text().len(), 1);
        assert_eq!(doc.virtual_text()[0].at, Position::new(2, 5));
        doc.set_virtual_text("inlay", vec![]);
        assert!(doc.virtual_text().is_empty());
    }
}
