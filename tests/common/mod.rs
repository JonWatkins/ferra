//! Shared helpers for integration tests.
#![allow(dead_code)] // Each test binary uses a different subset.

use ratatui::{Terminal, backend::TestBackend};
use terminal_editor::{App, Config, input::parse_keys};

/// An app with the default config.
pub fn app() -> App {
    App::default()
}

/// An app with `toml` merged over the default config.
pub fn app_with_config(toml: &str) -> App {
    App::new(Config::from_toml(toml).expect("test config should be valid"))
}

/// Feeds keys to the app using Vim notation, e.g. `"ihello<Esc>"`.
pub fn keys(app: &mut App, notation: &str) {
    for key in parse_keys(notation).expect("valid key notation") {
        app.handle_key(key);
    }
}

pub fn text(app: &App) -> String {
    app.editor().document.buffer().text()
}

/// `(row, col)` of the cursor, 0-based.
pub fn cursor(app: &App) -> (usize, usize) {
    let c = app.editor().document.cursor();
    (c.row, c.col)
}

/// A rendered frame: each screen row as a string, plus the cursor position.
pub struct Screen {
    pub rows: Vec<String>,
    pub cursor: (u16, u16),
}

pub fn render(app: &mut App, width: u16, height: u16) -> Screen {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let buffer = terminal.backend().buffer();
    let rows = (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let cursor = terminal.get_cursor_position().unwrap();
    Screen {
        rows,
        cursor: (cursor.x, cursor.y),
    }
}
