//! Visual block mode (`<C-v>`): operators on a rectangle of text, `I`/`A`
//! on every row, and pasting a block as a column.
//!
//! The shape comes from [`crate::core::Block`], in display columns. Edits
//! go row by row, bottom to top, so earlier rows' positions stay valid.
//! A tab or wide character cut by the block's edge leaves spaces for its
//! cells outside the block (and yanks as spaces for those inside), so the
//! text around the block stays where it was on screen.

use super::registers::{Register, StoreKind};
use super::transform::CaseChange;
use super::transform::whitespace;
use super::{Editor, Mode, Operator, SelectionKind};
use crate::core::columns::{char_cells, char_col_at, display_col, display_width};
use crate::core::{Block, Position, Range};

/// An `I`, `A` or `c` on a block, waiting for insert mode to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInsert {
    /// Where typing began on the first row.
    start: Position,
    /// The other rows that get the text.
    rows: Vec<usize>,
    target: Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// At this display column (`I`, `c`).
    Column(usize),
    /// At this display column, padding short rows with spaces (`A`).
    PaddedColumn(usize),
    /// At the end of each row (`$A`).
    LineEnd,
}

impl Editor {
    /// The selected block, in visual block mode (or at a search prompt
    /// opened from it, which keeps the selection).
    pub fn selection_block(&self) -> Option<Block> {
        let selection = self.selection?;
        let showing = matches!(self.mode(), Mode::Visual | Mode::Command);
        if selection.kind != SelectionKind::Block || !showing {
            return None;
        }
        let cursor = self.document.cursor();
        Some(Block::between(
            self.document.buffer(),
            (selection.anchor.row, selection.anchor.col),
            (cursor.row, cursor.col),
            self.block_to_eol,
            self.options.tab_width,
        ))
    }

    /// Takes the selected block and returns to normal mode.
    pub fn take_block(&mut self) -> Option<Block> {
        let block = self.selection_block()?;
        self.note_taken_shape();
        self.set_mode(Mode::Normal);
        Some(block)
    }

    /// After a motion in block mode: `$` makes every row run to its end,
    /// vertical moves keep that, and anything else ends it.
    pub fn note_block_motion(&mut self, to_line_end: bool, vertical: bool) {
        if self.selection_block().is_some() {
            self.block_to_eol = to_line_end || (vertical && self.block_to_eol);
        }
    }

    /// `O` in block mode: the cursor moves to the other side of the block on
    /// its own row (and the anchor to the cursor's side on its row).
    pub fn swap_block_sides(&mut self) {
        let (Some(block), Some(selection)) = (self.selection_block(), self.selection.as_mut())
        else {
            return;
        };
        let tab = self.options.tab_width;
        let cursor = self.document.cursor();
        let buffer = self.document.buffer();
        let cursor_line = buffer.line(cursor.row);
        let anchor_line = buffer.line(selection.anchor.row);
        let cursor_on_left = char_cells(&cursor_line, cursor.col, tab).0 <= block.left;
        let (cursor_col, anchor_col) = if cursor_on_left {
            (block.right, block.left)
        } else {
            (block.left, block.right)
        };
        selection.anchor.col = char_col_at(&anchor_line, anchor_col, tab);
        let col = char_col_at(&cursor_line, cursor_col, tab);
        self.document.set_cursor(cursor.with_col(col));
        self.block_to_eol = false;
    }

    /// The text of each row of `block`.
    pub fn block_text(&self, block: Block) -> Vec<String> {
        let tab = self.options.tab_width;
        block
            .rows()
            .map(|row| block.text_in(&self.document.buffer().line(row), tab))
            .collect()
    }

