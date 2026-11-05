use super::Position;

/// The text being edited, stored as a list of lines (without line endings).
///
/// A buffer always contains at least one (possibly empty) line. All mutation
/// goes through two primitives, [`insert_text`](Self::insert_text) and
/// [`delete_range`](Self::delete_range), which makes every edit expressible
/// as an invertible [`Change`](super::Change) for undo.
///
/// Positions passed in must be valid (`row < line_count()`,
/// `col <= line_len(row)`); keeping them valid is the caller's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    lines: Vec<String>,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
        }
    }
}

impl Buffer {
    /// Creates a buffer from a string, splitting on `\n`.
    pub fn from_text(text: &str) -> Self {
        Self {
            lines: text.split('\n').map(String::from).collect(),
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line(&self, row: usize) -> &str {
        &self.lines[row]
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Length of the given line in `char`s.
    pub fn line_len(&self, row: usize) -> usize {
        self.lines[row].chars().count()
    }

    /// The position just past the last character of `row`.
    pub fn line_end(&self, row: usize) -> Position {
        Position::new(row, self.line_len(row))
    }

    /// Returns the whole buffer joined with `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The character at `pos`, or `None` at (or past) the end of the line.
    pub fn char_at(&self, pos: Position) -> Option<char> {
        self.lines.get(pos.row)?.chars().nth(pos.col)
    }

    /// The text between `start` (inclusive) and `end` (exclusive).
    pub fn text_range(&self, start: Position, end: Position) -> String {
        debug_assert!(start <= end, "range must be ordered");
        if start.row == end.row {
            return char_slice(&self.lines[start.row], start.col, end.col).to_string();
        }
        let mut text = char_slice(&self.lines[start.row], start.col, usize::MAX).to_string();
        for line in &self.lines[start.row + 1..end.row] {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text.push_str(char_slice(&self.lines[end.row], 0, end.col));
        text
    }

    /// Inserts `text` (which may span lines) at `at`. Returns the position
    /// just after the inserted text.
    pub fn insert_text(&mut self, at: Position, text: &str) -> Position {
        let line = &mut self.lines[at.row];
        let tail = line.split_off(byte_index(line, at.col));

        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or_default();
        line.push_str(first);

        let rest: Vec<String> = parts.map(String::from).collect();
        let end = match rest.last() {
            None => Position::new(at.row, at.col + first.chars().count()),
            Some(last) => Position::new(at.row + rest.len(), last.chars().count()),
        };
        self.lines.splice(at.row + 1..at.row + 1, rest);
        self.lines[end.row].push_str(&tail);
        end
    }

    /// Removes the text between `start` (inclusive) and `end` (exclusive) and
    /// returns it.
    pub fn delete_range(&mut self, start: Position, end: Position) -> String {
        let removed = self.text_range(start, end);
        let end_line = &self.lines[end.row];
        let tail = end_line[byte_index(end_line, end.col)..].to_string();

        let start_line = &mut self.lines[start.row];
        start_line.truncate(byte_index(start_line, start.col));
        start_line.push_str(&tail);
        self.lines.drain(start.row + 1..=end.row);
        removed
    }
}

/// Converts a `char` index into a byte index within `s`.
fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or(s.len(), |(byte, _)| byte)
}

/// The substring between two `char` indices (clamped to the string).
fn char_slice(s: &str, from: usize, to: usize) -> &str {
    &s[byte_index(s, from)..byte_index(s, to)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn default_buffer_has_one_empty_line() {
        assert_eq!(Buffer::default().lines(), &[String::new()]);
    }

    #[test]
    fn char_at_returns_none_past_end() {
        let buffer = Buffer::from_text("ab");
        assert_eq!(buffer.char_at(pos(0, 1)), Some('b'));
        assert_eq!(buffer.char_at(pos(0, 2)), None);
        assert_eq!(buffer.char_at(pos(5, 0)), None);
    }

    #[test]
    fn insert_single_line_text() {
        let mut buffer = Buffer::from_text("héllo");
        let end = buffer.insert_text(pos(0, 2), "XY");
        assert_eq!(buffer.text(), "héXYllo");
        assert_eq!(end, pos(0, 4));
    }

    #[test]
    fn insert_multi_line_text() {
        let mut buffer = Buffer::from_text("ab\ncd");
        let end = buffer.insert_text(pos(0, 1), "1\n2\n3");
        assert_eq!(buffer.text(), "a1\n2\n3b\ncd");
        assert_eq!(end, pos(2, 1));
    }

    #[test]
    fn insert_newline_splits_line() {
        let mut buffer = Buffer::from_text("ab");
        let end = buffer.insert_text(pos(0, 1), "\n");
        assert_eq!(buffer.lines(), &["a", "b"]);
        assert_eq!(end, pos(1, 0));
    }

    #[test]
    fn insert_empty_text_is_noop() {
        let mut buffer = Buffer::from_text("ab");
        assert_eq!(buffer.insert_text(pos(0, 1), ""), pos(0, 1));
        assert_eq!(buffer.text(), "ab");
    }

    #[test]
    fn text_range_within_and_across_lines() {
        let buffer = Buffer::from_text("abc\ndef\nghi");
        assert_eq!(buffer.text_range(pos(0, 1), pos(0, 3)), "bc");
        assert_eq!(buffer.text_range(pos(0, 2), pos(2, 1)), "c\ndef\ng");
        assert_eq!(buffer.text_range(pos(0, 3), pos(1, 0)), "\n");
    }

    #[test]
    fn delete_within_line() {
        let mut buffer = Buffer::from_text("abcdef");
        assert_eq!(buffer.delete_range(pos(0, 1), pos(0, 4)), "bcd");
        assert_eq!(buffer.text(), "aef");
    }

    #[test]
    fn delete_across_lines() {
        let mut buffer = Buffer::from_text("abc\ndef\nghi");
        assert_eq!(buffer.delete_range(pos(0, 2), pos(2, 1)), "c\ndef\ng");
        assert_eq!(buffer.text(), "abhi");
    }

    #[test]
    fn delete_newline_joins_lines() {
        let mut buffer = Buffer::from_text("ab\ncd");
        buffer.delete_range(pos(0, 2), pos(1, 0));
        assert_eq!(buffer.text(), "abcd");
    }

    #[test]
    fn delete_everything_leaves_one_empty_line() {
        let mut buffer = Buffer::from_text("ab\ncd");
        buffer.delete_range(pos(0, 0), pos(1, 2));
        assert_eq!(buffer.lines(), &[String::new()]);
    }

    #[test]
    fn insert_then_delete_round_trips() {
        let original = Buffer::from_text("one\ntwo");
        let mut buffer = original.clone();
        let end = buffer.insert_text(pos(1, 1), "X\nY\n");
        buffer.delete_range(pos(1, 1), end);
        assert_eq!(buffer, original);
    }
}
