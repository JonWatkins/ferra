//! The bottom panel (`editor/panel.rs`): a row of tabs, then the tab's
//! body: the problems by file, the Output's lines, or the terminal
//! (`terminal.rs` draws that, with these tabs over its title).
//!
//! ```text
//!  PROBLEMS 2   OUTPUT   TERMINAL                ↵ go  Tab next  Esc editor
//!  ▾ src/main.rs  2
//!    ⊗ mismatched types  rustc  [Ln 3, Col 9]
//!    ⚠ unused variable: `x`  rustc  [Ln 7, Col 5]
//! ```

use std::path::Path;

use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};
use unicode_width::UnicodeWidthStr;

use super::screen_map::{PanelPart, ScreenMap, Target};
use super::text::truncate_end;
use crate::config::Theme;
use crate::editor::diagnostics::Severity as Level;
use crate::editor::panel::{LocationRow, PanelTab, ProblemRow};
use crate::editor::{Editor, Mode};

/// The tabs along `row`: their columns.
fn tab_spans(editor: &Editor, row: Rect) -> Vec<(u16, u16, PanelTab, String)> {
    let problems = editor.problems().len();
    let places = editor
        .panel()
        .locations
        .as_ref()
        .map_or(0, |l| l.items.len());
    let mut x = row.x + 1;
    editor
        .panel_tabs()
        .into_iter()
        .map(|tab| {
            let label = match tab {
                PanelTab::Problems if problems > 0 => format!("{} {problems}", tab.title()),
                PanelTab::Locations => format!("{} {places}", tab.title()),
                _ => tab.title().to_string(),
            };
            let from = x;
            x += label.width() as u16 + 3;
            (from, from + label.width() as u16, tab, label)
        })
        .collect()
}

/// Draws the tabs along the panel's title row `row` (over the terminal's,
/// when it's shown), the one shown underlined. Returns their columns.
pub fn draw_tabs(
    editor: &Editor,
    theme: &Theme,
    row: Rect,
    buf: &mut Buffer,
) -> Vec<(u16, u16, PanelTab)> {
    let shown = editor.panel_shown();
    let focused = matches!(editor.mode(), Mode::Panel | Mode::Terminal);
    let spans = tab_spans(editor, row);
    let end = spans.last().map_or(row.x, |(_, to, ..)| *to + 1);
    buf.set_style(
        Rect::new(row.x, row.y, end.saturating_sub(row.x).min(row.width), 1),
        theme.get("ui.terminal.title"),
    );
    // The terminal's own title goes: the tabs say it.
    for x in row.x..end.min(row.right()) {
        buf[(x, row.y)].set_symbol(" ");
    }
    let mut out = Vec::new();
    for (from, to, tab, label) in spans {
        if to > row.right() {
            break;
        }
        let style = if Some(tab) == shown && focused {
            theme.get("ui.terminal.title.focused")
        } else if Some(tab) == shown {
            theme.get("ui.panel.tab.shown")
        } else {
            theme.get("ui.panel.tab")
        };
        buf.set_string(from, row.y, &label, style);
        out.push((from, to, tab));
    }
    out
}

pub struct PanelView<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
}