    /// Applies `operator` to a block. `c` leaves insert mode waiting to copy
    /// the typed text to every row.
    pub fn apply_block_operator(
        &mut self,
        operator: Operator,
        block: Block,
        register: Option<char>,
    ) -> Result<(), String> {
        let tab = self.options.tab_width;
        match operator {
            Operator::Comment => return self.toggle_comment(block.first, block.last),
            Operator::Reindent => return self.reindent_lines(block.first, block.last),
            Operator::Format { keep_cursor } => {
                self.format_text(block.first, block.last, keep_cursor);
                return Ok(());
            }
            Operator::Indent | Operator::Outdent => {
                self.shift_block(block, operator == Operator::Indent);
                return Ok(());
            }
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                let change = match operator {
                    Operator::Lowercase => CaseChange::Lower,
                    Operator::Uppercase => CaseChange::Upper,
                    _ => CaseChange::Toggle,
                };
                for row in block.rows() {
                    let (start, end) = block.chars_in(&self.document.buffer().line(row), tab);
                    if start < end {
                        let range = Range::Chars {
                            start: Position::new(row, start),
                            end: Position::new(row, end),
                        };
                        self.change_case(range, change);
                    }
                }
                self.set_cursor_to_block_start(block);
                return Ok(());
            }
            Operator::Delete | Operator::Change | Operator::Yank => {}
        }

        let rows = self.block_text(block);
        let kind = if operator == Operator::Yank {
            StoreKind::Yank
        } else {
            StoreKind::Delete
        };
        if rows.iter().any(|r| !r.is_empty()) {
            self.registers
                .store(register, Register::blockwise(&rows), kind)?;
        }
        if operator == Operator::Yank {
            self.set_cursor_to_block_start(block);
            return Ok(());
        }

