//! The tab bar (`tab-bar`): a tab for each open buffer along the top of
//! the windows, as VS Code's. The active window's buffer is highlighted;
//! `●` marks unsaved changes, `×` closes. With more tabs than fit, it
//! scrolls so the active one shows.
//!
//! ```text
//!  ▎ main.rs × │  lib.rs ● │  Cargo.toml × │
//! ```

use std::collections::HashMap;
use std::path::Path;

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
use unicode_width::UnicodeWidthStr;

use crate::config::{Icons, Theme};
use crate::editor::{DocumentId, Editor};

/// Where each tab went, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabBarMap {
    pub row: Rect,
    /// Each tab's columns, its `×`'s column, and its buffer.
    pub tabs: Vec<(u16, u16, u16, DocumentId)>,
}

/// One tab: its buffer, label and width.
struct Tab {
    id: DocumentId,
    icon: Option<(String, ratatui::style::Color)>,
    label: String,
    modified: bool,
    active: bool,
}

impl Tab {
    /// `▎` (or a space), the icon and a space, the label, ` ●`/` ×`, a
    /// space and the `│` after it.
    fn width(&self) -> u16 {
        let icon = self.icon.as_ref().map_or(0, |(g, _)| g.width() + 1);
        (1 + 1 + icon + self.label.width() + 2 + 1 + 1) as u16
    }
}

fn tabs(editor: &Editor) -> Vec<Tab> {
    let buffers = editor.buffers();
    let base = |name: &str| {
        Path::new(name)
            .file_name()
            .map_or_else(|| name.to_string(), |n| n.to_string_lossy().into_owned())
    };
    // Two buffers with the same file name show their folders too.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for buffer in &buffers {
        *counts.entry(base(&buffer.name)).or_default() += 1;
    }
    let shown_active = editor.settings().is_none() && editor.keybindings().is_none();
    buffers
        .iter()
        .map(|buffer| {
            let name = base(&buffer.name);
            let label = if counts[&name] > 1 {
                let parent = Path::new(&buffer.name)
                    .parent()
                    .and_then(Path::file_name)
                    .map(|p| p.to_string_lossy().into_owned());
                match parent {
                    Some(parent) => format!("{parent}/{name}"),
                    None => name,
                }
            } else {
                name
            };
            let icon = (editor.options.icons == Icons::Nerd)
                .then(|| super::icons::for_file(Path::new(&buffer.name), &editor.languages));
            Tab {
                id: buffer.id,
                icon,
                label,
                modified: buffer.modified,
                active: buffer.active && shown_active,
            }
        })
        .collect()
}

/// The first tab to draw in `width` columns, so the active one fits.
fn first_shown(tabs: &[Tab], width: u16) -> usize {
    let Some(active) = tabs.iter().position(|t| t.active) else {
        return 0;
    };
    let mut first = 0;
    while first < active && tabs[first..=active].iter().map(Tab::width).sum::<u16>() > width {
        first += 1;
    }
    first
}

pub struct TabBar<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
}

impl<'a> TabBar<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme) -> Self {
        Self { editor, theme }
    }

    /// Where the tabs are in `row`.
    pub fn map(editor: &Editor, row: Rect) -> TabBarMap {
        let tabs = tabs(editor);
        let mut x = row.x;
        let mut out = Vec::new();
        for tab in &tabs[first_shown(&tabs, row.width)..] {
            let width = tab.width();
            if x + width > row.right() {
                break;
            }
            // The `×` (or `●`) is three columns before the end.
            out.push((x, x + width, x + width - 3, tab.id));
            x += width;
        }
        TabBarMap { row, tabs: out }
    }
}

impl Widget for TabBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        buf.set_style(area, theme.get("ui.tabbar"));
        let tabs = tabs(self.editor);
        let mut x = area.x;
        for tab in &tabs[first_shown(&tabs, area.width)..] {
            let width = tab.width();
            if x + width > area.right() {
                // There are more: say so at the end.
                if x < area.right() {
                    buf.set_string(area.right() - 1, area.y, "…", theme.get("ui.tabbar.tab"));
                }
                break;
            }
            let style = if tab.active {
                theme.get("ui.tabbar.active")
            } else {
                theme.get("ui.tabbar.tab")
            };
            buf.set_style(Rect::new(x, area.y, width - 1, 1), style);
            if tab.active {
                buf.set_string(x, area.y, "▎", style.patch(theme.get("ui.tabbar.marker")));
            }
            let mut at = x + 2;
            if let Some((glyph, color)) = &tab.icon {
                buf.set_string(at, area.y, glyph, style.fg(*color));
                at += glyph.width() as u16 + 1;
            }
            buf.set_string(at, area.y, &tab.label, style);
            at += tab.label.width() as u16 + 1;
            let (mark, mark_style) = if tab.modified {
                ("●", style.patch(theme.get("ui.tabbar.modified")))
            } else {
                ("×", style.patch(theme.get("ui.tabbar.close")))
            };
            buf.set_string(at, area.y, mark, mark_style);
            buf.set_string(x + width - 1, area.y, "│", theme.get("ui.tabbar.border"));
            x += width;
        }
    }
}
