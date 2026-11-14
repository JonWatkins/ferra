//! The keybinding editor (`editor/keybindings.rs`), in the windows' place:
//!
//! ```text
//!  Keyboard Shortcuts                                   312 of 790
//!  ⌕  Search commands (/), or press keys (r)
//! ─────────────────────────────────────────────────────────────────
//!  Command                 Keys             Mode         Source
//!  write                   ⌃S               insert       keymap
//!  write                   ⌃K               insert       user
//!  ...
//!  Save the file, or write to [path] (! to overwrite)
//!  ↵ change  a add  d remove  u reset  / search  r search by keys  q close
//! ```

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
use unicode_width::UnicodeWidthStr;

use super::find::key_label;
use super::text::truncate_end;
use crate::config::Theme;
use crate::editor::Editor;
use crate::editor::keybindings::{BindingSource, KeysFocus, Recording};

const KEYS: u16 = 18;
const MODE: u16 = 13;
const SOURCE: u16 = 9;

/// Where its parts went, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeybindingsMap {
    pub area: Rect,
    pub search: Rect,
    /// The rows of the table, and the shown row on the first.
    pub list: Rect,
    pub first: usize,
}

pub struct KeybindingsWidget<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
}

struct Parts {
    title: Rect,
    search: Rect,
    line: Rect,
    header: Rect,
    list: Rect,
    detail: Rect,
    hints: Rect,
}

fn parts(area: Rect) -> Parts {
    let row = |y: u16| Rect::new(area.x, y, area.width, 1);
    let inner = |y: u16| Rect::new(area.x + 1, y, area.width.saturating_sub(2), 1);
    Parts {
        title: row(area.y),
        search: inner(area.y + 1),
        line: row(area.y + 2),
        header: inner(area.y + 3),
        list: Rect::new(
            area.x + 1,
            area.y + 4,
            area.width.saturating_sub(2),
            // The detail, a gap and the hints under it.
            area.height.saturating_sub(7),
        ),
        detail: inner(area.bottom().saturating_sub(2)),
        hints: inner(area.bottom().saturating_sub(1)),
    }
}

/// The columns' x positions in `row`: command, keys, mode, source.
fn columns(row: Rect) -> [u16; 4] {
    let narrow = row.width < 70;
    let source = if narrow { 0 } else { SOURCE };
    let right = row.right();
    let source_x = right - source;
    let mode_x = source_x.saturating_sub(MODE);
    let keys_x = mode_x.saturating_sub(KEYS);
    [row.x, keys_x, mode_x, source_x]
}

fn mac() -> bool {
    cfg!(target_os = "macos")
}

impl<'a> KeybindingsWidget<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme) -> Self {
        Self { editor, theme }
    }

    /// The first row to draw, so the selection shows.
    fn first(editor: &Editor, list: Rect) -> usize {
        let Some(view) = editor.keybindings() else {
            return 0;
        };
        let height = list.height.max(1) as usize;
        let mut first = view.scroll.get();
        if view.selected < first {
            first = view.selected;
        } else if view.selected >= first + height {
            first = view.selected + 1 - height;
        }
        view.scroll.set(first);
        first
    }

    pub fn map(editor: &Editor, area: Rect) -> KeybindingsMap {
        let parts = parts(area);
        KeybindingsMap {
            area,
            search: parts.search,
            list: parts.list,
            first: editor.keybindings().map_or(0, |v| v.scroll.get()),
        }
    }

    pub fn cursor_position(editor: &Editor, area: Rect) -> Option<(u16, u16)> {
        let view = editor.keybindings()?;
        (view.focus == KeysFocus::Search).then(|| {
            let parts = parts(area);
            let width = parts.search.width.saturating_sub(4);
            let scroll = super::find::field_scroll(&view.query, width);
            (
                parts.search.x + 3 + (view.query.cursor() - scroll) as u16,
                parts.search.y,
            )
        })
    }
}

