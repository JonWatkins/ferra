//! Shared helpers for integration tests.
#![allow(dead_code)] // Each test binary uses a different subset.

use ferra::{App, Config, input::parse_keys};
use ratatui::{Terminal, backend::TestBackend};

/// The screen most tests check positions on: the editor without the
/// activity bar down the left, with the classic status line under each
/// window (their own tests check the new chrome). Tests of what's in the
/// windows needn't move with the chrome around them.
fn with_classic_chrome(toml: &str) -> String {
    let mut table: toml::Table = toml.parse().expect("test config should be valid TOML");
    let editor = table
        .entry("editor")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .expect("[editor] is a table");
    for key in ["activity-bar", "menu-bar"] {
        editor.entry(key).or_insert(toml::Value::Boolean(false));
    }
    // The classic status line: one per window, worded tersely, laid out as
    // it was.
    for (key, value) in [("statusline", "windows"), ("statusline-style", "classic")] {
        editor
            .entry(key)
            .or_insert(toml::Value::String(value.into()));
    }
    if !editor.contains_key("statusline-layout") {
        let layout: toml::Table = r#"
            left = ["mode", "file-name", "modified"]
            center = []
            right = ["recording", "pending-keys", "lsp-progress", "diagnostics", "git", "file-encoding", "file-type", "position"]
        "#
        .parse()
        .expect("valid");
        editor.insert("statusline-layout".into(), toml::Value::Table(layout));
    }
    toml::to_string(&table).expect("serializes")
}

/// An app with the default config, but the classic chrome (see
/// `with_classic_chrome`).
pub fn app() -> App {
    app_with_config("")
}

/// An app with `toml` merged over the default config (and the classic
/// chrome, unless `toml` says otherwise).
pub fn app_with_config(toml: &str) -> App {
    App::new(Config::from_toml(&with_classic_chrome(toml)).expect("test config should be valid"))
}

/// An app with the default config exactly, chrome and all.
pub fn app_as_shipped() -> App {
    App::default()
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
