/// A location in a buffer.
///
/// `col` is measured in `char`s (not bytes or display cells), so multi-byte
/// UTF-8 text is handled correctly. Converting to screen columns is the UI's job.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

impl Position {
    pub const fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }

    /// Returns a copy with the column replaced.
    pub const fn with_col(self, col: usize) -> Self {
        Self::new(self.row, col)
    }
}
