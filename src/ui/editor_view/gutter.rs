//! The line-number gutter on the left of the editor pane.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use crate::config::LineNumbers;

const MIN_DIGITS: usize = 3;
/// One cell of padding on each side of the number.
const PADDING: usize = 2;

/// Gutter width in cells for a buffer with `line_count` lines.
pub fn width(line_count: usize, mode: LineNumbers) -> u16 {
    if mode == LineNumbers::None {
        return 0;
    }
    let digits = line_count.to_string().len().max(MIN_DIGITS);
    (digits + PADDING) as u16
}

/// The number displayed next to `row` (0-based).
pub fn label(row: usize, cursor_row: usize, mode: LineNumbers) -> String {
    match mode {
        LineNumbers::None => String::new(),
        LineNumbers::Absolute => (row + 1).to_string(),
        LineNumbers::Relative if row == cursor_row => (row + 1).to_string(),
        LineNumbers::Relative => row.abs_diff(cursor_row).to_string(),
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
    fn absolute_labels() {
        assert_eq!(label(0, 5, LineNumbers::Absolute), "1");
        assert_eq!(label(9, 5, LineNumbers::Absolute), "10");
    }

    #[test]
    fn relative_labels_are_hybrid() {
        assert_eq!(label(5, 5, LineNumbers::Relative), "6");
        assert_eq!(label(2, 5, LineNumbers::Relative), "3");
        assert_eq!(label(8, 5, LineNumbers::Relative), "3");
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
