//! The picker popup: a centred, bordered box with a filter prompt and the
//! matching items, matched letters highlighted.
//!
//! ```text
//! ╭─ Buffers ──────────────────────╮
//! │ > ma                           │
//! │──────────────────────────────── │
//! │ src/main.rs              #1 [+] │
//! │ README.md                    #2 │
//! ╰──────────────────────── 2/5 ───╯
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Style,
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::text::{truncate_end, truncate_start};
use crate::config::Theme;
use crate::editor::picker::Picker;

pub struct PickerView<'a> {
    picker: &'a Picker,
    theme: &'a Theme,
}

impl<'a> PickerView<'a> {
    pub fn new(picker: &'a Picker, theme: &'a Theme) -> Self {
        Self { picker, theme }
    }

    /// The popup's area: centred, about 60% of the screen, within limits.
    pub fn area(screen: Rect) -> Rect {
        let width = (screen.width * 6 / 10).clamp(screen.width.min(40), screen.width.min(100));
        let height = (screen.height * 6 / 10).clamp(screen.height.min(8), screen.height);
        Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        )
    }

    /// Where the terminal cursor goes: after the typed filter.
    pub fn cursor_position(picker: &Picker, area: Rect) -> Position {
        let inner = Block::bordered().inner(area);
        let x = inner.x as usize + 2 + picker.query().width();
        let x = x.min(inner.right().saturating_sub(1) as usize);
        Position::new(x as u16, inner.y)
    }
}

impl Widget for PickerView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let base = self.theme.get("ui.popup");
        Clear.render(area, buf);
        buf.set_style(area, base);

        let picker = self.picker;
        let count = format!(" {}/{} ", picker.matches().len(), picker.total());
        let block = Block::bordered()
            .border_set(border::ROUNDED)
            .border_style(self.theme.get("ui.popup.border"))
            .title(Span::styled(
                // Still filling in from the background (the file listing).
                if picker.loading().is_some() {
                    format!(" {} (listing…) ", picker.title())
                } else {
                    format!(" {} ", picker.title())
                },
                self.theme.get("ui.popup.title"),
            ))
            .title_bottom(Line::from(count).right_aligned());
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Filter prompt, then a separator, then the list.
        let prompt = Line::from(vec![
            Span::styled("> ", self.theme.get("ui.popup.prompt")),
            Span::raw(picker.query()),
        ]);
        prompt.render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
        if inner.height < 3 {
            return;
        }
        let separator = "─".repeat(inner.width as usize);
        buf.set_string(
            inner.x,
            inner.y + 1,
            separator,
            self.theme.get("ui.popup.border"),
        );

        let list = Rect::new(inner.x, inner.y + 2, inner.width, inner.height - 2);
        let visible = list.height as usize;
        // Scroll just enough to keep the selection in view.
        let first = picker.selected().saturating_sub(visible.saturating_sub(1));
        for (row, (i, found)) in picker
            .matches()
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .enumerate()
        {
            let y = list.y + row as u16;
            let line_area = Rect::new(list.x, y, list.width, 1);
            if i == picker.selected() {
                buf.set_style(line_area, self.theme.get("ui.popup.selected"));
            }
            let item = picker.item(found);
            render_item(
                buf,
                line_area,
                &item.label,
                &found.positions,
                &item.detail,
                picker.keeps_label_start(),
                self.theme,
            );
        }
    }
}

