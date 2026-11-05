use super::Position;

/// The text being edited, stored as a list of lines (without line endings).
///
/// A buffer always contains at least one (possibly empty) line. Positions
/// passed to its methods must be valid (`row < line_count()`,
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

    /// Returns the whole buffer joined with `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The character at `pos`, or `None` at (or past) the end of the line.
    pub fn char_at(&self, pos: Position) -> Option<char> {
        self.lines.get(pos.row)?.chars().nth(pos.col)
    }

    pub fn insert_char(&mut self, at: Position, c: char) {
        let line = &mut self.lines[at.row];
        line.insert(byte_index(line, at.col), c);
    }

    /// Inserts `s` (which must not contain newlines) at `at`.
    pub fn insert_str(&mut self, at: Position, s: &str) {
        debug_assert!(!s.contains('\n'), "use split_line for newlines");
        let line = &mut self.lines[at.row];
        line.insert_str(byte_index(line, at.col), s);
    }

    /// Splits the line at `at`, moving the remainder onto a new line below.
    pub fn split_line(&mut self, at: Position) {
        let line = &mut self.lines[at.row];
        let rest = line.split_off(byte_index(line, at.col));
        self.lines.insert(at.row + 1, rest);
    }

    /// Removes and returns the character at `at`, or `None` at the end of the line.
    pub fn remove_char(&mut self, at: Position) -> Option<char> {
        let line = &mut self.lines[at.row];
        let idx = byte_index(line, at.col);
        (idx < line.len()).then(|| line.remove(idx))
    }

    /// Appends the line below `row` onto `row`. Returns `false` if `row` is the last line.
    pub fn join_with_next(&mut self, row: usize) -> bool {
        if row + 1 >= self.lines.len() {
            return false;
        }
        let next = self.lines.remove(row + 1);
        self.lines[row].push_str(&next);
        true
    }

    /// Inserts a new line so that it becomes line `row`.
    pub fn insert_line(&mut self, row: usize, text: String) {
        self.lines.insert(row, text);
    }

    /// Removes line `row` and returns its contents. Removing the only line
    /// leaves a single empty line, preserving the "at least one line" invariant.
    pub fn remove_line(&mut self, row: usize) -> String {
        if self.lines.len() == 1 {
            return std::mem::take(&mut self.lines[0]);
        }
        self.lines.remove(row)
    }
}

/// Converts a `char` index into a byte index within `s`.
fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or(s.len(), |(byte, _)| byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_buffer_has_one_empty_line() {
        assert_eq!(Buffer::default().lines(), &[String::new()]);
    }

    #[test]
    fn insert_char_handles_multibyte_text() {
        let mut buffer = Buffer::from_text("héllo");
        buffer.insert_char(Position::new(0, 2), 'X');
        assert_eq!(buffer.text(), "héXllo");
    }

    #[test]
    fn insert_str_inserts_at_char_index() {
        let mut buffer = Buffer::from_text("ac");
        buffer.insert_str(Position::new(0, 1), "bb");
        assert_eq!(buffer.text(), "abbc");
    }

    #[test]
    fn split_and_join_are_inverse() {
        let mut buffer = Buffer::from_text("hello world");
        buffer.split_line(Position::new(0, 5));
        assert_eq!(buffer.lines(), &["hello", " world"]);
        assert!(buffer.join_with_next(0));
        assert_eq!(buffer.text(), "hello world");
    }

    #[test]
    fn remove_char_returns_removed_char() {
        let mut buffer = Buffer::from_text("aé");
        assert_eq!(buffer.remove_char(Position::new(0, 1)), Some('é'));
        assert_eq!(buffer.remove_char(Position::new(0, 1)), None);
        assert_eq!(buffer.text(), "a");
    }

    #[test]
    fn join_on_last_line_is_noop() {
        assert!(!Buffer::from_text("only").join_with_next(0));
    }

    #[test]
    fn char_at_returns_none_past_end() {
        let buffer = Buffer::from_text("ab");
        assert_eq!(buffer.char_at(Position::new(0, 1)), Some('b'));
        assert_eq!(buffer.char_at(Position::new(0, 2)), None);
        assert_eq!(buffer.char_at(Position::new(5, 0)), None);
    }

    #[test]
    fn remove_line_keeps_at_least_one_line() {
        let mut buffer = Buffer::from_text("a\nb");
        assert_eq!(buffer.remove_line(0), "a");
        assert_eq!(buffer.remove_line(0), "b");
        assert_eq!(buffer.lines(), &[String::new()]);
    }

    #[test]
    fn insert_line_places_line_at_row() {
        let mut buffer = Buffer::from_text("a\nc");
        buffer.insert_line(1, "b".into());
        assert_eq!(buffer.text(), "a\nb\nc");
    }
}
