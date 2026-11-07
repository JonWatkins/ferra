//! The splash screen shown when ferra starts without a file: the logo, the
//! version, and a few ways to get going, centred in the editor pane.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use crate::config::Theme;

/// "ferra", in block letters (figlet's "ANSI Shadow" style).
pub const LOGO: [&str; 6] = [
    "███████╗███████╗██████╗ ██████╗  █████╗",
    "██╔════╝██╔════╝██╔══██╗██╔══██╗██╔══██╗",
    "█████╗  █████╗  ██████╔╝██████╔╝███████║",
    "██╔══╝  ██╔══╝  ██╔══██╗██╔══██╗██╔══██║",
    "██║     ███████╗██║  ██║██║  ██║██║  ██║",
    "╚═╝     ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝",
];

/// One line of the menu: the keys to press and what they do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplashEntry {
    pub keys: String,
    pub description: &'static str,
}

pub struct Splash<'a> {
    entries: &'a [SplashEntry],
    theme: &'a Theme,
}

impl<'a> Splash<'a> {
    pub fn new(entries: &'a [SplashEntry], theme: &'a Theme) -> Self {
        Self { entries, theme }
    }
}

/// Space between the keys and their descriptions.
const GAP: usize = 4;

impl Widget for Splash<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let key_width = self
            .entries
            .iter()
            .map(|e| e.keys.width())
            .max()
            .unwrap_or(0);
        let menu: Vec<Line> = self
            .entries
            .iter()
            .map(|entry| {
                let padding = " ".repeat(key_width - entry.keys.width() + GAP);
                Line::from(vec![
                    Span::styled(entry.keys.clone(), self.theme.get("ui.splash.key")),
                    Span::raw(padding),
                    Span::styled(entry.description, self.theme.get("ui.splash.text")),
                ])
            })
            .collect();
        let version = Line::styled(
            format!("version {}", env!("CARGO_PKG_VERSION")),
            self.theme.get("ui.splash.version"),
        );

        // Logo, blank, version, blank, menu; without the logo if there
        // isn't room for it (either way), and nothing if the menu doesn't
        // fit either.
        let menu_height = menu.len() + 2;
        let with_logo = LOGO.len() + 1 + menu_height;
        let logo_width = LOGO.iter().map(|l| l.width()).max().unwrap_or(0);
        let show_logo = area.height as usize >= with_logo && area.width as usize >= logo_width;
        let height = if show_logo { with_logo } else { menu_height };
        let menu_width = menu.iter().map(Line::width).max().unwrap_or(0);
        if (area.height as usize) < height || (area.width as usize) < menu_width {
            return;
        }

        let mut y = area.y + ((area.height as usize - height) / 2) as u16;
        if show_logo {
            let style = self.theme.get("ui.splash.logo");
            for line in LOGO {
                draw_block_line(buf, area, y, logo_width, Line::styled(line, style));
                y += 1;
            }
            y += 1;
        }
        let version_width = version.width();
        draw_block_line(buf, area, y, version_width, version);
        y += 2;
        for line in menu {
            draw_block_line(buf, area, y, menu_width, line);
            y += 1;
        }
    }
}

/// Draws `line` on row `y`, left-aligned within a block `block_width` wide
/// that's centred in `area` (so the logo's lines, and the menu's columns,
/// stay aligned with each other).
fn draw_block_line(buf: &mut Buffer, area: Rect, y: u16, block_width: usize, line: Line) {
    let x = area.x + ((area.width as usize).saturating_sub(block_width) / 2) as u16;
    let width = (area.right() - x).min(block_width as u16);
    line.style(Style::default())
        .render(Rect::new(x, y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<SplashEntry> {
        vec![
            SplashEntry {
                keys: ":e <file>".into(),
                description: "open a file",
            },
            SplashEntry {
                keys: "<Space>f".into(),
                description: "find a file",
            },
        ]
    }

    fn render(width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        Splash::new(&entries(), &Theme::default()).render(area, &mut buf);
        (0..height)
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
    fn logo_version_and_menu_are_centred() {
        let rows = render(60, 20);
        let logo = rows.iter().position(|r| r.contains("███████╗")).unwrap();
        assert!(rows[logo + 5].contains("╚═╝     ╚══════╝"), "{rows:#?}");
        let version = rows.iter().position(|r| r.contains("version")).unwrap();
        assert_eq!(version, logo + 7, "a blank line after the logo");
        let menu = rows.iter().position(|r| r.contains(":e <file>")).unwrap();
        assert_eq!(menu, version + 2);
        // The keys line up in a column, and so do the descriptions.
        let first = &rows[menu];
        let second = &rows[menu + 1];
        assert_eq!(first.find(":e"), second.find("<Space>"));
        assert_eq!(first.find("open"), second.find("find"));
        // Roughly centred both ways.
        let top_gap = rows.iter().position(|r| !r.is_empty()).unwrap();
        let bottom_gap = rows.iter().rev().position(|r| !r.is_empty()).unwrap();
        assert!(
            top_gap.abs_diff(bottom_gap) <= 1,
            "{top_gap} vs {bottom_gap}"
        );
    }

    #[test]
    fn the_logos_lines_line_up() {
        let rows = render(60, 20);
        let logo = rows.iter().position(|r| r.contains("███████╗")).unwrap();
        let starts: Vec<usize> = (logo..logo + 6)
            .map(|y| rows[y].chars().take_while(|c| *c == ' ').count())
            .collect();
        assert!(starts.iter().all(|&x| x == starts[0]), "{starts:?}");
    }

    #[test]
    fn small_screens_drop_the_logo_then_everything() {
        let rows = render(60, 9);
        assert!(
            !rows.iter().any(|r| r.contains('█')),
            "too short for the logo"
        );
        let rows = render(30, 20);
        assert!(!rows.iter().any(|r| r.contains('█')), "too narrow for it");
        let top_gap = rows.iter().position(|r| !r.is_empty()).unwrap();
        let bottom_gap = rows.iter().rev().position(|r| !r.is_empty()).unwrap();
        assert!(top_gap.abs_diff(bottom_gap) <= 1, "and no gap left for it");
        let rows = render(40, 6);
        assert!(rows.iter().any(|r| r.contains("open a file")));
        assert!(render(10, 3).iter().all(String::is_empty), "nothing fits");
    }
}
