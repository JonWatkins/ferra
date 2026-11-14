//! The settings editor (`editor/settings.rs`), in the windows' place:
//!
//! ```text
//!  Settings                                          2 changed
//!  ⌕ Search settings (/)
//! ─────────────────────────────────────────────────────────────
//!  Editing          │▎Tab Width
//!  Display          │ Width of a tab stop, and of one indent level.
//!  Workbench        │ [ 4      ]
//!  Search           │
//!  Files            │ Expand Tab
//!  …                │ Indent with spaces (true) or tab characters…
//!                   │ [✓] on
//!  ↵ change  h/l choose  u reset  / search  Tab categories  e config.toml  q close
//! ```
//!
//! A bar left of a setting's name marks one that isn't at its default.

use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};
use toml::Value;
use unicode_width::UnicodeWidthStr;

use super::text::truncate_end;
use crate::config::Theme;
use crate::config::schema::{CATEGORIES, Kind, Setting};
use crate::editor::Editor;
use crate::editor::settings::{SettingsFocus, SettingsView};

/// Where its parts went, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsMap {
    pub area: Rect,
    pub search: Rect,
    /// Each category's row.
    pub categories: Vec<(Rect, usize)>,
    /// Each setting drawn: its rows, its value's, and its index in the
    /// list.
    pub items: Vec<(Rect, Rect, usize)>,
    /// The settings' column (the wheel scrolls it).
    pub list: Rect,
}

pub struct SettingsWidget<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
}

/// The parts of `area`: the title row, the search field, the categories'
/// column, the settings' column, and the hints' row.
struct Parts {
    title: Rect,
    search: Rect,
    line: Rect,
    categories: Option<Rect>,
    list: Rect,
    hints: Rect,
}

fn parts(area: Rect) -> Parts {
    let row = |y: u16| Rect::new(area.x, y, area.width, 1);
    let body_y = area.y + 3;
    // A blank row between the last setting and the keys' row.
    let body_h = area.height.saturating_sub(5);
    // Narrow: no categories' column (Tab still goes through them).
    let side = if area.width >= 60 {
        (area.width / 4).clamp(16, 24)
    } else {
        0
    };
    let categories = (side > 0).then(|| Rect::new(area.x, body_y, side, body_h));
    let list_x = area.x + side + u16::from(side > 0);
    let list = Rect::new(
        list_x + 1,
        body_y,
        area.right().saturating_sub(list_x + 2),
        body_h,
    );
    Parts {
        title: row(area.y),
        search: Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1),
        line: row(area.y + 2),
        categories,
        list,
        hints: row(area.bottom().saturating_sub(1)),
    }
}

/// `text` wrapped at spaces to lines of at most `width` columns.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.width() + 1 + word.width() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// How a value shows in its control.
fn value_text(setting: &Setting, value: Option<&Value>) -> String {
    match (&setting.kind, value) {
        (Kind::Bool, Some(Value::Boolean(true))) => "[✓] on".into(),
        (Kind::Bool, _) => "[ ] off".into(),
        (Kind::Choice(names), Some(Value::String(name))) => {
            let others = names.len().saturating_sub(1);
            if others > 0 {
                format!("‹ {name} ›")
            } else {
                name.clone()
            }
        }
        (Kind::List, Some(Value::Array(items))) => {
            let names: Vec<String> = items
                .iter()
                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
                .collect();
            if names.is_empty() {
                "(none) · edit in config.toml".into()
            } else {
                format!("{} · edit in config.toml", names.join(", "))
            }
        }
        (_, Some(Value::String(text))) => format!("{text:?}"),
        (_, Some(other)) => other.to_string(),
        (_, None) => String::new(),
    }
}

/// The rows a setting takes: its name, description, value and a gap.
fn height(setting: &Setting, width: u16) -> usize {
    3 + wrap(&setting.description, width as usize).len()
}

