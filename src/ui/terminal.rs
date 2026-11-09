//! The terminal panel (`editor/terminal_panel.rs`), under the windows: a
//! title row, then the shell's screen.
//!
//! ```text
//!  TERMINAL  zsh                              ⌃\ ⌃N editor   ⌃` hide
//! ~/Code/ferra $ cargo test
//!    Compiling ferra v0.1.0
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

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
    terminal: &'a Terminal,
    focused: bool,
    keys: &'a TerminalKeys,
    theme: &'a Theme,
}

/// The shell's part of the panel's area: under the title row, a column
/// in from the left (so it doesn't run into the sidebar's line).
fn body(area: Rect) -> Rect {
    let margin = 1.min(area.width.saturating_sub(1));
    Rect::new(
        area.x + margin,
        area.y + 1.min(area.height),
        area.width - margin,
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
    pub fn new(
        terminal: &'a Terminal,
        focused: bool,
        keys: &'a TerminalKeys,
        theme: &'a Theme,
    ) -> Self {
        Self {
            terminal,
            focused,
            keys,
            theme,
        }
    }

    /// Where the terminal cursor goes: the shell's cursor, while the panel
    /// has focus (and isn't scrolled back, or hiding it).
    pub fn cursor_position(terminal: &Terminal, area: Rect) -> Option<Position> {
        let screen = terminal.screen();
        if screen.hide_cursor() || screen.scrollback() > 0 || !terminal.is_running() {
            return None;
        }
        let body = body(area);
        let (row, col) = screen.cursor_position();
        (row < body.height && col < body.width).then(|| Position::new(body.x + col, body.y + row))
    }

    fn title_row(&self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let title_style = if self.focused {
            theme.get("ui.terminal.title.focused")
        } else {
            theme.get("ui.terminal.title")
        };
        let row = Rect::new(area.x, area.y, area.width, 1);
        buf.set_style(row, theme.get("ui.terminal.title"));
        let width = area.width as usize;

        let pair = |key: &Option<String>, what: &str| key.as_ref().map(|k| format!("{k} {what}"));
        let right = match self.terminal.exited() {
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

        let left = format!(" TERMINAL  {}", self.terminal.title());
        let left = truncate_end(&left, width.saturating_sub(right_width + 1));
        buf.set_string(area.x, area.y, &left, title_style);
        if let Some((text, style)) = right {
            buf.set_string(area.right() - right_width as u16, area.y, text, style);
        }
    }
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let base = self.theme.get("ui.terminal");
        buf.set_style(area, base);
        self.title_row(area, buf);

        let body = body(area);
        self.terminal.note_size(body.height, body.width);
        let screen = self.terminal.screen();
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
            terminal.pump();
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
        TerminalView::new(&terminal, true, &keys, &theme).render(area, &mut buf);
        let row = |y: u16| -> String { (0..50).map(|x| buf[(x, y)].symbol()).collect() };
        assert!(row(0).starts_with(" TERMINAL  sh"), "{:?}", row(0));
        assert!(
            row(0).trim_end().ends_with("⌃\\ ⌃N editor   ⌃` hide"),
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
            TerminalView::cursor_position(&terminal, area),
            Some(Position::new(1, 2)),
            "under what was printed"
        );
    }
}
