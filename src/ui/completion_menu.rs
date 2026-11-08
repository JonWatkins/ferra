//! The menu of matches shown while completing a word in insert mode.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    text::Line,
    widgets::{Block, Clear, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Theme;
use crate::editor::WordCompletion;
use crate::editor::picker::fuzzy_match;

/// The most matches shown at once; the list scrolls to keep the selected
/// one in view.
const MAX_ROWS: usize = 10;

/// The widest a label and a detail (a server's type or signature) are shown.
const MAX_LABEL: usize = 40;
const MAX_DETAIL: usize = 30;

pub struct CompletionMenu<'a> {
    completion: &'a WordCompletion,
    theme: &'a Theme,
}

impl<'a> CompletionMenu<'a> {
    pub fn new(completion: &'a WordCompletion, theme: &'a Theme) -> Self {
        Self { completion, theme }
    }

    /// Where the menu goes, for a cursor at `cursor` in `screen`: in a
    /// border, its text lined up with the start of the word being
    /// completed, below the cursor's row if it fits there, otherwise above.
    pub fn area(&self, cursor: Position, screen: Rect) -> Rect {
        let items = &self.completion.items;
        let height = items.len().min(MAX_ROWS) as u16 + 2;
        let (labels, details) = self.column_widths();
        let gap = if details > 0 { 2 } else { 0 };
        let width = ((labels + gap + details + 4) as u16).min(screen.width);
        let typed = self
            .completion
            .selected
            .map_or(self.completion.typed.width(), |i| items[i].width()) as u16;
        let x = cursor
            .x
            .saturating_sub(typed + 2)
            .max(screen.x)
            .min((screen.x + screen.width).saturating_sub(width));
        let below = cursor.y + 1;
        let y = if below + height <= screen.y + screen.height {
            below
        } else {
            cursor.y.saturating_sub(height).max(screen.y)
        };
        Rect::new(x, y, width, height).intersection(screen)
    }

    /// How wide the labels and the details beside them are.
    fn column_widths(&self) -> (usize, usize) {
        let c = self.completion;
        let count = c.items.len();
        let labels = (0..count)
            .map(|i| c.label(i).width().min(MAX_LABEL))
            .max()
            .unwrap_or(0);
        let details = (0..count)
            .map(|i| c.detail(i).width().min(MAX_DETAIL))
            .max()
            .unwrap_or(0);
        (labels, details)
    }
}

impl Widget for CompletionMenu<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width < 5 {
            return;
        }
        Clear.render(area, buf);
        let normal = self.theme.get("ui.popup");
        let count = self.completion.items.len();
        let rows = (area.height - 2) as usize;
        let selected = self.completion.selected;
        // Scroll so the selected match is in view.
        let first = selected.map_or(0, |i| (i + 1).saturating_sub(rows));
        let mut block = Block::bordered()
            .border_style(normal.patch(self.theme.get("ui.popup.border")))
            .style(normal);
        if count > rows {
            let shown = match selected {
                Some(i) => format!(" {}/{count} ", i + 1),
                None => format!(" {count} "),
            };
            block = block.title_bottom(Line::from(shown).right_aligned());
        }
        let inner = block.inner(area);
        block.render(area, buf);

        let chosen = normal.patch(self.theme.get("ui.popup.selected"));
        let matched = self.theme.get("ui.popup.match");
        let dim = self.theme.get("ui.popup.detail");
        let (labels, _) = self.column_widths();
        let right = inner.x + inner.width;
        let typed = &self.completion.typed;
        for (row, i) in (0..count).skip(first).take(rows).enumerate() {
            let y = inner.y + row as u16;
            let style = if Some(i) == selected { chosen } else { normal };
            buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
            // The label, with the letters typed picked out.
            let label = self.completion.label(i);
            let hits = fuzzy_match(typed, label)
                .map(|m| m.positions)
                .unwrap_or_default();
            let mut x = inner.x + 1;
            for (col, c) in label.chars().enumerate() {
                let w = c.width().unwrap_or(0) as u16;
                if x + w > right.saturating_sub(1) || col >= MAX_LABEL {
                    break;
                }
                let style = if hits.contains(&col) {
                    style.patch(matched)
                } else {
                    style
                };
                buf.set_string(x, y, c.to_string(), style);
                x += w;
            }
            let detail = self.completion.detail(i);
            let column = inner.x + 1 + labels as u16 + 2;
            if !detail.is_empty() && column >= x && column < right {
                let room = (right - column).saturating_sub(1) as usize;
                buf.set_stringn(column, y, detail, room, style.patch(dim));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position as TextPosition;

    fn completion(items: &[&str], selected: Option<usize>) -> WordCompletion {
        WordCompletion {
            start: TextPosition::new(0, 0),
            typed: "fo".into(),
            items: items.iter().map(|s| s.to_string()).collect(),
            selected,
            labels: Vec::new(),
            details: Vec::new(),
            server: None,
        }
    }

    #[test]
    fn below_the_cursor_lined_up_with_the_word() {
        let c = completion(&["foo", "foobar"], Some(0));
        let theme = Theme::default();
        let menu = CompletionMenu::new(&c, &theme);
        // Cursor after "foo" at x=13: the word starts at 10, the menu's
        // border and padding two cells before it; 6 wide inside.
        let area = menu.area(Position::new(13, 5), Rect::new(0, 0, 40, 20));
        assert_eq!(area, Rect::new(8, 6, 10, 4));
    }

    #[test]
    fn above_the_cursor_when_theres_no_room_below() {
        let c = completion(&["a1", "a2", "a3"], None);
        let theme = Theme::default();
        let menu = CompletionMenu::new(&c, &theme);
        let area = menu.area(Position::new(5, 19), Rect::new(0, 0, 40, 20));
        assert_eq!((area.y, area.height), (14, 5), "3 rows and the border");
    }

    #[test]
    fn long_lists_scroll_to_the_selected_match() {
        let words: Vec<String> = (0..30).map(|i| format!("w{i:02}")).collect();
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let c = completion(&refs, Some(25));
        let area = Rect::new(0, 0, 9, 12);
        let mut buf = Buffer::empty(area);
        CompletionMenu::new(&c, &Theme::default()).render(area, &mut buf);
        let row = |y: u16| -> String { (0..9).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        assert_eq!(row(10), "│ w25   │", "the last row inside the border");
        assert_eq!(row(11), "└ 26/30 ┘", "where it is in the list");
    }
}
