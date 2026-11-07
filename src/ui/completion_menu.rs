//! The menu of matches shown while completing a word in insert mode.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    widgets::{Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use crate::config::Theme;
use crate::editor::WordCompletion;

/// The most matches shown at once; the list scrolls to keep the selected
/// one in view.
const MAX_ROWS: usize = 10;

pub struct CompletionMenu<'a> {
    completion: &'a WordCompletion,
    theme: &'a Theme,
}

impl<'a> CompletionMenu<'a> {
    pub fn new(completion: &'a WordCompletion, theme: &'a Theme) -> Self {
        Self { completion, theme }
    }

    /// Where the menu goes, for a cursor at `cursor` in `screen`: lined up
    /// with the start of the word being completed, below the cursor's row
    /// if it fits there, otherwise above.
    pub fn area(&self, cursor: Position, screen: Rect) -> Rect {
        let items = &self.completion.items;
        let rows = items.len().min(MAX_ROWS) as u16;
        let widest = items.iter().map(|w| w.width()).max().unwrap_or(0);
        let width = ((widest + 2) as u16).min(screen.width);
        let typed = self
            .completion
            .selected
            .map_or(self.completion.typed.width(), |i| items[i].width()) as u16;
        let x = cursor
            .x
            .saturating_sub(typed + 1)
            .max(screen.x)
            .min((screen.x + screen.width).saturating_sub(width));
        let below = cursor.y + 1;
        let y = if below + rows <= screen.y + screen.height {
            below
        } else {
            cursor.y.saturating_sub(rows).max(screen.y)
        };
        Rect::new(x, y, width, rows).intersection(screen)
    }
}

impl Widget for CompletionMenu<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        Clear.render(area, buf);
        let rows = area.height as usize;
        let selected = self.completion.selected;
        // Scroll so the selected match is in view.
        let first = selected.map_or(0, |i| (i + 1).saturating_sub(rows));
        let normal = self.theme.get("ui.popup");
        let chosen = normal.patch(self.theme.get("ui.popup.selected"));
        for (row, (i, word)) in self
            .completion
            .items
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let y = area.y + row as u16;
            let style = if Some(i) == selected { chosen } else { normal };
            buf.set_style(Rect::new(area.x, y, area.width, 1), style);
            buf.set_stringn(
                area.x + 1,
                y,
                word,
                (area.width as usize).saturating_sub(2),
                style,
            );
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
        }
    }

    #[test]
    fn below_the_cursor_lined_up_with_the_word() {
        let c = completion(&["foo", "foobar"], Some(0));
        let theme = Theme::default();
        let menu = CompletionMenu::new(&c, &theme);
        // Cursor after "foo" at x=13: the word starts at 10, the menu's
        // padding one cell before it.
        let area = menu.area(Position::new(13, 5), Rect::new(0, 0, 40, 20));
        assert_eq!(area, Rect::new(9, 6, 8, 2));
    }

    #[test]
    fn above_the_cursor_when_theres_no_room_below() {
        let c = completion(&["a1", "a2", "a3"], None);
        let theme = Theme::default();
        let menu = CompletionMenu::new(&c, &theme);
        let area = menu.area(Position::new(5, 19), Rect::new(0, 0, 40, 20));
        assert_eq!((area.y, area.height), (16, 3));
    }

    #[test]
    fn long_lists_scroll_to_the_selected_match() {
        let words: Vec<String> = (0..30).map(|i| format!("w{i:02}")).collect();
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let c = completion(&refs, Some(25));
        let area = Rect::new(0, 0, 6, 10);
        let mut buf = Buffer::empty(area);
        CompletionMenu::new(&c, &Theme::default()).render(area, &mut buf);
        let last: String = (0..6).map(|x| buf[(x, 9)].symbol().to_string()).collect();
        assert_eq!(last.trim(), "w25");
    }
}
