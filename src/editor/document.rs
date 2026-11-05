use crate::core::{Buffer, Position};

/// A buffer plus the cursor editing it.
///
/// `Document` offers primitive, mode-agnostic edits. Higher-level behaviour
/// (counts, mode switching, Vim semantics) lives in the commands.
#[derive(Debug, Default, Clone)]
pub struct Document {
    buffer: Buffer,
    cursor: Position,
    /// Column to aim for when moving vertically through shorter lines.
    sticky_col: Option<usize>,
}

impl Document {
    pub fn with_text(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            ..Self::default()
        }
    }

    /// Display name for the UI. Will come from the file path once files exist.
    pub fn name(&self) -> &str {
        "[scratch]"
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    /// Moves the cursor, clamping it into the buffer. Allows the end-of-line slot.
    pub fn set_cursor(&mut self, pos: Position) {
        let row = pos.row.min(self.buffer.line_count() - 1);
        let col = pos.col.min(self.buffer.line_len(row));
        self.cursor = Position::new(row, col);
        self.sticky_col = None;
    }

    /// Moves the cursor `delta` lines, keeping the column where possible.
    pub fn move_vertical(&mut self, delta: isize) {
        let max_row = self.buffer.line_count() - 1;
        let row = self.cursor.row.saturating_add_signed(delta).min(max_row);
        let target = *self.sticky_col.get_or_insert(self.cursor.col);
        self.cursor = Position::new(row, target.min(self.buffer.line_len(row)));
    }

    /// Keeps the cursor on a character, as required in normal mode.
    pub fn clamp_to_last_char(&mut self) {
        let max = self.buffer.line_len(self.cursor.row).saturating_sub(1);
        self.cursor.col = self.cursor.col.min(max);
    }

    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert_char(self.cursor, c);
        self.set_cursor(Position::new(self.cursor.row, self.cursor.col + 1));
    }

    pub fn insert_str(&mut self, s: &str) {
        self.buffer.insert_str(self.cursor, s);
        let col = self.cursor.col + s.chars().count();
        self.set_cursor(Position::new(self.cursor.row, col));
    }

    pub fn insert_newline(&mut self) {
        self.buffer.split_line(self.cursor);
        self.set_cursor(Position::new(self.cursor.row + 1, 0));
    }

    /// Deletes the char before the cursor, joining lines at the line start.
    pub fn delete_backward(&mut self) {
        let Position { row, col } = self.cursor;
        if col > 0 {
            self.buffer.remove_char(Position::new(row, col - 1));
            self.set_cursor(Position::new(row, col - 1));
        } else if row > 0 {
            let prev_len = self.buffer.line_len(row - 1);
            self.buffer.join_with_next(row - 1);
            self.set_cursor(Position::new(row - 1, prev_len));
        }
    }

    /// Deletes the char under the cursor, joining lines at the line end.
    pub fn delete_forward(&mut self) {
        if self.buffer.remove_char(self.cursor).is_none() {
            self.buffer.join_with_next(self.cursor.row);
        }
    }

    /// Deletes up to `count` chars under the cursor without crossing lines.
    pub fn delete_chars(&mut self, count: usize) {
        for _ in 0..count {
            if self.buffer.remove_char(self.cursor).is_none() {
                break;
            }
        }
        self.sticky_col = None;
    }

    /// Deletes `count` lines starting at the cursor's line.
    pub fn delete_lines(&mut self, count: usize) {
        let row = self.cursor.row;
        let available = self.buffer.line_count() - row;
        for _ in 0..count.min(available) {
            self.buffer.remove_line(row);
        }
        self.set_cursor(Position::new(row, 0));
    }

    /// Inserts an empty line below (or above) the cursor and moves onto it.
    pub fn open_line(&mut self, below: bool) {
        let row = self.cursor.row + usize::from(below);
        self.buffer.insert_line(row, String::new());
        self.set_cursor(Position::new(row, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cursor_clamps_into_buffer() {
        let mut doc = Document::with_text("ab\nc");
        doc.set_cursor(Position::new(9, 9));
        assert_eq!(doc.cursor(), Position::new(1, 1));
    }

    #[test]
    fn vertical_movement_remembers_column() {
        let mut doc = Document::with_text("long line\nab\nlong line");
        doc.set_cursor(Position::new(0, 6));
        doc.move_vertical(1);
        assert_eq!(doc.cursor(), Position::new(1, 2));
        doc.move_vertical(1);
        assert_eq!(doc.cursor(), Position::new(2, 6));
    }

    #[test]
    fn vertical_movement_clamps_rows() {
        let mut doc = Document::with_text("a\nb");
        doc.move_vertical(-5);
        assert_eq!(doc.cursor().row, 0);
        doc.move_vertical(5);
        assert_eq!(doc.cursor().row, 1);
    }

    #[test]
    fn clamp_to_last_char_moves_off_line_end() {
        let mut doc = Document::with_text("abc");
        doc.set_cursor(Position::new(0, 3));
        doc.clamp_to_last_char();
        assert_eq!(doc.cursor().col, 2);
    }

    #[test]
    fn delete_backward_joins_lines() {
        let mut doc = Document::with_text("ab\ncd");
        doc.set_cursor(Position::new(1, 0));
        doc.delete_backward();
        assert_eq!(doc.buffer().text(), "abcd");
        assert_eq!(doc.cursor(), Position::new(0, 2));
    }

    #[test]
    fn delete_forward_joins_lines_at_end() {
        let mut doc = Document::with_text("ab\ncd");
        doc.set_cursor(Position::new(0, 2));
        doc.delete_forward();
        assert_eq!(doc.buffer().text(), "abcd");
    }

    #[test]
    fn delete_chars_stops_at_line_end() {
        let mut doc = Document::with_text("abc\nd");
        doc.set_cursor(Position::new(0, 1));
        doc.delete_chars(10);
        assert_eq!(doc.buffer().text(), "a\nd");
    }

    #[test]
    fn delete_lines_removes_count_lines() {
        let mut doc = Document::with_text("a\nb\nc\nd");
        doc.set_cursor(Position::new(1, 0));
        doc.delete_lines(2);
        assert_eq!(doc.buffer().text(), "a\nd");
        assert_eq!(doc.cursor(), Position::new(1, 0));
    }

    #[test]
    fn delete_lines_at_end_moves_cursor_up() {
        let mut doc = Document::with_text("a\nb");
        doc.set_cursor(Position::new(1, 0));
        doc.delete_lines(5);
        assert_eq!(doc.buffer().text(), "a");
        assert_eq!(doc.cursor(), Position::new(0, 0));
    }

    #[test]
    fn open_line_above_and_below() {
        let mut doc = Document::with_text("x");
        doc.open_line(true);
        assert_eq!(doc.cursor(), Position::new(1, 0));
        doc.open_line(false);
        assert_eq!(doc.buffer().text(), "x\n\n");
        assert_eq!(doc.cursor(), Position::new(1, 0));
    }

    #[test]
    fn insert_str_advances_by_chars() {
        let mut doc = Document::default();
        doc.insert_str("éé");
        assert_eq!(doc.cursor().col, 2);
    }
}
