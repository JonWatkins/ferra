//! The line-number gutter on the left of the editor pane.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use crate::config::LineNumbers;

/// One cell of padding on each side of the number.
const PADDING: usize = 2;

/// Gutter width in cells for a buffer with `line_count` lines.
pub fn width(line_count: usize, mode: LineNumbers) -> u16 {
    crate::editor::windows::gutter_width(line_count, mode)
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