/// Draws one row: the label (matched letters highlighted, shortened from
/// the left if needed, or the right with `keep_start`) and the detail
/// right-aligned.
fn render_item(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    positions: &[usize],
    detail: &str,
    keep_start: bool,
    theme: &Theme,
) {
    // One cell of padding each side, and two between label and detail.
    let room = (area.width as usize).saturating_sub(2);
    let gap = if detail.is_empty() { 0 } else { 2 };
    let (label_room, detail_room) = if label.width() + gap + detail.width() <= room {
        (label.width(), detail.width())
    } else {
        // The label keeps what it needs, as long as the detail still gets
        // a third of the row.
        let detail_room = room
            .saturating_sub(label.width() + gap)
            .max(detail.width().min(room / 3));
        (room.saturating_sub(detail_room + gap), detail_room)
    };
    // Labels (often paths) are cut from the start, descriptions at the end.
    let shown = if keep_start {
        truncate_end(label, label_room)
    } else {
        truncate_start(label, label_room)
    };
    let detail = truncate_end(detail, detail_room);
    let shortened = shown != label;

    // If the label lost its start, its first char is an ellipsis standing
    // in for `cut` chars; shift the match positions to suit.
    let cut = if keep_start || !shortened {
        0
    } else {
        label.chars().count() + 1 - shown.chars().count()
    };
    let ellipsis_at = match (shortened, keep_start) {
        (false, _) => None,
        (true, true) => Some(shown.chars().count() - 1),
        (true, false) => Some(0),
    };
    let match_style = theme.get("ui.popup.match");
    let spans: Vec<Span> = shown
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let original = i + cut;
            let matched = ellipsis_at != Some(i) && positions.contains(&original);
            let style = if matched {
                match_style
            } else {
                Style::default()
            };
            Span::styled(c.to_string(), style)
        })
        .collect();
    Line::from(spans).render(
        Rect::new(area.x + 1, area.y, area.width.saturating_sub(1), 1),
        buf,
    );

    if !detail.is_empty() {
        let x = area.right().saturating_sub(1 + detail.width() as u16);
        buf.set_string(x, area.y, &detail, theme.get("ui.popup.detail"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::picker::{PickerAction, PickerItem};

    fn item(label: &str, detail: &str) -> PickerItem {
        PickerItem {
            label: label.into(),
            detail: detail.into(),
            action: PickerAction::RunCommand(label.into()),
        }
    }

    fn render(picker: &Picker, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        PickerView::new(picker, &Theme::default()).render(area, &mut buf);
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn draws_title_prompt_items_and_count() {
        let mut picker = Picker::new(
            "Commands",
            vec![item("write", "Save"), item("quit", "Leave")],
        );
        picker.push('w');
        let rows = render(&picker, 30, 7);
        assert!(rows[0].contains("Commands"), "{:?}", rows[0]);
        assert!(rows[1].contains("> w"), "{:?}", rows[1]);
        assert!(
            rows[3].contains("write") && rows[3].trim_end().ends_with("Save │"),
            "{:?}",
            rows[3]
        );
        assert!(rows[6].contains("1/2"), "{:?}", rows[6]);
    }

    #[test]
    fn scrolls_to_keep_the_selection_visible() {
        let items = (0..20).map(|i| item(&format!("item{i:02}"), "")).collect();
        let mut picker = Picker::new("Many", items);
        picker.move_selection(15);
        let rows = render(&picker, 30, 8);
        // 8 rows: border, prompt, separator, 4 list rows, border.
        assert!(rows[6].contains("item15"), "{rows:#?}");
    }

    #[test]
    fn long_labels_are_shortened_from_the_left() {
        let picker = Picker::new(
            "Buffers",
            vec![item("/a/very/long/path/to/some/file.rs", "#1")],
        );
        let rows = render(&picker, 24, 6);
        assert!(rows[3].contains("…"), "{:?}", rows[3]);
        assert!(rows[3].contains("file.rs"), "{:?}", rows[3]);
        assert!(rows[3].contains("#1"), "{:?}", rows[3]);
    }

    #[test]
    fn some_pickers_keep_the_start_of_long_labels() {
        let picker = Picker::new(
            "Registers",
            vec![item("\"a  some long text that will not fit", "1 line")],
        )
        .keep_label_start();
        let rows = render(&picker, 24, 6);
        assert!(rows[3].contains("\"a  some"), "{:?}", rows[3]);
        assert!(rows[3].contains("…"), "{:?}", rows[3]);
    }

    #[test]
    fn matched_letters_are_highlighted() {
        let mut theme = Theme::default();
        theme.set(
            "ui.popup.match",
            Style::new().fg(ratatui::style::Color::Red),
        );
        let mut picker = Picker::new("C", vec![item("buffer_next", "")]);
        picker.push('b');
        picker.push('n');

        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        PickerView::new(&picker, &theme).render(area, &mut buf);
        let red: Vec<u16> = (0..20)
            .filter(|&x| buf[(x, 3)].fg == ratatui::style::Color::Red)
            .collect();
        // Border at x=0, padding at x=1: "b" at 2, "n" at 2 + 7.
        assert_eq!(red, vec![2, 9]);
    }

    #[test]
    fn long_details_keep_their_start() {
        let picker = Picker::new(
            "Commands",
            vec![item(
                "buffer_next",
                "Switch to the next buffer ([count] times)",
            )],
        );
        let rows = render(&picker, 40, 6);
        assert!(
            rows[3].contains("buffer_next  Switch to the"),
            "{:?}",
            rows[3]
        );
        assert!(rows[3].contains("…"), "{:?}", rows[3]);
    }

    #[test]
    fn area_is_centred_and_bounded() {
        let area = PickerView::area(Rect::new(0, 0, 200, 50));
        assert_eq!((area.width, area.height), (100, 30));
        assert_eq!((area.x, area.y), (50, 10));
        let small = PickerView::area(Rect::new(0, 0, 30, 6));
        assert_eq!((small.width, small.height), (30, 6));
    }

    #[test]
    fn cursor_follows_the_query() {
        let mut picker = Picker::new("C", vec![]);
        picker.push('a');
        picker.push('b');
        let pos = PickerView::cursor_position(&picker, Rect::new(10, 5, 30, 10));
        assert_eq!(pos, Position::new(10 + 1 + 2 + 2, 6));
    }
}
