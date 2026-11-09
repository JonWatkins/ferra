//! A plugin's popup (`editor/popup.rs`):
//!
//! ```text
//! ╭─ TODOs ──────────────────────────────╮
//! │ src/main.rs:12  TODO: tidy this      │
//! │ src/app.rs:40   TODO: and this       │   (the selected line highlighted)
//! ╰─ 1/2 ─────────────── ⏎ open  q close ╯
//! ```
//!
//! Sized to its lines (or as the plugin says), within the windows' area;
//! a long one scrolls, with where it's at in the bottom border.

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
use crate::editor::popup::{Popup, PopupLine, PopupPosition};

pub struct PopupView<'a> {
    popup: &'a Popup,
    theme: &'a Theme,
}

fn line_width(line: &PopupLine) -> usize {
    line.iter().map(|span| span.text.width()).sum()
}

impl<'a> PopupView<'a> {
    pub fn new(popup: &'a Popup, theme: &'a Theme) -> Self {
        Self { popup, theme }
    }

    /// Where it goes in `screen` (the windows' area): `cursor` is the
    /// terminal cursor in the active window, `window` that window's area.
    pub fn area(
        popup: &Popup,
        screen: Rect,
        cursor: Option<Position>,
        window: Option<Rect>,
    ) -> Rect {
        let content = popup
            .lines
            .iter()
            .map(line_width)
            .max()
            .unwrap_or(0)
            .max(popup.title.width() + 4)
            .max(popup.footer.width() + 8);
        // Borders and a column of padding each side.
        let max_width = screen.width.saturating_sub(4).max(screen.width.min(10));
        let width = popup
            .width
            .unwrap_or((content + 4).min(u16::MAX as usize) as u16)
            .clamp(screen.width.min(16), max_width);
        let max_height = screen.height.saturating_sub(2).max(screen.height.min(3));
        let height = popup
            .height
            .unwrap_or((popup.lines.len() + 2).min(u16::MAX as usize) as u16)
            .clamp(3.min(screen.height), max_height);

        let (x, y) = match (popup.position, cursor, window) {
            (PopupPosition::Cursor, Some(cursor), _) => {
                let below = screen.bottom().saturating_sub(cursor.y + 1);
                let above = cursor.y.saturating_sub(screen.y);
                let y = if height <= below || below >= above {
                    cursor.y + 1
                } else {
                    cursor.y.saturating_sub(height)
                };
                let x = cursor
                    .x
                    .saturating_sub(1)
                    .max(screen.x)
                    .min(screen.right().saturating_sub(width));
                (x, y.max(screen.y))
            }
            (PopupPosition::TopRight, _, Some(window)) => (
                window.right().saturating_sub(width + 1).max(screen.x),
                window.y,
            ),
            _ => (
                screen.x + (screen.width.saturating_sub(width)) / 2,
                screen.y + (screen.height.saturating_sub(height)) / 3,
            ),
        };
        Rect::new(x, y, width, height).intersection(screen)
    }
}