impl<'a> SettingsWidget<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme) -> Self {
        Self { editor, theme }
    }

    /// Keeps the selected setting in view: the first one to draw.
    fn first_shown(view: &SettingsView, shown: &[&Setting], list: Rect) -> usize {
        let mut first = view.scroll.get().min(view.selected);
        let fits = |first: usize| {
            shown[first..=view.selected.min(shown.len().saturating_sub(1))]
                .iter()
                .map(|s| height(s, list.width))
                .sum::<usize>()
                // The last one's gap needn't show.
                .saturating_sub(1)
                <= list.height as usize
        };
        while first < view.selected && !fits(first) {
            first += 1;
        }
        view.scroll.set(first);
        first
    }

    /// Where things are, for the mouse (as `render` lays them out).
    pub fn map(editor: &Editor, area: Rect) -> SettingsMap {
        let Some(view) = editor.settings() else {
            return SettingsMap::default();
        };
        let parts = parts(area);
        let categories = parts
            .categories
            .map(|column| {
                (0..CATEGORIES.len())
                    .filter(|i| (*i as u16) < column.height)
                    .map(|i| (Rect::new(column.x, column.y + i as u16, column.width, 1), i))
                    .collect()
            })
            .unwrap_or_default();
        let shown = view.shown();
        let mut items = Vec::new();
        let mut y = parts.list.y;
        for (i, setting) in shown.iter().enumerate().skip(view.scroll.get()) {
            if y >= parts.list.bottom() {
                break;
            }
            let rows = height(setting, parts.list.width) as u16;
            let block = Rect::new(
                parts.list.x,
                y,
                parts.list.width,
                rows.min(parts.list.bottom() - y),
            );
            let value_y = y + rows - 2;
            let value = Rect::new(
                parts.list.x + 1,
                value_y,
                parts.list.width.saturating_sub(1),
                1,
            );
            items.push((block, value, i));
            y += rows;
        }
        SettingsMap {
            area,
            search: parts.search,
            categories,
            items,
            list: parts.list,
        }
    }

    /// Where the terminal's cursor goes while a field is typed in.
    pub fn cursor_position(editor: &Editor, area: Rect) -> Option<(u16, u16)> {
        let view = editor.settings()?;
        let parts = parts(area);
        match view.focus {
            SettingsFocus::Search => {
                let x = parts.search.x + 3;
                let width = parts.search.width.saturating_sub(4);
                let scroll = super::find::field_scroll(&view.query, width);
                Some((x + (view.query.cursor() - scroll) as u16, parts.search.y))
            }
            SettingsFocus::Edit => {
                let map = Self::map(editor, area);
                let (_, value, _) = map.items.iter().find(|(_, _, i)| *i == view.selected)?;
                let width = value.width.saturating_sub(2).min(40);
                let scroll = super::find::field_scroll(&view.edit, width);
                Some((value.x + 1 + (view.edit.cursor() - scroll) as u16, value.y))
            }
            _ => None,
        }
    }

    fn draw_field(
        &self,
        buf: &mut Buffer,
        rect: Rect,
        field: &crate::editor::text_field::TextField,
        focused: bool,
    ) {
        let theme = self.theme;
        let style = if focused {
            theme.get("ui.find.field.focused")
        } else {
            theme.get("ui.find.field")
        };
        buf.set_style(rect, style);
        let width = rect.width.saturating_sub(2);
        let shown: String = field
            .text()
            .chars()
            .skip(super::find::field_scroll(field, width))
            .take(width as usize)
            .collect();
        let text_style = if field.is_selected() {
            style.patch(theme.get("ui.find.field.selected"))
        } else {
            style
        };
        buf.set_stringn(rect.x + 1, rect.y, shown, width as usize, text_style);
    }
}

