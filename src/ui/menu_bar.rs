//! The menu bar's row along the top of the screen: each menu's title, the
//! letter that opens it (with Alt, or ⌥) underlined, the open one
//! highlighted. The menus themselves are popups (`editor/menu_bar.rs`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::config::Theme;
use crate::editor::menu_bar::{MENUS, title_spans};

pub struct MenuBar<'a> {
    open: Option<usize>,
    theme: &'a Theme,
}

impl<'a> MenuBar<'a> {
    pub fn new(open: Option<usize>, theme: &'a Theme) -> Self {
        Self { open, theme }
    }
}

impl Widget for MenuBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = self.theme;
        buf.set_style(area, theme.get("ui.menubar"));
        for (i, (menu, (from, to))) in MENUS.iter().zip(title_spans(area.x)).enumerate() {
            if to > area.right() {
                break;
            }
            let style = if self.open == Some(i) {
                theme.get("ui.menubar.open")
            } else {
                theme.get("ui.menubar")
            };
            buf.set_string(from, area.y, format!(" {} ", menu.title), style);
            // The letter that opens it.
            if let Some(at) = menu
                .title
                .chars()
                .position(|c| c.to_ascii_lowercase() == menu.letter)
            {
                let cell = &mut buf[(from + 1 + at as u16, area.y)];
                cell.set_style(style.patch(theme.get("ui.menubar.letter")));
            }
        }
    }
}
