//! The terminal panel (`editor/terminal_panel.rs`), under the windows: a
//! title row, then the shell's screen; with two or more shells, a list of
//! them down the right, as VS Code's tabs list.
//!
//! ```text
//!  TERMINAL  zsh                          ⌃\ ⌃N editor   ⌃` hide   +  ×
//! ~/Code/ferra $ cargo test                         │ $ zsh
//!    Compiling ferra v0.1.0                         │ $ cargo test
//! ```
//!
//! `ui/terminal.rs` draws the title row and the screen cell by cell (indexed
//! colours stay the user's palette). `ui::draw` takes the panel's rows from the
//! bottom of the windows' side, under their status lines. The panel notes the
//! size it was drawn at, and the app resizes the pty and parser to it after
//! drawing. Its key hints come from the keymap like the find box's;
//! `Keymap::hint_keys` ranks keys that need the kitty protocol (``<C-`>``,
//! `<C-CR>`) after ones every terminal sends.
//!
//! The title row ends in two buttons, a new shell and ending the one shown
//! (codicons with `icons = "nerd"`, else `+` and `×`). The list (`list_area`,
//! `terminal-list-width` columns, at most half the panel) has a row per shell,
//! its icon and title, the one shown highlighted; it's scrolled to keep that
//! one in sight (`list_first`). The line left of it (`list_edge`) drags to
//! resize it. The same functions place
//! them for drawing and for the mouse (`ui/panel.rs` records them).

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use super::screen_map::PanelPart;
use super::text::truncate_end;
use crate::config::Theme;
use crate::terminal::Terminal;

/// The keys the title row shows, as bound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalKeys {
    /// Back to the editor (in the panel).
    pub leave: Option<String>,
    /// Hide the panel (in the panel).
    pub hide: Option<String>,
    /// Go to the panel (from the editor).
    pub focus: Option<String>,
}

pub struct TerminalView<'a> {
    /// The shells, and the one shown.
    shells: &'a [Terminal],
    shown: usize,
    focused: bool,
    keys: &'a TerminalKeys,
    theme: &'a Theme,
    /// Drawn in the bottom panel, under its tabs (no title of its own).
    tabbed: bool,
    /// A Nerd Font's icons (`icons = "nerd"`).
    nerd: bool,
    /// The list's width wanted (`terminal-list-width`).
    list_width: u16,
}

/// The narrowest the list goes.
const LIST_MIN: u16 = 8;

/// Where the list of shells goes in the panel's `area` (its title row
/// included), with two or more: down the right, under the title row,
/// `width` columns (`terminal-list-width`, at most half the panel), a line
/// to its left. None without room.
pub fn list_area(area: Rect, shells: usize, width: u16) -> Option<Rect> {
    if shells < 2 || area.height < 2 || area.width / 2 < LIST_MIN {
        return None;
    }
    let width = width.clamp(LIST_MIN, area.width / 2);
    Some(Rect::new(
        area.right() - width,
        area.y + 1,
        width,
        area.height - 1,
    ))
}

/// The editor's list of shells in the panel's `area`, if it's shown.
pub fn shell_list(editor: &crate::editor::Editor, area: Rect) -> Option<Rect> {
    list_area(
        area,
        editor.terminals().len(),
        editor.options.terminal_list_width,
    )
}

/// The line left of the list, which drags to resize it.
pub fn list_edge(list: Rect) -> Rect {
    Rect::new(list.x - 1, list.y, 1, list.height)
}

/// The first shell the list shows: the one shown kept in sight.
pub fn list_first(shown: usize, list: Rect) -> usize {
    shown.saturating_sub(list.height.saturating_sub(1) as usize)
}

/// The title row's buttons, at its right end: a new shell, and ending the
/// one shown. None without room.
pub fn buttons(area: Rect) -> Vec<(Rect, PanelPart)> {
    if area.width < 30 || area.height == 0 {
        return Vec::new();
    }
    vec![
        (
            Rect::new(area.right() - 6, area.y, 3, 1),
            PanelPart::NewShell,
        ),
        (
            Rect::new(area.right() - 3, area.y, 3, 1),
            PanelPart::EndShell,
        ),
    ]
}