impl Widget for SettingsWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(view) = self.editor.settings() else {
            return;
        };
        let theme = self.theme;
        let base = theme.get("ui.settings");
        // Over the windows: nothing of them shows through.
        ratatui::widgets::Clear.render(area, buf);
        buf.set_style(area, base);
        if area.height < 6 || area.width < 20 {
            return;
        }
        let parts = parts(area);
        let all = crate::config::schema::settings();
        let changed = all
            .iter()
            .filter(|s| self.editor.setting_modified(s))
            .count();

        // The title, and how many settings are changed.
        buf.set_string(
            parts.title.x + 1,
            parts.title.y,
            "Settings",
            theme.get("ui.settings.title"),
        );
        if changed > 0 {
            let text = format!("{changed} changed");
            let x = parts.title.right().saturating_sub(text.width() as u16 + 1);
            buf.set_string(x, parts.title.y, text, theme.get("ui.settings.modified"));
        }

        // The search box.
        let searching = view.focus == SettingsFocus::Search;
        buf.set_string(
            parts.search.x,
            parts.search.y,
            "⌕",
            theme.get("ui.settings.hint"),
        );
        let field = Rect::new(
            parts.search.x + 2,
            parts.search.y,
            parts.search.width.saturating_sub(2),
            1,
        );
        if view.query.text().is_empty() && !searching {
            buf.set_style(field, theme.get("ui.find.field"));
            buf.set_string(
                field.x + 1,
                field.y,
                "Search settings (/)",
                theme.get("ui.find.hint"),
            );
        } else {
            self.draw_field(buf, field, &view.query, searching);
        }
        let border = theme.get("ui.settings.border");
        for x in parts.line.left()..parts.line.right() {
            buf[(x, parts.line.y)].set_symbol("─").set_style(border);
        }

        // The categories.
        if let Some(column) = parts.categories {
            for (i, name) in CATEGORIES.iter().enumerate() {
                if i as u16 >= column.height {
                    break;
                }
                let y = column.y + i as u16;
                let row = Rect::new(column.x, y, column.width, 1);
                let style = if view.searching() {
                    theme.get("ui.settings.hint")
                } else if i == view.category && view.focus == SettingsFocus::Categories {
                    theme.get("ui.settings.category.selected")
                } else if i == view.category {
                    theme.get("ui.settings.category.shown")
                } else {
                    theme.get("ui.settings.category")
                };
                buf.set_style(row, style);
                buf.set_stringn(column.x + 1, y, name, column.width as usize - 1, style);
            }
            let x = column.right();
            for y in column.top()..column.bottom() {
                buf[(x, y)].set_symbol("│").set_style(border);
            }
        }

        // The settings.
        let shown = view.shown();
        let list = parts.list;
        if shown.is_empty() {
            buf.set_string(
                list.x + 1,
                list.y,
                "No settings match",
                theme.get("ui.settings.hint"),
            );
        } else {
            let first = Self::first_shown(view, &shown, list);
            let mut y = list.y;
            let in_list = matches!(view.focus, SettingsFocus::List | SettingsFocus::Edit);
            for (i, setting) in shown.iter().enumerate().skip(first) {
                if y >= list.bottom() {
                    break;
                }
                let rows = height(setting, list.width) as u16;
                let selected = i == view.selected;
                if selected && in_list {
                    let block = Rect::new(list.x, y, list.width, (rows - 1).min(list.bottom() - y));
                    buf.set_style(block, theme.get("ui.settings.selected"));
                }
                let modified = self.editor.setting_modified(setting);
                let mut line = y;
                let put = |buf: &mut Buffer, line: u16, x: u16, text: &str, style: Style| {
                    if line < list.bottom() {
                        let room = list.right().saturating_sub(x) as usize;
                        buf.set_stringn(x, line, truncate_end(text, room), room, style);
                    }
                };
                // The name (with its category, among search results), and a
                // bar for one that's changed.
                if modified {
                    put(buf, line, list.x, "▎", theme.get("ui.settings.modified"));
                }
                let name_style = theme.get("ui.settings.name");
                let mut x = list.x + 1;
                if view.searching() {
                    let prefix = format!("{} › ", setting.category);
                    put(buf, line, x, &prefix, theme.get("ui.settings.hint"));
                    x += prefix.width() as u16;
                }
                put(buf, line, x, &setting.title, name_style);
                line += 1;
                for text in wrap(&setting.description, list.width as usize) {
                    put(
                        buf,
                        line,
                        list.x + 1,
                        &text,
                        theme.get("ui.settings.description"),
                    );
                    line += 1;
                }
                // Its value.
                let value = self.editor.setting_value(setting);
                if selected && view.focus == SettingsFocus::Edit {
                    let width = list.width.saturating_sub(2).min(40);
                    if line < list.bottom() {
                        self.draw_field(
                            buf,
                            Rect::new(list.x + 1, line, width + 2, 1),
                            &view.edit,
                            true,
                        );
                    }
                } else {
                    let text = format!(" {} ", value_text(setting, value.as_ref()));
                    let style = match (&setting.kind, &value) {
                        (Kind::Bool, Some(Value::Boolean(true))) => {
                            theme.get("ui.settings.value.on")
                        }
                        (Kind::List, _) => theme.get("ui.settings.hint"),
                        _ => theme.get("ui.settings.value"),
                    };
                    // A field's width, at least, so it looks like one.
                    let text = match setting.kind {
                        Kind::Number | Kind::Text => format!("{text:<10}"),
                        _ => text,
                    };
                    put(buf, line, list.x + 1, &text, style);
                }
                y += rows;
            }
        }

        // The keys for what has focus.
        let hints = match view.focus {
            SettingsFocus::List => {
                "↵ change  h/l choose  u reset  / search  Tab categories  e config.toml  q close"
            }
            SettingsFocus::Categories => {
                "j/k category  ↵ settings  / search  e config.toml  q close"
            }
            SettingsFocus::Search => "type to search  ↵ settings  Esc clear",
            SettingsFocus::Edit => "↵ set  Esc cancel",
        };
        buf.set_stringn(
            parts.hints.x + 1,
            parts.hints.y,
            hints,
            parts.hints.width.saturating_sub(1) as usize,
            theme.get("ui.settings.hint"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_at_spaces() {
        assert_eq!(
            wrap("Width of a tab stop, and of one indent level.", 20),
            ["Width of a tab stop,", "and of one indent", "level."]
        );
        assert!(wrap("", 20).is_empty());
    }
}
