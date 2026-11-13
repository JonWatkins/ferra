//! The activity bar: a narrow column of icons down the left of the screen,
//! always there, as VS Code's. Each of the sidebar's views has an icon
//! (files, Source Control, plugins' views), the shown one marked; the
//! settings sit at the bottom. Clicking one runs `activity <name>`.
//!
//! ```text
//!  ▎    │ src/
//!  ▎ ☰  │ ▸ docs
//!  ▎    │ ▸ src
//!       │
//!    ⎇  │
//!       │
//!       ...
//!    ⚙  │
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::config::Theme;
use crate::editor::sidebar_views::ActivityItem;

/// Columns the bar takes: the icon centred, with room either side.
pub const WIDTH: u16 = 5;

/// Rows each icon's block takes: the icon in the middle.
pub const BLOCK: u16 = 3;

/// Where each item's block goes in `area`: its top row, one under
/// another from the top, and the settings' at the bottom. Items that
/// wouldn't fit above the settings are left out.
pub fn layout(area: Rect, items: usize) -> (Vec<u16>, Option<u16>) {
    if area.height < BLOCK {
        return (Vec::new(), None);
    }
    let settings = area.bottom() - BLOCK;
    let rows = (0..items)
        .map(|i| area.y + BLOCK * i as u16)
        .take_while(|&y| y + BLOCK <= settings)
        .collect();
    (rows, Some(settings))
}

pub struct ActivityBar<'a> {
    items: &'a [ActivityItem],
    settings: &'a ActivityItem,
    theme: &'a Theme,
}

impl<'a> ActivityBar<'a> {
    pub fn new(items: &'a [ActivityItem], settings: &'a ActivityItem, theme: &'a Theme) -> Self {
        Self {
            items,
            settings,
            theme,
        }
    }
}

impl Widget for ActivityBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = self.theme;
        buf.set_style(area, theme.get("ui.activitybar"));
        let (rows, settings) = layout(area, self.items.len());
        let draw = |buf: &mut Buffer, item: &ActivityItem, top: u16| {
            let block = Rect::new(area.x, top, area.width, BLOCK.min(area.bottom() - top));
            let style = if item.active {
                // The shown view's block stands out, with a bar beside it.
                buf.set_style(block, theme.get("ui.activitybar.active"));
                for y in block.top()..block.bottom() {
                    buf[(area.x, y)]
                        .set_symbol("▎")
                        .set_style(theme.get("ui.activitybar.marker"));
                }
                theme.get("ui.activitybar.active")
            } else {
                theme.get("ui.activitybar")
            };
            let icon: String = item.icon.chars().take(2).collect();
            let x = area.x + area.width / 2;
            buf.set_stringn(
                x,
                top + BLOCK / 2,
                &icon,
                (area.right() - x) as usize,
                style,
            );
        };
        for (item, y) in self.items.iter().zip(rows) {
            draw(buf, item, y);
        }
        if let Some(y) = settings {
            draw(buf, self.settings, y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_blocks_from_the_top_with_the_settings_at_the_bottom() {
        let (rows, settings) = layout(Rect::new(0, 0, 5, 20), 3);
        assert_eq!(rows, [0, 3, 6]);
        assert_eq!(settings, Some(17));
        // Too short for them all: the ones that fit, settings kept.
        let (rows, settings) = layout(Rect::new(0, 0, 5, 10), 5);
        assert_eq!(rows, [0, 3]);
        assert_eq!(settings, Some(7));
    }
}