        // Rows that reach the block, decided before deleting from them.
        let reached: Vec<usize> = block
            .rows()
            .filter(|&row| block.reaches(&self.document.buffer().line(row), tab))
            .collect();
        self.delete_block(block);
        if operator == Operator::Change {
            let target = Target::Column(block.left);
            self.begin_block_insert(block.first, reached, target);
        }
        Ok(())
    }

    /// `>` / `<` on a block: shifts the text from the block's left edge by
    /// one indent, as Vim does. The whitespace starting there grows or
    /// shrinks (`<` never takes whitespace from left of the block) and is
    /// rewritten as tabs or spaces as the buffer indents. Rows that stop short of
    /// the block, or have nothing but whitespace in it, are left alone.
    fn shift_block(&mut self, block: Block, indent: bool) {
        let tab = self.options.tab_width.max(1);
        let unit = self.indent_width();
        let expand_tab = !self.indent_with_tabs();
        for row in block.rows() {
            let line = self.document.buffer().line(row).into_owned();
            if !block.reaches(&line, tab) {
                continue;
            }
            let start = char_col_at(&line, block.left, tab);
            let blanks = line
                .chars()
                .skip(start)
                .take_while(|c| matches!(c, ' ' | '\t'))
                .count();
            let end = start + blanks;
            if end == line.chars().count() {
                continue;
            }
            let from = display_col(&line, start, tab);
            let to = display_col(&line, end, tab);
            let new_to = if indent {
                to + unit
            } else if to > block.left {
                to.saturating_sub(unit).max(block.left)
            } else {
                continue;
            };
            let space = whitespace(from, new_to, tab, expand_tab);
            if line.chars().skip(start).take(blanks).ne(space.chars()) {
                self.document
                    .delete(Position::new(row, start), Position::new(row, end));
                self.document.insert(Position::new(row, start), &space);
            }
        }
        self.set_cursor_to_block_start(block);
    }

    /// `I` (or `A` with `append`) on a block: types on the first row, then
    /// copies the text to the rest when insert mode ends. `I` skips rows that
    /// stop short of the block; `A` pads them with spaces, or with `$`
    /// appends to each row's end.
    pub fn block_insert(&mut self, block: Block, append: bool) {
        let tab = self.options.tab_width;
        let (target, rows): (Target, Vec<usize>) = if !append {
            let rows = block
                .rows()
                .filter(|&row| block.reaches(&self.document.buffer().line(row), tab))
                .collect();
            (Target::Column(block.left), rows)
        } else if block.to_eol {
            (Target::LineEnd, block.rows().collect())
        } else {
            (
                Target::PaddedColumn(block.right + 1),
                block.rows().collect(),
            )
        };
        self.note_taken_shape();
        self.set_mode(Mode::Normal);
        self.begin_block_insert(block.first, rows, target);
    }

    fn begin_block_insert(&mut self, first: usize, rows: Vec<usize>, target: Target) {
        let start = self.prepare_row(first, target);
        let rows = rows.into_iter().filter(|&row| row != first).collect();
        self.document.set_cursor(start);
        self.set_mode(Mode::Insert);
        self.block_insert = Some(BlockInsert {
            start,
            rows,
            target,
        });
    }

    /// Where `target` is on `row`, padding the row with spaces first if a
    /// padded column is past its end, or splitting a tab the column is in
    /// the middle of.
    fn prepare_row(&mut self, row: usize, target: Target) -> Position {
        let tab = self.options.tab_width;
        let line = self.document.buffer().line(row).to_string();
        let len = line.chars().count();
        let col = match target {
            Target::LineEnd => len,
            Target::Column(col) | Target::PaddedColumn(col) => {
                let width = display_width(&line, tab);
                if width < col {
                    if matches!(target, Target::Column(_)) {
                        return Position::new(row, len);
                    }
                    let pad = " ".repeat(col - width);
                    self.document.insert(Position::new(row, len), &pad);
                    len + pad.len()
                } else {
                    self.split_tab_at(row, &line, col)
                }
            }
        };
        Position::new(row, col)
    }

    /// The character column at display column `col` on `row` (whose text is
    /// `line`). A tab covering `col` but starting before it becomes spaces,
    /// so `col` is a character boundary.
    fn split_tab_at(&mut self, row: usize, line: &str, col: usize) -> usize {
        let tab = self.options.tab_width;
        let at = char_col_at(line, col, tab);
        let (first, last) = char_cells(line, at, tab);
        if first == col || line.chars().nth(at) != Some('\t') {
            return at;
        }
        let spaces = " ".repeat(last - first + 1);
        self.document
            .delete(Position::new(row, at), Position::new(row, at + 1));
        self.document.insert(Position::new(row, at), &spaces);
        at + (col - first)
    }

    /// Called when insert mode ends: copies the text typed on the first row
    /// to the other rows. Typing that left the row (a line break, moving
    /// away) copies nothing, as in Vim.
    pub(super) fn finish_block_insert(&mut self, insert: BlockInsert) {
        let cursor = self.document.cursor();
        if cursor.row != insert.start.row || cursor.col <= insert.start.col {
            return;
        }
        let text: String = self
            .document
            .buffer()
            .line(cursor.row)
            .chars()
            .skip(insert.start.col)
            .take(cursor.col - insert.start.col)
            .collect();
        for &row in insert.rows.iter().rev() {
            let at = self.prepare_row(row, insert.target);
            self.document.insert(at, &text);
        }
        // Back to the first inserted char (set_mode then steps back one).
        self.document
            .set_cursor(insert.start.with_col(insert.start.col + 1));
    }

    fn delete_block(&mut self, block: Block) {
        let tab = self.options.tab_width;
        for row in block.rows().rev() {
            let part = block.row(&self.document.buffer().line(row), tab);
            if !part.is_empty() {
                let start = Position::new(row, part.start);
                self.document.delete(start, Position::new(row, part.end));
                let remainder = part.remainder();
                if !remainder.is_empty() {
                    self.document.insert(start, &remainder);
                }
            }
        }
        self.set_cursor_to_block_start(block);
    }

    fn set_cursor_to_block_start(&mut self, block: Block) {
        let line = self.document.buffer().line(block.first);
        let col = char_col_at(&line, block.left, self.options.tab_width);
        self.document.set_cursor(Position::new(block.first, col));
    }

    /// Pastes a blockwise register as a column: row `i` of the block goes
    /// into line `cursor + i` at the cursor's column (after it with
    /// `after`), padding short lines and adding lines at the end as needed.
    /// With a count, each row is repeated, padded to the block's width.
    pub fn paste_block(&mut self, register: &Register, after: bool, count: usize) {
        let tab = self.options.tab_width;
        let cursor = self.document.cursor();
        let first_line = self.document.buffer().line(cursor.row).to_string();
        let col = if after && !first_line.is_empty() {
            char_cells(&first_line, cursor.col, tab).1 + 1
        } else {
            display_col(&first_line, cursor.col, tab)
        };
        let pieces: Vec<&str> = register.text.split('\n').collect();
        let width = pieces
            .iter()
            .map(|p| display_width(p, tab))
            .max()
            .unwrap_or(0);

        for (i, piece) in pieces.iter().enumerate() {
            let row = cursor.row + i;
            let buffer = self.document.buffer();
            if row >= buffer.line_count() {
                let end = buffer.line_end(buffer.line_count() - 1);
                self.document.insert(end, "\n");
            }
            let line = self.document.buffer().line(row).to_string();
            let len = line.chars().count();
            let line_width = display_width(&line, tab);
            let (at, mut text) = if line_width < col {
                (len, " ".repeat(col - line_width))
            } else {
                (char_col_at(&line, col, tab), String::new())
            };
            let follows = at < len;
            let padded = format!("{piece}{}", " ".repeat(width - display_width(piece, tab)));
            for k in 0..count.max(1) {
                let last = k + 1 == count.max(1);
                text.push_str(if last && !follows { piece } else { &padded });
            }
            self.document.insert(Position::new(row, at), &text);
        }
        let line = self.document.buffer().line(cursor.row);
        let col = char_col_at(&line, col, tab);
        self.document.set_cursor(Position::new(cursor.row, col));
    }

    /// Replaces each row of a block with `text` (`p` in block mode with a
    /// one-line register), on rows that reach the block.
    pub fn fill_block(&mut self, block: Block, text: &str) {
        let tab = self.options.tab_width;
        let reached: Vec<usize> = block
            .rows()
            .filter(|&row| block.reaches(&self.document.buffer().line(row), tab))
            .collect();
        self.delete_block(block);
        for row in reached.into_iter().rev() {
            let line = self.document.buffer().line(row);
            let at = char_col_at(&line, block.left, tab);
            self.document.insert(Position::new(row, at), text);
        }
        self.set_cursor_to_block_start(block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn text(e: &Editor) -> String {
        e.document.buffer().text()
    }

    /// An editor selecting the block from `anchor` to `cursor`.
    fn selecting(content: &str, anchor: Position, cursor: Position) -> Editor {
        let mut e = Editor::with_document(Document::with_text(content));
        e.document.set_cursor(anchor);
        e.toggle_visual(SelectionKind::Block);
        e.document.set_cursor(cursor);
        e
    }

    fn type_text(e: &mut Editor, s: &str) {
        s.chars().for_each(|c| e.insert_text_char(c));
    }

    #[test]
    fn yank_and_delete_a_block() {
        let mut e = selecting("abcd\nefgh\nijkl", pos(0, 1), pos(2, 2));
        let block = e.take_block().unwrap();
        assert_eq!(e.block_text(block), ["bc", "fg", "jk"]);
        e.apply_block_operator(Operator::Delete, block, None)
            .unwrap();
        assert_eq!(text(&e), "ad\neh\nil");
        assert_eq!(e.document.cursor(), pos(0, 1));
        let unnamed = e.registers.get(None).unwrap();
        assert!(unnamed.blockwise);
        assert_eq!(unnamed.text, "bc\nfg\njk");
    }

    #[test]
    fn short_lines_lose_only_what_they_have() {
        let mut e = selecting("abcd\na\nabcd", pos(0, 1), pos(2, 2));
        let block = e.take_block().unwrap();
        e.apply_block_operator(Operator::Delete, block, None)
            .unwrap();
        assert_eq!(text(&e), "ad\na\nad");
    }

    #[test]
    fn to_eol_blocks_take_each_row_to_its_end() {
        let mut e = selecting("abcd\nab\nabcdef", pos(0, 1), pos(2, 1));
        e.note_block_motion(true, false);
        let block = e.take_block().unwrap();
        e.apply_block_operator(Operator::Delete, block, None)
            .unwrap();
        assert_eq!(text(&e), "a\na\na");
    }

    #[test]
    fn insert_copies_to_rows_that_reach_the_block() {
        let mut e = selecting("abc\n\nabc", pos(0, 1), pos(2, 1));
        let block = e.selection_block().unwrap();
        e.block_insert(block, false);
        assert_eq!(e.mode(), Mode::Insert);
        type_text(&mut e, "XY");
        e.set_mode(Mode::Normal);
        assert_eq!(text(&e), "aXYbc\n\naXYbc", "the empty row is skipped");
        assert_eq!(e.document.cursor(), pos(0, 1));
    }

    #[test]
    fn append_pads_short_rows() {
        let mut e = selecting("abc\na\nabc", pos(0, 1), pos(2, 2));
        let block = e.selection_block().unwrap();
        e.block_insert(block, true);
        type_text(&mut e, "|");
        e.set_mode(Mode::Normal);
        assert_eq!(text(&e), "abc|\na  |\nabc|");
    }

    #[test]
    fn dollar_append_goes_to_each_end() {
        let mut e = selecting("abc\na\nabcde", pos(0, 0), pos(2, 0));
        e.note_block_motion(true, false);
        let block = e.selection_block().unwrap();
        e.block_insert(block, true);
        type_text(&mut e, ";");
        e.set_mode(Mode::Normal);
        assert_eq!(text(&e), "abc;\na;\nabcde;");
    }

    #[test]
    fn change_replaces_the_block_on_every_row() {
        let mut e = selecting("a12b\na34b", pos(0, 1), pos(1, 2));
        let block = e.take_block().unwrap();
        e.apply_block_operator(Operator::Change, block, None)
            .unwrap();
        type_text(&mut e, "x");
        e.set_mode(Mode::Normal);
        assert_eq!(text(&e), "axb\naxb");
    }

    #[test]
    fn a_line_break_while_inserting_copies_nothing() {
        let mut e = selecting("ab\nab", pos(0, 0), pos(1, 0));
        let block = e.selection_block().unwrap();
        e.block_insert(block, false);
        e.document.insert_char('\n');
        e.set_mode(Mode::Normal);
        assert_eq!(text(&e), "\nab\nab");
    }

    #[test]
    fn paste_a_block_as_a_column() {
        let mut e = Editor::with_document(Document::with_text("12\n34\n"));
        e.document.set_cursor(pos(0, 0));
        e.paste_block(
            &Register::blockwise(&["ab".into(), "c".into(), "de".into()]),
            true,
            1,
        );
        assert_eq!(text(&e), "1ab2\n3c 4\n de");
        assert_eq!(e.document.cursor(), pos(0, 1));
    }

    #[test]
    fn shifting_a_block_moves_text_from_its_left_edge() {
        let mut e = selecting("x  y\nx  y", pos(0, 2), pos(1, 2));
        let block = e.take_block().unwrap();
        e.apply_block_operator(Operator::Indent, block, None)
            .unwrap();
        assert_eq!(text(&e), "x      y\nx      y");
        let mut e = selecting("x      y", pos(0, 2), pos(0, 2));
        let block = e.take_block().unwrap();
        e.apply_block_operator(Operator::Outdent, block, None)
            .unwrap();
        assert_eq!(text(&e), "x  y", "the space left of the block stays");
    }

    #[test]
    fn swapping_sides_keeps_the_rows() {
        let mut e = selecting("abcd\nabcd", pos(0, 0), pos(1, 2));
        e.swap_block_sides();
        assert_eq!(e.document.cursor(), pos(1, 0));
        assert_eq!(e.selection().unwrap().anchor, pos(0, 2));
    }
}