impl Widget for PopupView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 3 {
            return;
        }
        let theme = self.theme;
        let popup = self.popup;
        let base = theme.get("ui.popup");
        Clear.render(area, buf);
        buf.set_style(area, base);

        let inner = Block::bordered().inner(area);
        let first = popup.first_shown(inner.height as usize);
        let shown = inner.height as usize;
        let total = popup.lines.len();
        // Where it's at, when it doesn't all fit (or for a list).
        let place = match popup.selected() {
            Some(selected) if total > 1 => Some(format!(" {}/{total} ", selected + 1)),
            _ if total > shown => Some(format!(
                " {}-{} of {total} ",
                first + 1,
                (first + shown).min(total)
            )),
            _ => None,
        };

        let mut block = Block::bordered()
            .border_set(border::ROUNDED)
            .border_style(theme.get("ui.popup.border"));
        if !popup.title.is_empty() {
            let title = truncate_end(&popup.title, area.width.saturating_sub(6) as usize);
            block = block.title(Span::styled(
                format!(" {title} "),
                theme.get("ui.popup.title"),
            ));
        }
        let place_width = place.as_ref().map_or(0, |p| p.width() + 2);
        if let Some(place) = place {
            block = block.title_bottom(Span::styled(place, theme.get("ui.popup.detail")));
        }
        if !popup.footer.is_empty() && popup.footer.width() + place_width + 4 <= area.width as usize
        {
            block = block.title_bottom(
                Line::from(Span::styled(
                    format!(" {} ", popup.footer),
                    theme.get("ui.popup.detail"),
                ))
                .right_aligned(),
            );
        }
        block.render(area, buf);

        let text_x = inner.x + 1;
        let text_width = inner.width.saturating_sub(2) as usize;
        for (row, (index, line)) in popup
            .lines
            .iter()
            .enumerate()
            .skip(first)
            .take(shown)
            .enumerate()
        {
            let y = inner.y + row as u16;
            let line_style = if popup.selected() == Some(index) {
                let selected = base.patch(theme.get("ui.popup.selected"));
                buf.set_style(Rect::new(inner.x, y, inner.width, 1), selected);
                selected
            } else {
                base
            };
            let mut x = text_x;
            let mut room = text_width;
            for span in line {
                if room == 0 {
                    break;
                }
                let style = match &span.style {
                    Some(scope) => line_style.patch(theme.get(scope)),
                    None => line_style,
                };
                let text = truncate_end(&span.text, room);
                let (next_x, _) = buf.set_stringn(x, y, &text, room, style);
                room = room.saturating_sub((next_x - x) as usize);
                x = next_x;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::popup::PopupSpan;

    fn text(lines: &[&str]) -> Vec<PopupLine> {
        lines
            .iter()
            .map(|l| {
                vec![PopupSpan {
                    text: l.to_string(),
                    style: None,
                }]
            })
            .collect()
    }

    fn render(popup: &Popup, screen: Rect) -> (Vec<String>, Rect, Buffer) {
        let area = PopupView::area(popup, screen, None, None);
        let mut buf = Buffer::empty(screen);
        let theme = crate::config::Config::default().theme;
        PopupView::new(popup, &theme).render(area, &mut buf);
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
    fn fits_its_lines_centred_with_title_and_footer() {
        let popup = Popup {
            title: "Hello".into(),
            lines: text(&["first line", "second"]),
            footer: "q close".into(),
            ..Popup::default()
        };
        let (rows, area, _) = render(&popup, Rect::new(0, 0, 60, 20));
        // Wide enough for the footer too.
        assert_eq!((area.width, area.height), (19, 4));
        assert_eq!(area.x, 20, "centred");
        assert!(
            rows[0].starts_with("╭ Hello ─") && rows[0].ends_with("╮"),
            "{rows:#?}"
        );
        assert_eq!(rows[1], "│ first line      │");
        assert_eq!(rows[2], "│ second          │");
        assert!(rows[0].contains(" Hello "), "{rows:#?}");
        assert!(rows[1].contains("│ first line"), "{rows:#?}");
        assert!(rows[3].contains(" q close "), "{rows:#?}");
    }

    #[test]
    fn a_long_list_scrolls_to_its_selection_and_says_where_it_is() {
        let mut popup = Popup {
            title: "List".into(),
            lines: text(&["a", "b", "c", "d", "e", "f", "g", "h"]),
            select: true,
            height: Some(5),
            ..Popup::default()
        };
        popup.select(6);
        let (rows, area, buf) = render(&popup, Rect::new(0, 0, 40, 20));
        assert_eq!(area.height, 5);
        assert!(rows[1].contains("│ e"), "{rows:#?}");
        assert!(rows[3].contains("│ g"), "{rows:#?}");
        assert!(rows[4].contains(" 7/8 "), "{rows:#?}");
        let theme = crate::config::Config::default().theme;
        assert_eq!(
            buf[(area.x + 2, area.y + 3)].bg,
            theme.get("ui.popup.selected").bg.unwrap()
        );
    }

    #[test]
    fn styled_spans_and_placement_by_the_cursor() {
        let popup = Popup {
            lines: vec![vec![
                PopupSpan {
                    text: "key".into(),
                    style: Some("keyword".into()),
                },
                PopupSpan {
                    text: " plain".into(),
                    style: None,
                },
            ]],
            position: PopupPosition::Cursor,
            ..Popup::default()
        };
        let screen = Rect::new(0, 0, 60, 20);
        let area = PopupView::area(&popup, screen, Some(Position::new(10, 5)), None);
        assert_eq!((area.x, area.y), (9, 6), "below the cursor");
        let area = PopupView::area(&popup, screen, Some(Position::new(10, 19)), None);
        assert!(area.bottom() <= 19, "above it at the bottom: {area:?}");

        let mut buf = Buffer::empty(screen);
        let theme = crate::config::Config::default().theme;
        let area = PopupView::area(&popup, screen, None, None);
        PopupView::new(&popup, &theme).render(area, &mut buf);
        assert_eq!(
            buf[(area.x + 2, area.y + 1)].fg,
            theme.get("keyword").fg.unwrap()
        );
    }
}