impl<'a> PanelView<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme) -> Self {
        Self { editor, theme }
    }

    /// The list's rows in `area` (the panel, its title row included).
    fn list(area: Rect) -> Rect {
        Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        )
    }

    /// The first row to draw: the selected problem in sight, or the
    /// Output's end (unless scrolled back).
    fn first(editor: &Editor, list: Rect) -> usize {
        let panel = editor.panel();
        let height = list.height.max(1) as usize;
        match editor.panel_shown() {
            Some(PanelTab::Problems | PanelTab::Locations) => {
                let mut first = panel.scroll.get();
                if panel.selected < first {
                    first = panel.selected;
                } else if panel.selected >= first + height {
                    first = panel.selected + 1 - height;
                }
                panel.scroll.set(first);
                first
            }
            Some(PanelTab::Output) => {
                let end = panel.output.len().saturating_sub(height);
                let first = if panel.output_held {
                    panel.scroll.get().min(end)
                } else {
                    end
                };
                panel.scroll.set(first);
                first
            }
            _ => 0,
        }
    }

    /// Records where its parts are in `area` (the panel, its title row
    /// included), for the mouse.
    pub fn record(editor: &Editor, area: Rect, screen: &mut ScreenMap) {
        let panel = Target::Panel;
        let row = Rect::new(area.x, area.y, area.width, 1);
        let list = Self::list(area);
        screen.add(area, panel(PanelPart::Rest));
        match editor.panel_shown() {
            Some(PanelTab::Terminal) => {
                screen.add(list, panel(PanelPart::Terminal));
                let shells = editor.terminals();
                if let Some(rows) = super::terminal::shell_list(editor, area) {
                    let first = super::terminal::list_first(shells.shown_index(), rows);
                    screen.rows(rows, first, shells.len(), |i| panel(PanelPart::Shell(i)));
                    let edge = super::terminal::list_edge(rows);
                    screen.add(edge, panel(PanelPart::ListEdge));
                }
            }
            Some(tab @ (PanelTab::Problems | PanelTab::Locations)) => {
                let len = if tab == PanelTab::Problems {
                    editor.problem_rows().len()
                } else {
                    editor.location_rows().len()
                };
                let first = editor.panel().scroll.get();
                screen.rows(list, first, len, |i| panel(PanelPart::Row(i)));
            }
            _ => {}
        }
        screen.add(row, panel(PanelPart::Title));
        for (from, to, tab, _) in tab_spans(editor, row) {
            let span = Rect::new(from, row.y, to - from, 1);
            screen.add(span.intersection(row), panel(PanelPart::Tab(tab)));
        }
        if editor.panel_shown() == Some(PanelTab::Terminal) {
            for (rect, part) in super::terminal::buttons(area) {
                screen.add(rect, panel(part));
            }
        }
    }
}

fn shown_path(path: &Path) -> String {
    let here = std::env::current_dir().unwrap_or_default();
    let here = std::fs::canonicalize(&here).unwrap_or(here);
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    full.strip_prefix(&here)
        .unwrap_or(&full)
        .to_string_lossy()
        .into_owned()
}

