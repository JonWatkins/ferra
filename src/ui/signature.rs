//! The signature help popup: the signature of the call being typed, above
//! the cursor's line (the completion menu goes below), the parameter being
//! typed picked out, and its documentation under a rule.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    text::Line,
    widgets::{Block, Clear, Widget},
};
use unicode_width::UnicodeWidthChar;

use crate::config::Theme;
use crate::editor::signature::SignatureHelp;

/// The widest the popup's text gets, and the widest documentation gets
/// before it wraps (the signature itself can be wider), in how many rows.
const MAX_WIDTH: usize = 100;
const DOC_WIDTH: usize = 72;
const DOC_ROWS: usize = 4;

pub struct SignatureView<'a> {
    help: &'a SignatureHelp,
    theme: &'a Theme,
}

impl<'a> SignatureView<'a> {
    pub fn new(help: &'a SignatureHelp, theme: &'a Theme) -> Self {
        Self { help, theme }
    }

    fn width_of(text: &str) -> usize {
        text.chars().map(|c| c.width().unwrap_or(0)).sum()
    }

    /// How wide the text is in `screen`.
    fn text_width(&self, screen: Rect) -> usize {
        let docs = self.help.documentation.join(" ");
        Self::width_of(&self.help.label)
            .max(Self::width_of(&docs).min(DOC_WIDTH))
            .min(MAX_WIDTH)
            .min((screen.width as usize).saturating_sub(4))
            .max(1)
    }

    /// The documentation, as one paragraph wrapped to `width`, at most
    /// `DOC_ROWS` rows (the last ending in `…` if there's more).
    fn doc_rows(&self, width: usize) -> Vec<String> {
        let text: Vec<char> = self.help.documentation.join(" ").chars().collect();
        if text.is_empty() {
            return Vec::new();
        }
        let ranges = super::hover::wrap(&text, width);
        let more = ranges.len() > DOC_ROWS;
        let mut rows: Vec<String> = ranges
            .iter()
            .take(DOC_ROWS)
            .map(|&(s, e)| text[s..e].iter().collect::<String>().trim_end().to_string())
            .collect();
        if more && let Some(last) = rows.last_mut() {
            while Self::width_of(last) + 1 > width {
                last.pop();
            }
            last.push('…');
        }
        rows
    }

    /// Where it goes for a cursor at `cursor` in `screen`: above the
    /// cursor's line, starting at the cursor's column (moved left to fit),
    /// or below the line if there's no room above.
    pub fn area(&self, cursor: Position, screen: Rect) -> Rect {
        let text = self.text_width(screen);
        let width = (text as u16 + 4).min(screen.width);
        let docs = self.doc_rows(text).len() as u16;
        let height = 3 + if docs > 0 { docs + 1 } else { 0 };
        let above = cursor.y.saturating_sub(screen.y);
        let y = if height <= above {
            cursor.y - height
        } else {
            cursor.y + 1
        };
        let x = cursor
            .x
            .saturating_sub(2)
            .max(screen.x)
            .min((screen.x + screen.width).saturating_sub(width));
        Rect::new(x, y, width, height).intersection(screen)
    }
}

impl Widget for SignatureView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width < 5 {
            return;
        }
        Clear.render(area, buf);
        let base = self.theme.get("ui.popup");
        let border = base.patch(self.theme.get("ui.popup.border"));
        let mut block = Block::bordered().border_style(border).style(base);
        if self.help.total > 1 {
            let which = format!(" {}/{} ", self.help.index + 1, self.help.total);
            block = block.title_bottom(Line::from(which).right_aligned());
        }
        let inner = block.inner(area);
        block.render(area, buf);
        let room = (inner.width as usize).saturating_sub(2);

        // The label, scrolled to keep the active parameter in view if it's
        // too wide.
        let chars: Vec<char> = self.help.label.chars().collect();
        let first = match self.help.active {
            Some((_, end)) if Self::width_of(&self.help.label) > room => {
                let mut start = 0;
                while start < end
                    && chars[start..end]
                        .iter()
                        .map(|c| c.width().unwrap_or(0))
                        .sum::<usize>()
                        > room
                {
                    start += 1;
                }
                start
            }
            _ => 0,
        };
        let active = base.patch(self.theme.get("ui.popup.match"));
        let mut x = inner.x + 1;
        let right = inner.x + 1 + room as u16;
        for (i, &c) in chars.iter().enumerate().skip(first) {
            let w = c.width().unwrap_or(0) as u16;
            if x + w > right {
                break;
            }
            let in_active = self.help.active.is_some_and(|(s, e)| s <= i && i < e);
            let style = if in_active { active } else { base };
            buf.set_string(x, inner.y, c.to_string(), style);
            x += w;
        }

        let docs = self.doc_rows(room);
        if docs.is_empty() || inner.height < 3 {
            return;
        }
        buf.set_string(inner.x + 1, inner.y + 1, "─".repeat(room), border);
        let dim = base.patch(self.theme.get("ui.popup.detail"));
        for (row, line) in docs.iter().enumerate() {
            let y = inner.y + 2 + row as u16;
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_stringn(inner.x + 1, y, line, room, dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(help: &SignatureHelp, width: u16) -> Vec<String> {
        let theme = crate::config::Config::default().theme;
        let view = SignatureView::new(help, &theme);
        let area = view.area(Position::new(4, 10), Rect::new(0, 0, width, 12));
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 12));
        SignatureView::new(help, &theme).render(area, &mut buf);
        (area.y..area.y + area.height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn above_the_line_with_the_parameter_and_its_docs() {
        let help = SignatureHelp {
            label: "fn area(scale: f64) -> f64".into(),
            active: Some((8, 18)),
            index: 0,
            total: 2,
            documentation: vec!["How much bigger.".into()],
        };
        assert_eq!(
            rows(&help, 40),
            [
                "  ┌────────────────────────────┐",
                "  │ fn area(scale: f64) -> f64 │",
                "  │ ────────────────────────── │",
                "  │ How much bigger.           │",
                "  └─────────────────────── 1/2 ┘",
            ]
        );
    }

    #[test]
    fn long_documentation_wraps_to_a_few_rows() {
        let help = SignatureHelp {
            label: "Printf(format string, a ...any)".into(),
            active: Some((7, 20)),
            index: 0,
            total: 1,
            documentation: vec![
                "Printf formats according to a format specifier and writes to standard output."
                    .into(),
                "It returns the number of bytes written and any write error encountered.".into(),
            ],
        };
        assert_eq!(
            rows(&help, 50)[3..7],
            [
                "│ Printf formats according to a format           │",
                "│ specifier and writes to standard output. It    │",
                "│ returns the number of bytes written and any    │",
                "│ write error encountered.                       │",
            ]
        );
        // More than four rows' worth: the fourth ends in `…`.
        let mut longer = help.clone();
        longer
            .documentation
            .push("See the package documentation for the verbs.".into());
        let shown = rows(&longer, 50);
        assert_eq!(shown.len(), 8, "{shown:#?}");
        assert!(
            shown[6].trim_end_matches(['│', ' ']).ends_with('…'),
            "{shown:#?}"
        );
    }

    #[test]
    fn a_long_signature_keeps_the_parameter_in_view() {
        let help = SignatureHelp {
            label: "fn f(first: u8, second: u8, third: u8)".into(),
            active: Some((29, 38)),
            index: 0,
            total: 1,
            documentation: Vec::new(),
        };
        let shown = rows(&help, 24);
        assert!(shown[1].contains("third: u8"), "{shown:?}");
    }
}
