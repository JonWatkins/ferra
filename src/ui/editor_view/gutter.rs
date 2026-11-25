//! The gutter on the left of the editor pane: the fold column
//! (`fold-column`), then the line numbers.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use crate::config::{EditorOptions, LineNumbers};
use crate::core::fold::Mark;

/// One cell of padding on each side of the number.
const PADDING: usize = 2;

/// The line numbers' width in cells for a buffer with `line_count` lines.
pub fn width(line_count: usize, mode: LineNumbers) -> u16 {
    crate::editor::windows::numbers_width(line_count, mode)
}

/// The fold column's width in cells.
pub fn fold_width(options: &EditorOptions) -> u16 {
    crate::editor::windows::fold_column_width(options)
}

/// What the fold column shows beside a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FoldMark {
    #[default]
    None,
    /// The first line of a closed fold.
    Closed,
    /// The first line of an open fold.
    Open,
    /// A line inside an open fold.
    Inside,
}

impl From<Mark> for FoldMark {
    fn from(mark: Mark) -> Self {
        match mark {
            Mark::None => Self::None,
            Mark::Head => Self::Open,
            Mark::Inside => Self::Inside,
        }
    }
}

impl FoldMark {
    /// The mark for a wrapped line's later rows.
    pub fn continued(self) -> Self {
        match self {
            Self::Open | Self::Inside => Self::Inside,
            Self::None | Self::Closed => Self::None,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::None => " ",
            Self::Closed => "▸",
            Self::Open => "▾",
            Self::Inside => "│",
        }
    }
}

/// Draws one row's fold column cell at `area` (one line high).
pub fn render_fold(buf: &mut Buffer, area: Rect, mark: FoldMark, style: Style) {
    if area.width == 0 {
        return;
    }
    buf.set_stringn(area.x, area.y, mark.symbol(), area.width as usize, style);
}

/// The number displayed next to `row` (0-based), `distance` visible lines
/// from the cursor's (a closed fold counts as one, so it's the count `j`/`k`
/// take there; 0 on the cursor's line).
pub fn label(row: usize, distance: usize, mode: LineNumbers) -> String {
    match mode {
        LineNumbers::None => String::new(),
        LineNumbers::Absolute => (row + 1).to_string(),
        LineNumbers::Relative if distance == 0 => (row + 1).to_string(),
        LineNumbers::Relative => distance.to_string(),
    }
}

/// Draws one gutter cell row at `area` (which must be one line high).
pub fn render(buf: &mut Buffer, area: Rect, label: &str, style: Style) {
    if area.width == 0 {
        return;
    }
    let digits = (area.width as usize).saturating_sub(PADDING);
    let text = format!(" {label:>digits$} ");
    buf.set_stringn(area.x, area.y, text, area.width as usize, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_has_minimum_and_grows() {
        assert_eq!(width(5, LineNumbers::Absolute), 5);
        assert_eq!(width(12345, LineNumbers::Relative), 7);
        assert_eq!(width(5, LineNumbers::None), 0);
    }

    #[test]
    fn fold_marks_draw_one_cell() {
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        render_fold(&mut buf, area, FoldMark::Closed, Style::default());
        assert_eq!(buf[(0, 0)].symbol(), "▸");
        render_fold(&mut buf, area, FoldMark::None, Style::default());
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(FoldMark::Open.continued(), FoldMark::Inside);
        assert_eq!(FoldMark::Closed.continued(), FoldMark::None);
    }

    #[test]
    fn absolute_labels() {
        assert_eq!(label(0, 5, LineNumbers::Absolute), "1");
        assert_eq!(label(9, 4, LineNumbers::Absolute), "10");
    }

    #[test]
    fn relative_labels_are_hybrid() {
        assert_eq!(label(5, 0, LineNumbers::Relative), "6");
        assert_eq!(label(2, 3, LineNumbers::Relative), "3");
        assert_eq!(label(8, 3, LineNumbers::Relative), "3");
    }

    #[test]
    fn render_right_aligns_with_padding() {
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, "7", Style::default());
        let text: String = (0..5).map(|x| buf[(x, 0)].symbol()).collect();
        assert_eq!(text, "   7 ");
    }
}