impl Widget for KeybindingsWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(view) = self.editor.keybindings() else {
            return;
        };
        let theme = self.theme;
        ratatui::widgets::Clear.render(area, buf);
        buf.set_style(area, theme.get("ui.settings"));
        if area.height < 9 || area.width < 30 {
            return;
        }
        let parts = parts(area);
        let hint = theme.get("ui.settings.hint");
        let shown = view.shown();

        // The title, and how many rows are shown.
        buf.set_string(
            parts.title.x + 1,
            parts.title.y,
            "Keyboard Shortcuts",
            theme.get("ui.settings.title"),
        );
        let count = format!("{} of {}", shown.len(), view_total(view));
        let x = parts.title.right().saturating_sub(count.width() as u16 + 1);
        buf.set_string(x, parts.title.y, count, hint);

        // The search box: text, or the keys pressed to search with.
        let field = Rect::new(
            parts.search.x + 2,
            parts.search.y,
            parts.search.width.saturating_sub(2),
            1,
        );
        buf.set_string(parts.search.x, parts.search.y, "⌕", hint);
        let focused = view.focus == KeysFocus::Search || view.recording == Some(Recording::Search);
        let style = if focused {
            theme.get("ui.find.field.focused")
        } else {
            theme.get("ui.find.field")
        };
        buf.set_style(field, style);
        let width = field.width.saturating_sub(2) as usize;
        if !view.key_filter.is_empty() || view.recording == Some(Recording::Search) {
            let keys = key_label(&view.key_filter, mac());
            let text = format!("Keys: {keys}");
            buf.set_stringn(field.x + 1, field.y, text, width, style);
        } else if view.query.text().is_empty() && !focused {
            buf.set_stringn(
                field.x + 1,
                field.y,
                "Search commands (/), or press keys (r)",
                width,
                theme.get("ui.find.hint"),
            );
        } else {
            let scroll = super::find::field_scroll(&view.query, field.width.saturating_sub(2));
            let text: String = view.query.text().chars().skip(scroll).take(width).collect();
            let text_style = if view.query.is_selected() {
                style.patch(theme.get("ui.find.field.selected"))
            } else {
                style
            };
            buf.set_stringn(field.x + 1, field.y, text, width, text_style);
        }
        let border = theme.get("ui.settings.border");
        for x in parts.line.left()..parts.line.right() {
            buf[(x, parts.line.y)].set_symbol("─").set_style(border);
        }

        // The table.
        let [command_x, keys_x, mode_x, source_x] = columns(parts.header);
        let head = theme.get("ui.settings.name");
        buf.set_string(command_x, parts.header.y, "Command", head);
        buf.set_string(keys_x, parts.header.y, "Keys", head);
        buf.set_string(mode_x, parts.header.y, "Mode", head);
        if source_x < parts.header.right() {
            buf.set_string(source_x, parts.header.y, "Source", head);
        }
        let first = KeybindingsWidget::first(self.editor, parts.list);
        if shown.is_empty() {
            buf.set_string(parts.list.x, parts.list.y, "No bindings match", hint);
        }
        for (i, row) in shown.iter().enumerate().skip(first) {
            let y = parts.list.y + (i - first) as u16;
            if y >= parts.list.bottom() {
                break;
            }
            let line = Rect::new(parts.list.x, y, parts.list.width, 1);
            let selected = i == view.selected;
            if selected {
                buf.set_style(line, theme.get("ui.settings.category.selected"));
            }
            let style = |scope: &str| {
                let base = theme.get(scope);
                if selected {
                    base.patch(theme.get("ui.settings.category.selected"))
                } else {
                    base
                }
            };
            let put = |buf: &mut Buffer, x: u16, end: u16, text: &str, style| {
                let room = end.saturating_sub(x).saturating_sub(1) as usize;
                buf.set_stringn(x, y, truncate_end(text, room), room, style);
            };
            let name_style = match row.source {
                BindingSource::Removed => style("ui.settings.hint"),
                _ => style("ui.settings.category"),
            };
            put(buf, command_x, keys_x, &row.command, name_style);
            let keys = if row.keys.is_empty() {
                "—".to_string()
            } else {
                key_label(&row.keys, mac())
            };
            let keys_style = match row.source {
                BindingSource::Removed => style("ui.settings.hint"),
                BindingSource::Unbound => style("ui.settings.hint"),
                _ => style("ui.settings.value"),
            };
            put(buf, keys_x, mode_x, &keys, keys_style);
            if let Some(mode) = row.mode {
                put(
                    buf,
                    mode_x,
                    source_x,
                    mode.id(),
                    style("ui.settings.description"),
                );
            }
            if source_x < line.right() {
                let source_style = match row.source {
                    BindingSource::User => style("ui.settings.modified"),
                    _ => style("ui.settings.hint"),
                };
                put(
                    buf,
                    source_x,
                    line.right() + 1,
                    row.source.label(),
                    source_style,
                );
            }
        }

        // What's selected, or the keys being pressed for it.
        let detail_width = parts.detail.width as usize;
        match (view.recording, view.current()) {
            (Some(why @ (Recording::Change | Recording::Add)), Some(row)) => {
                let mode = row.mode.unwrap_or(view.home);
                let keys = if view.recorded.is_empty() {
                    "…".to_string()
                } else {
                    key_label(&view.recorded, mac())
                };
                let verb = if why == Recording::Add {
                    "More keys"
                } else {
                    "Keys"
                };
                let mut text = format!("{verb} for {} in {}: {keys}", row.command, mode.id());
                let clashes: Vec<String> = view
                    .conflicts()
                    .iter()
                    .map(|r| format!("{} ({})", r.command, key_label(&r.keys, mac())))
                    .collect();
                if !clashes.is_empty() {
                    text.push_str(&format!("   also: {}", clashes.join(", ")));
                }
                let style = if clashes.is_empty() {
                    theme.get("ui.settings.modified")
                } else {
                    theme.get("ui.message.error")
                };
                buf.set_stringn(parts.detail.x, parts.detail.y, text, detail_width, style);
            }
            (_, Some(row)) => {
                buf.set_stringn(
                    parts.detail.x,
                    parts.detail.y,
                    &row.description,
                    detail_width,
                    theme.get("ui.settings.description"),
                );
            }
            _ => {}
        }

        let hints = match (view.focus, view.recording) {
            (KeysFocus::Record, Some(Recording::Search)) => {
                "press keys to find them  ↵ done  Esc clear"
            }
            (KeysFocus::Record, _) => "press the keys  ↵ keep them  Esc cancel",
            (KeysFocus::Search, _) => "type to search  ↵ the list  Esc clear",
            _ => {
                "↵ change  a add  d remove  u reset  / search  r search by keys  e config.toml  q close"
            }
        };
        buf.set_stringn(
            parts.hints.x,
            parts.hints.y,
            hints,
            parts.hints.width as usize,
            hint,
        );
    }
}

/// How many rows there are in all.
fn view_total(view: &crate::editor::keybindings::KeybindingsView) -> usize {
    view.total()
}