/// The shell's part of the panel's area: under the title row, a column
/// in from the left (so it doesn't run into the sidebar's line), left of
/// the `list` and its line.
fn body(area: Rect, list: Option<Rect>) -> Rect {
    let margin = 1.min(area.width.saturating_sub(1));
    let right = list.map_or(area.right(), |list| list_edge(list).x);
    Rect::new(
        area.x + margin,
        area.y + 1.min(area.height),
        right.saturating_sub(area.x + margin),
        area.height.saturating_sub(1),
    )
}

fn color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

impl<'a> TerminalView<'a> {
    /// The panel showing `shells[shown]`.
    pub fn new(
        shells: &'a [Terminal],
        shown: usize,
        focused: bool,
        keys: &'a TerminalKeys,
        theme: &'a Theme,
    ) -> Self {
        Self {
            shells,
            shown: shown.min(shells.len().saturating_sub(1)),
            focused,
            keys,
            theme,
            tabbed: false,
            nerd: false,
            list_width: 22,
        }
    }

    /// In the bottom panel: its tabs are drawn over the title row.
    pub fn tabbed(mut self) -> Self {
        self.tabbed = true;
        self
    }

    /// The list's width wanted (`terminal-list-width`).
    pub fn list_width(mut self, width: u16) -> Self {
        self.list_width = width;
        self
    }

    /// With a Nerd Font's icons.
    pub fn nerd(mut self, nerd: bool) -> Self {
        self.nerd = nerd;
        self
    }

    /// Where the terminal cursor goes: the shell's cursor, while the panel
    /// has focus (and isn't scrolled back, or hiding it). `list` is where
    /// the list of shells is (`list_area`), if it's shown.
    pub fn cursor_position(
        terminal: &Terminal,
        area: Rect,
        list: Option<Rect>,
    ) -> Option<Position> {
        let screen = terminal.screen();
        if screen.hide_cursor() || screen.scrollback() > 0 || !terminal.is_running() {
            return None;
        }
        let body = body(area, list);
        let (row, col) = screen.cursor_position();
        (row < body.height && col < body.width).then(|| Position::new(body.x + col, body.y + row))
    }

