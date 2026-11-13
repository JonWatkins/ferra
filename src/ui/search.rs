//! The Search view in the sidebar: the query field with its toggles, a
//! line saying what was found, and the results, grouped by file.
//!
//! ```text
//!  SEARCH
//!  alpha            Aa W .*
//!  3 results in 2 files
//!  ▾ main.rs src          2
//!      fn alpha() {}
//!      let b = alpha();
//!  ▾ notes.txt            1
//!      alpha
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::text::truncate_end;
use crate::config::{Icons, Theme};
use crate::editor::Editor;
use crate::editor::project_search::SearchRow;

/// The toggles after the field: the chip, plain and with a Nerd Font.
const TOGGLES: [(&str, &str); 3] = [("Aa", "\u{eab1}"), ("W", "\u{eb7e}"), (".*", "\u{eb38}")];

/// Rows above the results: the title, the field, the status.
const HEAD: u16 = 3;

/// Where the view's parts are, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchHits {
    /// The query field (its text area).
    pub field: Rect,
    /// Each toggle's chip: match case, whole word, regex.
    pub toggles: Vec<Rect>,
}

pub struct SearchViewWidget<'a> {
    editor: &'a Editor,
    focused: bool,
    theme: &'a Theme,
}

impl<'a> SearchViewWidget<'a> {
    pub fn new(editor: &'a Editor, focused: bool, theme: &'a Theme) -> Self {
        Self {
            editor,
            focused,
            theme,
        }
    }

    /// The rows of its results list (a blank row kept under it, so the
    /// last result isn't against the status bar).
    pub fn list(area: Rect) -> Rect {
        Rect::new(
            area.x,
            area.y + HEAD.min(area.height),
            area.width,
            area.height.saturating_sub(HEAD + 1),
        )
    }

    fn chips(editor: &Editor) -> [&'static str; 3] {
        let nerd = editor.options.icons == Icons::Nerd;
        TOGGLES.map(|(plain, nerd_icon)| if nerd { nerd_icon } else { plain })
    }

    /// Where the field and the toggles go.
    pub fn hits(editor: &Editor, area: Rect) -> SearchHits {
        let y = area.y + 1;
        let chips = Self::chips(editor);
        let chips_width: u16 = chips.iter().map(|c| c.width() as u16 + 2).sum();
        let right = area.right().saturating_sub(1);
        let mut x = right.saturating_sub(chips_width);
        let toggles = chips
            .iter()
            .map(|chip| {
                let width = chip.width() as u16 + 2;
                let rect = Rect::new(x, y, width, 1);
                x += width;
                rect
            })
            .collect();
        let field_x = area.x + 1;
        let field_width = right
            .saturating_sub(chips_width + 1)
            .saturating_sub(field_x);
        SearchHits {
            field: Rect::new(field_x, y, field_width, 1),
            toggles,
        }
    }

    /// Where the terminal cursor goes while the query's typed.
    pub fn cursor_position(editor: &Editor, area: Rect) -> Option<(u16, u16)> {
        let hits = Self::hits(editor, area);
        let field = &editor.project_search().fields.find;
        let width = hits.field.width.saturating_sub(2);
        let scroll = super::find::field_scroll(field, width);
        let before: String = field
            .text()
            .chars()
            .skip(scroll)
            .take(field.cursor() - scroll)
            .collect();
        Some((hits.field.x + 1 + before.width() as u16, hits.field.y))
    }
}