impl Widget for PanelView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 10 {
            return;
        }
        let theme = self.theme;
        let editor = self.editor;
        let base = theme.get("ui.terminal");
        buf.set_style(area, base);
        let row = Rect::new(area.x, area.y, area.width, 1);
        buf.set_style(row, theme.get("ui.terminal.title"));
        let tabs = draw_tabs(editor, theme, row, buf);
        let focused = editor.mode() == Mode::Panel;
        // The keys, at the right.
        let hints = match editor.panel_shown() {
            Some(PanelTab::Problems | PanelTab::Locations) if focused => {
                "↵ go   Tab next   Esc editor   q hide  "
            }
            Some(PanelTab::Output) if focused => "j/k scroll   G end   Tab next   Esc editor  ",
            _ => "",
        };
        let tabs_end = tabs.last().map_or(row.x, |(_, to, _)| *to);
        if hints.width() as u16 + tabs_end + 4 <= row.right() {
            let x = row.right() - hints.width() as u16;
            buf.set_string(x, row.y, hints, theme.get("ui.terminal.hint"));
        }

        let list = Self::list(area);
        let first = Self::first(editor, list);
        let width = list.width as usize;
        let put = |buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style| -> u16 {
            let room = (list.right().saturating_sub(x)) as usize;
            let text = truncate_end(text, room);
            buf.set_stringn(x, y, &text, room, style);
            x + text.width() as u16
        };
        match editor.panel_shown() {
            Some(PanelTab::Problems) => {
                let rows = editor.problem_rows();
                if rows.is_empty() {
                    put(
                        buf,
                        list.x + 2,
                        list.y,
                        "No problems have been found in the open files.",
                        theme.get("ui.sidebar.empty"),
                    );
                    return;
                }
                let selected = editor.panel().selected;
                for (i, row) in rows.iter().enumerate().skip(first) {
                    let y = list.y + (i - first) as u16;
                    if y >= list.bottom() {
                        break;
                    }
                    match row {
                        ProblemRow::File { path, count } => {
                            let x = put(buf, list.x + 1, y, "▾ ", theme.get("ui.sidebar.marker"));
                            let x = put(
                                buf,
                                x,
                                y,
                                &shown_path(path),
                                theme.get("ui.sidebar.search.file"),
                            );
                            put(
                                buf,
                                x + 1,
                                y,
                                &format!(" {count} "),
                                theme.get("ui.sidebar.search.count"),
                            );
                        }
                        ProblemRow::Problem { diagnostic, .. } => {
                            if i == selected {
                                let style = if focused {
                                    theme.get("ui.sidebar.selected")
                                } else {
                                    theme.get("ui.git.selected.unfocused")
                                };
                                buf.set_style(Rect::new(list.x, y, list.width, 1), style);
                            }
                            let (icon, scope) = match diagnostic.severity {
                                Level::Error => ("⊗", "ui.statusline.diagnostic.error"),
                                Level::Warning => ("⚠", "ui.statusline.diagnostic.warning"),
                                _ => ("ⓘ", "ui.statusline.diagnostic.info"),
                            };
                            let x = put(buf, list.x + 3, y, icon, theme.get(scope));
                            let message = diagnostic.message.lines().next().unwrap_or_default();
                            let x = put(buf, x + 1, y, message, Style::default());
                            if let Some(source) = &diagnostic.source {
                                let x = put(buf, x + 2, y, source, theme.get("ui.panel.detail"));
                                let _ = x;
                            }
                            let place = format!(
                                "[Ln {}, Col {}]",
                                diagnostic.start.row + 1,
                                diagnostic.start.col + 1
                            );
                            let end = list.x as usize + width;
                            let used = rows_width(buf, list, y);
                            if used + place.width() + 2 <= width {
                                let at = (end - place.width() - 1) as u16;
                                buf.set_string(at, y, place, theme.get("ui.panel.detail"));
                            }
                        }
                    }
                }
            }
            Some(PanelTab::Locations) => {
                let selected = editor.panel().selected;
                for (i, row) in editor.location_rows().iter().enumerate().skip(first) {
                    let y = list.y + (i - first) as u16;
                    if y >= list.bottom() {
                        break;
                    }
                    match row {
                        LocationRow::Title(title) => {
                            put(buf, list.x + 1, y, title, theme.get("ui.panel.detail"));
                        }
                        LocationRow::File { path, count } => {
                            let x = put(buf, list.x + 1, y, "▾ ", theme.get("ui.sidebar.marker"));
                            let x = put(
                                buf,
                                x,
                                y,
                                &shown_path(path),
                                theme.get("ui.sidebar.search.file"),
                            );
                            put(
                                buf,
                                x + 1,
                                y,
                                &format!(" {count} "),
                                theme.get("ui.sidebar.search.count"),
                            );
                        }
                        LocationRow::Item(item) => {
                            if i == selected {
                                let style = if focused {
                                    theme.get("ui.sidebar.selected")
                                } else {
                                    theme.get("ui.git.selected.unfocused")
                                };
                                buf.set_style(Rect::new(list.x, y, list.width, 1), style);
                            }
                            put(buf, list.x + 3, y, &item.note, Style::default());
                            let place = format!(
                                "[Ln {}, Col {}]",
                                item.position.row + 1,
                                item.position.col + 1
                            );
                            let end = list.x as usize + width;
                            let used = rows_width(buf, list, y);
                            if used + place.width() + 2 <= width {
                                let at = (end - place.width() - 1) as u16;
                                buf.set_string(at, y, place, theme.get("ui.panel.detail"));
                            }
                        }
                    }
                }
            }
            Some(PanelTab::Output) => {
                let lines = &editor.panel().output;
                if lines.is_empty() {
                    put(
                        buf,
                        list.x + 2,
                        list.y,
                        "Nothing yet.",
                        theme.get("ui.sidebar.empty"),
                    );
                    return;
                }
                for (i, line) in lines.iter().enumerate().skip(first) {
                    let y = list.y + (i - first) as u16;
                    if y >= list.bottom() {
                        break;
                    }
                    let x = put(
                        buf,
                        list.x + 1,
                        y,
                        &format!("[{}] ", line.time),
                        theme.get("ui.panel.detail"),
                    );
                    let style = if line.error {
                        theme.get("ui.message.error")
                    } else {
                        Style::default()
                    };
                    put(buf, x, y, &line.text, style);
                }
            }
            _ => {}
        }
    }
}

/// How far along row `y` of `list` has text (the last non-space column).
fn rows_width(buf: &Buffer, list: Rect, y: u16) -> usize {
    (list.x..list.right())
        .rev()
        .find(|&x| buf[(x, y)].symbol() != " ")
        .map_or(0, |x| (x - list.x + 1) as usize)
}