    fn icon(&self, plain: &'static str, nerd: &'static str) -> &'static str {
        if self.nerd { nerd } else { plain }
    }

    fn title_row(&self, terminal: &Terminal, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let title_style = if self.focused {
            theme.get("ui.terminal.title.focused")
        } else {
            theme.get("ui.terminal.title")
        };
        let row = Rect::new(area.x, area.y, area.width, 1);
        buf.set_style(row, theme.get("ui.terminal.title"));

        // The buttons at the right end, then the hints before them.
        let buttons = buttons(area);
        for (rect, part) in &buttons {
            let glyph = match part {
                PanelPart::NewShell => self.icon("+", "\u{ea60}"),
                _ => self.icon("×", "\u{ea81}"),
            };
            buf.set_string(rect.x + 1, rect.y, glyph, theme.get("ui.terminal.button"));
        }
        let right_end = buttons.first().map_or(area.right(), |(rect, _)| rect.x);
        let width = (right_end - area.x) as usize;

        let pair = |key: &Option<String>, what: &str| key.as_ref().map(|k| format!("{k} {what}"));
        let right = match terminal.exited() {
            Some(why) => Some((
                format!(" The shell {why}: press a key to close "),
                theme.get("ui.terminal.exited"),
            )),
            None if self.focused => {
                let hints: Vec<String> = [
                    pair(&self.keys.leave, "editor"),
                    pair(&self.keys.hide, "hide"),
                ]
                .into_iter()
                .flatten()
                .collect();
                Some((
                    format!(" {}  ", hints.join("   ")),
                    theme.get("ui.terminal.hint"),
                ))
            }
            None => pair(&self.keys.focus, "focus")
                .map(|hint| (format!(" {hint}  "), theme.get("ui.terminal.hint"))),
        };
        let right = right.filter(|(text, _)| text.width() + 12 <= width);
        let right_width = right.as_ref().map_or(0, |(text, _)| text.width());

        // In the bottom panel its tabs say what it is: no title of its own.
        if !self.tabbed {
            let left = format!(" TERMINAL  {}", terminal.title());
            let left = truncate_end(&left, width.saturating_sub(right_width + 1));
            buf.set_string(area.x, area.y, &left, title_style);
        }
        if let Some((text, style)) = right {
            buf.set_string(right_end - right_width as u16, area.y, text, style);
        }
    }

    /// The list of shells, with the line to its left.
    fn list(&self, list: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let line = theme.get("ui.terminal.border");
        let edge = list_edge(list);
        for y in edge.y..edge.bottom() {
            buf[(edge.x, y)].set_symbol("│").set_style(line);
        }
        let first = list_first(self.shown, list);
        let icon = self.icon("$", "\u{ea85}");
        for (i, shell) in self.shells.iter().enumerate().skip(first) {
            let y = list.y + (i - first) as u16;
            if y >= list.bottom() {
                break;
            }
            let row = Rect::new(list.x, y, list.width, 1);
            let style = if i == self.shown {
                theme.get("ui.terminal.list.shown")
            } else {
                theme.get("ui.terminal.list")
            };
            buf.set_style(row, style);
            let text = format!(" {icon} {}", shell.title());
            let text = truncate_end(&text, list.width as usize - 1);
            buf.set_stringn(list.x, y, &text, list.width as usize - 1, style);
        }
    }
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(terminal) = self.shells.get(self.shown) else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            return;
        }
        let base = self.theme.get("ui.terminal");
        buf.set_style(area, base);
        self.title_row(terminal, area, buf);
        let list = list_area(area, self.shells.len(), self.list_width);
        if let Some(list) = list {
            self.list(list, buf);
        }

        let body = body(area, list);
        terminal.note_size(body.height, body.width);
        let screen = terminal.screen();
        let (rows, cols) = screen.size();
        for row in 0..body.height.min(rows.get()) {
            for col in 0..body.width.min(cols.get()) {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = base;
                if let Some(fg) = color(cell.fgcolor()) {
                    style = style.fg(fg);
                }
                if let Some(bg) = color(cell.bgcolor()) {
                    style = style.bg(bg);
                }
                let mut modifiers = Modifier::empty();
                for (on, modifier) in [
                    (cell.bold(), Modifier::BOLD),
                    (cell.dim(), Modifier::DIM),
                    (cell.italic(), Modifier::ITALIC),
                    (cell.underline(), Modifier::UNDERLINED),
                    (cell.inverse(), Modifier::REVERSED),
                ] {
                    if on {
                        modifiers |= modifier;
                    }
                }
                let symbol = if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                };
                buf[(body.x + col, body.y + row)]
                    .set_symbol(symbol)
                    .set_style(style.add_modifier(modifiers));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn draws_the_title_and_the_shells_screen_in_colour() {
        let dir = tempfile::tempdir().unwrap();
        let mut terminal = Terminal::spawn(
            "/bin/sh",
            &["-c", "printf 'plain \\033[31mred\\033[0m\\n'; sleep 5"],
            dir.path(),
            4,
            30,
        )
        .unwrap();
        let start = Instant::now();
        while !terminal.screen().contents().contains("red")
            && start.elapsed() < Duration::from_secs(3)
        {
            terminal.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        let theme = crate::config::Config::default().theme;
        let keys = TerminalKeys {
            leave: Some("⌃\\ ⌃N".into()),
            hide: Some("⌃`".into()),
            focus: None,
        };
        let area = Rect::new(0, 0, 50, 5);
        let mut buf = Buffer::empty(area);
        let shells = std::slice::from_ref(&terminal);
        TerminalView::new(shells, 0, true, &keys, &theme).render(area, &mut buf);
        let row = |y: u16| -> String { (0..50).map(|x| buf[(x, y)].symbol()).collect() };
        assert!(row(0).starts_with(" TERMINAL  sh"), "{:?}", row(0));
        assert!(
            row(0)
                .trim_end()
                .ends_with("⌃\\ ⌃N editor   ⌃` hide   +  ×"),
            "{:?}",
            row(0)
        );
        assert!(
            row(1).starts_with(" plain red"),
            "a column in: {:?}",
            row(1)
        );
        assert_eq!(buf[(7, 1)].fg, Color::Indexed(1), "red");
        assert_eq!(
            buf[(1, 1)].fg,
            theme.get("ui.terminal").fg.unwrap_or(Color::Reset)
        );
        assert_eq!(
            TerminalView::cursor_position(&terminal, area, None),
            Some(Position::new(1, 2)),
            "under what was printed"
        );
    }
}