impl Widget for SearchViewWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 8 || area.height == 0 {
            return;
        }
        let theme = self.theme;
        let editor = self.editor;
        let search = editor.project_search();
        buf.set_style(area, theme.get("ui.sidebar"));
        buf.set_string(area.x, area.y, " SEARCH", theme.get("ui.sidebar.title"));
        if area.height < HEAD {
            return;
        }

        // The field, then the toggles.
        let hits = Self::hits(editor, area);
        let typing = editor.search_typing();
        let style = if typing {
            theme.get("ui.find.field.focused")
        } else {
            theme.get("ui.find.field")
        };
        buf.set_style(hits.field, style);
        let field = &search.fields.find;
        let width = hits.field.width.saturating_sub(2);
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
        if field.text().is_empty() && !typing {
            buf.set_stringn(
                hits.field.x + 1,
                hits.field.y,
                "Search",
                width as usize,
                theme.get("ui.find.hint"),
            );
        } else {
            buf.set_stringn(
                hits.field.x + 1,
                hits.field.y,
                shown,
                width as usize,
                text_style,
            );
        }
        let on = [
            search.fields.match_case,
            search.fields.whole_word,
            search.fields.regex,
        ];
        for ((rect, chip), on) in hits.toggles.iter().zip(Self::chips(editor)).zip(on) {
            let style = if on {
                theme.get("ui.find.toggle.on")
            } else {
                theme.get("ui.find.toggle")
            };
            buf.set_stringn(
                rect.x,
                rect.y,
                format!(" {chip} "),
                rect.width as usize,
                style,
            );
        }

        // What was found.
        let status = editor.search_status();
        let status_style =
            if search.fields.error().is_some() || (status == "No results" && !search.searching()) {
                theme.get("ui.find.none")
            } else {
                theme.get("ui.find.count")
            };
        buf.set_stringn(
            area.x + 1,
            area.y + 2,
            &status,
            area.width.saturating_sub(2) as usize,
            status_style,
        );

        // The results.
        let list = Self::list(area);
        let first = search.first_shown(list.height as usize);
        // Only the rows shown are made.
        let rows = search.rows_from(first, list.height as usize);
        let width = list.width as usize;
        let match_style = theme.get("ui.sidebar.search.match");
        for (offset, (i, row)) in rows
            .iter()
            .enumerate()
            .map(|(o, r)| (first + o, r))
            .enumerate()
        {
            let y = list.y + offset as u16;
            let line = Rect::new(list.x, y, list.width, 1);
            if self.focused && i == search.selected() {
                buf.set_style(line, theme.get("ui.sidebar.selected"));
            }
            match row {
                SearchRow::File(file, open) => {
                    let count = file.count().to_string();
                    let marker = if *open { " ▾ " } else { " ▸ " };
                    let name = file
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let dir = file
                        .path
                        .parent()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default();
                    let x = buf
                        .set_stringn(list.x, y, marker, width, theme.get("ui.sidebar.marker"))
                        .0;
                    // With a Nerd Font, its icon.
                    let x = if editor.options.icons == Icons::Nerd {
                        let (glyph, color) = super::icons::for_file(&file.path, &editor.languages);
                        buf.set_string(x, y, &glyph, theme.get("ui.sidebar").fg(color));
                        x + 2
                    } else {
                        x
                    };
                    let used = (x - list.x) as usize;
                    let room = width.saturating_sub(used + count.width() + 3);
                    let x = buf
                        .set_stringn(
                            x,
                            y,
                            truncate_end(&name, room),
                            room,
                            theme.get("ui.sidebar.search.file"),
                        )
                        .0;
                    let room = room.saturating_sub(name.width() + 1);
                    if room > 1 && !dir.is_empty() {
                        buf.set_stringn(
                            x + 1,
                            y,
                            truncate_end(&dir, room),
                            room,
                            theme.get("ui.sidebar.marker"),
                        );
                    }
                    // A badge, as VS Code's.
                    let count_x = list.right().saturating_sub(count.width() as u16 + 3);
                    buf.set_string(
                        count_x,
                        y,
                        format!(" {count} "),
                        theme.get("ui.sidebar.search.count"),
                    );
                }
                SearchRow::Line(_, found) => {
                    draw_line(
                        buf,
                        list.x + 4,
                        y,
                        width.saturating_sub(5),
                        found,
                        theme.get("ui.sidebar.search.line"),
                        match_style,
                    );
                }
            }
        }
    }
}

/// A matching line, its indentation dropped and, if the first match would
/// be off the end, cut from the start (with `…`) to show it; matches
/// highlighted.
fn draw_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: usize,
    found: &crate::editor::project_search::LineMatch,
    line_style: ratatui::style::Style,
    match_style: ratatui::style::Style,
) {
    if width == 0 {
        return;
    }
    let chars: Vec<char> = found.text.chars().collect();
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let first = found.ranges.first().map_or(indent, |r| r.0);
    // Some text before the first match, if the line's too long for it.
    let start = if first + 1 > indent + width.saturating_sub(8) {
        first.saturating_sub(8).max(indent)
    } else {
        indent
    };
    let mut cx = x;
    let right = x + width as u16;
    if start > indent {
        buf.set_string(cx, y, "…", line_style);
        cx += 1;
    }
    for (i, c) in chars.iter().enumerate().skip(start) {
        let w = unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0) as u16;
        if cx + w > right {
            break;
        }
        let in_match = found.ranges.iter().any(|&(s, e)| (s..e).contains(&i));
        let cell = &mut buf[(cx, y)];
        cell.set_char(if *c == '\t' { ' ' } else { *c });
        if in_match {
            cell.set_style(match_style);
        } else {
            cell.set_style(line_style);
        }
        cx += w.max(1);
    }
}
