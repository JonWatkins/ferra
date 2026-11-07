//! `<C-a>`, `<C-x>`: adding to numbers in the text (see
//! [`core::increment`](crate::core::increment)).

use super::Editor;
use crate::core::increment::{self, NumberAt};
use crate::core::{Position, Range};

impl Editor {
    /// `<C-a>` (or `<C-x>`, with `delta` negative): adds `delta` to the
    /// number under or after the cursor on its line, leaving the cursor on
    /// its last char.
    pub fn increment(&mut self, delta: i128) -> Result<(), String> {
        let cursor = self.document.cursor();
        let at = self
            .add_to_number(cursor.row, cursor.col, 0, usize::MAX, delta)
            .ok_or("No number under or after the cursor")?;
        self.document
            .set_cursor(Position::new(cursor.row, at.end.saturating_sub(1)));
        Ok(())
    }

    /// Visual `<C-a>`, `<C-x>`: adds `delta` to the first number on each
    /// selected line, within the selection. With `progressive` (`g<C-a>`)
    /// the first number gets `delta`, the next twice that, and so on. The
    /// cursor goes to the start of the selection, as in Vim.
    pub fn increment_selection(&mut self, delta: i128, progressive: bool) -> Result<(), String> {
        let tab = self.options.tab_width;
        // Each row's selected chars.
        let parts: Vec<(usize, usize, usize)> = if let Some(block) = self.take_block() {
            block
                .rows()
                .map(|row| {
                    let line = self.document.buffer().line(row);
                    let (from, to) = block.chars_in(&line, tab);
                    (row, from, to)
                })
                .collect()
        } else if let Some(range) = self.take_selection() {
            match range {
                Range::Lines { first, last } => {
                    (first..=last).map(|row| (row, 0, usize::MAX)).collect()
                }
                Range::Chars { start, end } => (start.row..=end.row)
                    .map(|row| {
                        let from = if row == start.row { start.col } else { 0 };
                        let to = if row == end.row { end.col } else { usize::MAX };
                        (row, from, to)
                    })
                    .collect(),
            }
        } else {
            return Err("No selection".into());
        };
        let Some(&(first_row, first_col, _)) = parts.first() else {
            return Ok(());
        };

        let mut changed = 0;
        for (row, from, to) in parts {
            let step = if progressive {
                delta * (changed + 1)
            } else {
                delta
            };
            if self.add_to_number(row, from, from, to, step).is_some() {
                changed += 1;
            }
        }
        self.document
            .set_cursor(Position::new(first_row, first_col));
        if changed == 0 {
            return Err("No numbers in the selection".into());
        }
        Ok(())
    }

    /// Adds `delta` to the number at or after `col` on `row`, among chars
    /// `from..to`, changing only the number's chars. Returns where it now
    /// is.
    fn add_to_number(
        &mut self,
        row: usize,
        col: usize,
        from: usize,
        to: usize,
        delta: i128,
    ) -> Option<NumberAt> {
        let line = self.document.buffer().line(row).into_owned();
        let (new_line, at) = increment::add(&line, col, from, to, delta)?;
        // Only the number changed: the text before and after it is as it
        // was, so its old end is as far from the end of the line.
        let old_len = line.chars().count();
        let new_len = new_line.chars().count();
        let old_end = old_len - (new_len - at.end);
        let number: String = new_line
            .chars()
            .skip(at.start)
            .take(at.end - at.start)
            .collect();
        self.document
            .delete(Position::new(row, at.start), Position::new(row, old_end));
        self.document.insert(Position::new(row, at.start), &number);
        Some(at)
    }
}
