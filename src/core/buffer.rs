use std::borrow::Cow;

use ropey::Rope;

use super::Position;

/// The text being edited, stored in a rope.
///
/// A rope keeps the text in a balanced tree of chunks, so inserting,
/// deleting and finding a line are O(log n) in the size of the file, rather
/// than O(n) as with one string or a list of lines. Lines are separated by
/// `\n` only (file line endings are handled by [`TextFormat`](super::TextFormat)),
/// and a buffer always has at least one (possibly empty) line.
///
/// All mutation goes through two primitives, [`insert_text`](Self::insert_text)
/// and [`delete_range`](Self::delete_range), which makes every edit
/// expressible as an invertible [`Change`](super::Change) for undo.
///
/// Positions passed in must be valid (`row < line_count()`,
/// `col <= line_len(row)`); keeping them valid is the caller's job.
///
/// Every edit is also logged as a [`TextEdit`], so a syntax tree built for
/// earlier text can be updated rather than rebuilt (see
/// [`edits_since`](Self::edits_since)).
#[derive(Debug)]
pub struct Buffer {
    rope: Rope,
    /// Tells buffers apart, so an edit log is only ever applied to data
    /// built from the same buffer. A clone gets a new one.
    id: u64,
    /// The most recent edits, oldest first.
    edits: Vec<TextEdit>,
    /// How many edits were made before `edits[0]` (dropped from the log).
    edits_dropped: u64,
}

/// One change to the text, in the byte offsets and (row, byte column)
/// points tree-sitter's `InputEdit` wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: (usize, usize),
    pub old_end: (usize, usize),
    pub new_end: (usize, usize),
}

/// The most edits kept; consumers further behind start over.
const MAX_LOGGED_EDITS: usize = 4096;

fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Default for Buffer {
    fn default() -> Self {
        Self::from_rope(Rope::new())
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        Self::from_rope(self.rope.clone())
    }
}

/// Buffers are equal when their text is.
impl PartialEq for Buffer {
    fn eq(&self, other: &Self) -> bool {
        self.rope == other.rope
    }
}

