//! A plugin's input box (`editor/input_box.rs`), at the top centre of the
//! windows, as in VS Code:
//!
//! ```text
//! ╭ New name ──────────────────────────────────╮
//! │  old_name█                                 │
//! ╰────────────────────────── ⏎ ok  Esc cancel ╯
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::text::truncate_end;
use crate::config::Theme;
use crate::editor::input_box::InputBox;

const WIDTH: u16 = 60;

/// The keys the box shows, as bound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputKeys {
    pub submit: Option<String>,
    pub cancel: Option<String>,
}

pub struct InputBoxView<'a> {
    input: &'a InputBox,
    keys: &'a InputKeys,
    theme: &'a Theme,
}

/// The field's area inside the box: a column in from each side of the
/// border, so the field has room around it.
pub fn field_area(area: Rect) -> Rect {
    let inner = Block::bordered().inner(area);
    Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        1.min(inner.height),
    )
}

/// The first char shown, so the cursor stays in a field `width` wide (the
/// text starts a column into it).
fn scroll(input: &InputBox, width: u16) -> usize {
    let room = width.saturating_sub(3) as usize;
    input.field.cursor().saturating_sub(room)
}

impl<'a> InputBoxView<'a> {
    pub fn new(input: &'a InputBox, keys: &'a InputKeys, theme: &'a Theme) -> Self {
        Self { input, keys, theme }
    }

    /// Where it goes: the top centre of `screen` (the windows' area).
    pub fn area(screen: Rect) -> Rect {
        let width = WIDTH
            .min(screen.width.saturating_sub(4))
            .max(screen.width.min(20));
        let x = screen.x + screen.width.saturating_sub(width) / 2;
        let y = screen.y + 1.min(screen.height.saturating_sub(3));
        Rect::new(x, y, width, 3).intersection(screen)
    }

    /// The field's column under `x` (for a click in it).
    pub fn field_col(input: &InputBox, area: Rect, x: u16) -> usize {
        let field = field_area(area);
        let col = scroll(input, field.width) + x.saturating_sub(field.x + 1) as usize;
        col.min(input.field.text().chars().count())
    }

    pub fn cursor_position(input: &InputBox, area: Rect) -> Position {
        let field = field_area(area);
        let col = input.field.cursor() - scroll(input, field.width);
        Position::new(
            (field.x + 1 + col as u16).min(field.right().saturating_sub(1)),
            field.y,
        )
    }
}

impl Widget for InputBoxView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 6 || area.height < 3 {
            return;
        }
        let theme = self.theme;
        Clear.render(area, buf);
        buf.set_style(area, theme.get("ui.popup"));

        let mut block = Block::bordered()
            .border_set(border::ROUNDED)
            .border_style(theme.get("ui.popup.border"));
        if !self.input.title.is_empty() {
            let title = truncate_end(&self.input.title, area.width.saturating_sub(6) as usize);
            block = block.title(Span::styled(
                format!(" {title} "),
                theme.get("ui.popup.title"),
            ));
        }
        let hint: Vec<String> = [
            self.keys.submit.as_ref().map(|k| format!("{k} ok")),
            self.keys.cancel.as_ref().map(|k| format!("{k} cancel")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let hint = format!(" {} ", hint.join("  "));
        if hint.width() + 4 <= area.width as usize {
            block = block.title_bottom(
                Line::from(Span::styled(hint, theme.get("ui.popup.detail"))).right_aligned(),
            );
        }
        block.render(area, buf);

        let field = field_area(area);
        let style = theme.get("ui.find.field.focused");
        buf.set_style(field, style);
        let width = field.width.saturating_sub(2) as usize;
        let text = self.input.field.text();
        if text.is_empty() {
            let placeholder = truncate_end(&self.input.placeholder, width);
            buf.set_string(
                field.x + 1,
                field.y,
                placeholder,
                style.patch(theme.get("ui.input.placeholder")),
            );
            return;
        }
        let shown: String = text
            .chars()
            .skip(scroll(self.input, field.width))
            .take(width)
            .collect();
        let text_style = if self.input.field.is_selected() {
            style.patch(theme.get("ui.find.field.selected"))
        } else {
            style
        };
        buf.set_stringn(field.x + 1, field.y, shown, width, text_style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(input: &InputBox, width: u16) -> (Vec<String>, Rect, Buffer) {
        let screen = Rect::new(0, 0, width, 12);
        let area = InputBoxView::area(screen);
        let mut buf = Buffer::empty(screen);
        let theme = crate::config::Config::default().theme;
        let keys = InputKeys {
            submit: Some("⏎".into()),
            cancel: Some("Esc".into()),
        };
        InputBoxView::new(input, &keys, &theme).render(area, &mut buf);
        let rows = (area.y..area.bottom())
            .map(|y| {
                (area.x..area.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect()
            })
            .collect();
        (rows, area, buf)
    }

    #[test]
    fn draws_the_title_field_and_keys_at_the_top_centre() {
        let input = InputBox::new("New name", "old_name", "rename");
        let (rows, area, buf) = render(&input, 80);
        assert_eq!((area.x, area.y, area.width, area.height), (10, 1, 60, 3));
        assert!(rows[0].starts_with("╭ New name ─"), "{rows:#?}");
        assert!(rows[1].starts_with("│  old_name "), "{rows:#?}");
        assert!(rows[2].ends_with(" ⏎ ok  Esc cancel ╯"), "{rows:#?}");
        let theme = crate::config::Config::default().theme;
        assert_eq!(
            buf[(area.x + 3, 2)].bg,
            theme.get("ui.find.field.selected").bg.unwrap(),
            "the starting text is selected"
        );
        assert_eq!(
            InputBoxView::cursor_position(&input, area),
            Position::new(area.x + 3 + 8, 2)
        );
    }

    #[test]
    fn shows_the_placeholder_while_empty_and_scrolls_long_text() {
        let mut input = InputBox::new("Name", "", "rename");
        input.placeholder = "type a name".into();
        let (rows, ..) = render(&input, 80);
        assert!(rows[1].contains("type a name"), "{rows:#?}");

        let long = "x".repeat(70) + "END";
        let input = InputBox::new("Name", &long, "rename");
        let (rows, area, _) = render(&input, 80);
        assert!(rows[1].contains("END"), "{rows:#?}");
        let cursor = InputBoxView::cursor_position(&input, area);
        assert!(cursor.x < area.right() - 1);
    }
}