impl Eq for Buffer {}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text))
    }

    fn from_rope(rope: Rope) -> Self {
        Self {
            rope,
            id: next_id(),
            edits: Vec::new(),
            edits_dropped: 0,
        }
    }

    /// Identifies this buffer (see [`edits_since`](Self::edits_since)).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// How many edits have been made, counting from 0 when the buffer was
    /// made. Pass it to [`edits_since`](Self::edits_since) later.
    pub fn edit_seq(&self) -> u64 {
        self.edits_dropped + self.edits.len() as u64
    }

    /// The edits made since [`edit_seq`](Self::edit_seq) returned `seq`,
    /// oldest first, or `None` if they're no longer all logged.
    pub fn edits_since(&self, seq: u64) -> Option<&[TextEdit]> {
        let skip = seq.checked_sub(self.edits_dropped)?;
        self.edits.get(usize::try_from(skip).ok()?..)
    }

    fn log_edit(&mut self, edit: TextEdit) {
        if self.edits.len() == MAX_LOGGED_EDITS {
            let half = MAX_LOGGED_EDITS / 2;
            self.edits.drain(..half);
            self.edits_dropped += half as u64;
        }
        self.edits.push(edit);
    }

    /// Length in bytes (UTF-8).
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// The byte offset where `row` starts (`len_bytes` past the end).
    pub fn row_to_byte(&self, row: usize) -> usize {
        if row >= self.line_count() {
            return self.len_bytes();
        }
        self.rope.line_to_byte(row)
    }

    /// The text from byte `byte` to the end of the rope chunk holding it,
    /// for reading the text piece by piece (empty at the end).
    pub fn chunk_from_byte(&self, byte: usize) -> &[u8] {
        if byte >= self.len_bytes() {
            return &[];
        }
        let (chunk, start, _, _) = self.rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - start..]
    }

    /// The text of bytes `range`, in pieces. The range must fall on
    /// character boundaries.
    pub fn byte_chunks(&self, range: std::ops::Range<usize>) -> impl Iterator<Item = &[u8]> {
        self.rope.byte_slice(range).chunks().map(str::as_bytes)
    }

    /// The text of bytes `range` (on character boundaries).
    pub fn byte_text(&self, range: std::ops::Range<usize>) -> String {
        self.rope.byte_slice(range).to_string()
    }

    /// The (row, byte column) point and byte offset of `pos`.
    fn point(&self, pos: Position) -> ((usize, usize), usize) {
        let byte = self.rope.char_to_byte(self.char_index(pos));
        ((pos.row, byte - self.rope.line_to_byte(pos.row)), byte)
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Total length in chars, including line breaks.
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Line `row` without its line break. Borrowed when the line lies in a
    /// single rope chunk (almost always), otherwise copied.
    pub fn line(&self, row: usize) -> Cow<'_, str> {
        let slice = self.rope.line(row);
        let slice = slice.slice(..self.line_len(row));
        match slice.as_str() {
            Some(text) => Cow::Borrowed(text),
            None => Cow::Owned(slice.to_string()),
        }
    }

    /// Length of the given line in `char`s, excluding its line break.
    pub fn line_len(&self, row: usize) -> usize {
        let line = self.rope.line(row);
        let len = line.len_chars();
        if len > 0 && line.char(len - 1) == '\n' {
            len - 1
        } else {
            len
        }
    }

    /// The position just past the last character of `row`.
    pub fn line_end(&self, row: usize) -> Position {
        Position::new(row, self.line_len(row))
    }

    /// Lines `first..=last` joined with `\n` (no trailing line break).
    pub fn lines_text(&self, first: usize, last: usize) -> String {
        self.text_range(Position::new(first, 0), self.line_end(last))
    }

    /// The whole buffer as one string.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The character at `pos`, or `None` at (or past) the end of the line.
    pub fn char_at(&self, pos: Position) -> Option<char> {
        if pos.row >= self.line_count() || pos.col >= self.line_len(pos.row) {
            return None;
        }
        Some(self.rope.char(self.char_index(pos)))
    }

    /// The text between `start` (inclusive) and `end` (exclusive).
    pub fn text_range(&self, start: Position, end: Position) -> String {
        debug_assert!(start <= end, "range must be ordered");
        self.rope
            .slice(self.char_index(start)..self.char_index(end))
            .to_string()
    }

    /// Inserts `text` (which may span lines) at `at`. Returns the position
    /// just after the inserted text.
    pub fn insert_text(&mut self, at: Position, text: &str) -> Position {
        let (start, start_byte) = self.point(at);
        let index = self.char_index(at);
        self.rope.insert(index, text);
        let end = self.position_of(index + text.chars().count());
        let (new_end, new_end_byte) = self.point(end);
        self.log_edit(TextEdit {
            start_byte,
            old_end_byte: start_byte,
            new_end_byte,
            start,
            old_end: start,
            new_end,
        });
        end
    }

    /// Removes the text between `start` (inclusive) and `end` (exclusive) and
    /// returns it.
    pub fn delete_range(&mut self, start: Position, end: Position) -> String {
        let (start_point, start_byte) = self.point(start);
        let (old_end, old_end_byte) = self.point(end);
        let range = self.char_index(start)..self.char_index(end);
        let removed = self.rope.slice(range.clone()).to_string();
        self.rope.remove(range);
        self.log_edit(TextEdit {
            start_byte,
            old_end_byte,
            new_end_byte: start_byte,
            start: start_point,
            old_end,
            new_end: start_point,
        });
        removed
    }

    /// Converts a position to an index into the rope's chars.
    fn char_index(&self, pos: Position) -> usize {
        self.rope.line_to_char(pos.row) + pos.col
    }

    /// Converts an index into the rope's chars to a position.
    fn position_of(&self, index: usize) -> Position {
        let row = self.rope.char_to_line(index);
        Position::new(row, index - self.rope.line_to_char(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn lines(buffer: &Buffer) -> Vec<String> {
        (0..buffer.line_count())
            .map(|row| buffer.line(row).into_owned())
            .collect()
    }

    #[test]
    fn edits_are_logged_in_bytes_and_points() {
        let mut buffer = Buffer::from_text("é\nab");
        let seq = buffer.edit_seq();
        buffer.insert_text(pos(1, 1), "x\ny");
        buffer.delete_range(pos(0, 1), pos(1, 0));
        let edits = buffer.edits_since(seq).unwrap();
        assert_eq!(
            edits[0],
            TextEdit {
                start_byte: 4,
                old_end_byte: 4,
                new_end_byte: 7,
                start: (1, 1),
                old_end: (1, 1),
                new_end: (2, 1),
            }
        );
        assert_eq!(
            edits[1],
            TextEdit {
                start_byte: 2,
                old_end_byte: 3,
                new_end_byte: 2,
                start: (0, 2),
                old_end: (1, 0),
                new_end: (0, 2),
            },
            "é is two bytes"
        );
        assert_eq!(buffer.edits_since(buffer.edit_seq()), Some(&[][..]));
    }

    #[test]
    fn old_edits_are_dropped_and_clones_start_afresh() {
        let mut buffer = Buffer::default();
        for _ in 0..MAX_LOGGED_EDITS + 1 {
            buffer.insert_text(pos(0, 0), "x");
        }
        assert_eq!(buffer.edit_seq(), MAX_LOGGED_EDITS as u64 + 1);
        assert!(buffer.edits_since(0).is_none());
        assert!(buffer.edits_since(buffer.edit_seq() - 10).is_some());

        let clone = buffer.clone();
        assert_ne!(clone.id(), buffer.id());
        assert_eq!(clone, buffer, "same text");
    }

    #[test]
    fn reading_bytes() {
        let buffer = Buffer::from_text("aé\nb");
        assert_eq!(buffer.len_bytes(), 5);
        assert_eq!(buffer.row_to_byte(1), 4);
        assert_eq!(buffer.row_to_byte(9), 5);
        assert_eq!(buffer.chunk_from_byte(1), "é\nb".as_bytes());
        assert_eq!(buffer.chunk_from_byte(5), b"");
        assert_eq!(buffer.byte_text(1..3), "é");
        assert_eq!(buffer.byte_chunks(0..5).flatten().count(), 5);
    }

    #[test]
    fn default_buffer_has_one_empty_line() {
        let buffer = Buffer::default();
        assert_eq!(lines(&buffer), [""]);
        assert_eq!(buffer.line_len(0), 0);
    }

    #[test]
    fn lines_exclude_line_breaks() {
        let buffer = Buffer::from_text("ab\n\ncd\n");
        assert_eq!(lines(&buffer), ["ab", "", "cd", ""]);
        assert_eq!(buffer.line_len(0), 2);
        assert_eq!(buffer.line_len(3), 0);
    }

    #[test]
    fn line_len_counts_chars() {
        let buffer = Buffer::from_text("héllo 日本");
        assert_eq!(buffer.line_len(0), 8);
    }

    #[test]
    fn char_at_returns_none_past_end() {
        let buffer = Buffer::from_text("ab\ncd");
        assert_eq!(buffer.char_at(pos(0, 1)), Some('b'));
        assert_eq!(
            buffer.char_at(pos(0, 2)),
            None,
            "the line break isn't a char"
        );
        assert_eq!(buffer.char_at(pos(1, 0)), Some('c'));
        assert_eq!(buffer.char_at(pos(5, 0)), None);
    }

    #[test]
    fn lines_text_joins_a_range() {
        let buffer = Buffer::from_text("a\nb\nc");
        assert_eq!(buffer.lines_text(0, 1), "a\nb");
        assert_eq!(buffer.lines_text(2, 2), "c");
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
        assert_eq!(lines(&buffer), ["a", "b"]);
        assert_eq!(end, pos(1, 0));
    }

    #[test]
    fn insert_at_end_of_buffer() {
        let mut buffer = Buffer::from_text("a");
        let end = buffer.insert_text(pos(0, 1), "\n");
        assert_eq!(end, pos(1, 0));
        assert_eq!(buffer.line_count(), 2);
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
        assert_eq!(lines(&buffer), [""]);
    }

    #[test]
    fn insert_then_delete_round_trips() {
        let original = Buffer::from_text("one\ntwo");
        let mut buffer = original.clone();
        let end = buffer.insert_text(pos(1, 1), "X\nY\n");
        buffer.delete_range(pos(1, 1), end);
        assert_eq!(buffer, original);
    }

    #[test]
    fn large_buffers_stay_fast_to_edit() {
        // 200k lines: a Vec<String> splice per edit would be noticeably slow;
        // with a rope this is instant. Mostly a smoke test for big files.
        let text = "line of text\n".repeat(200_000);
        let mut buffer = Buffer::from_text(&text);
        for i in 0..1_000 {
            let at = pos(i * 100, 0);
            let end = buffer.insert_text(at, "x\n");
            buffer.delete_range(at, end);
        }
        assert_eq!(buffer.line_count(), 200_001);
        assert_eq!(buffer.line(199_999), "line of text");
    }
}
