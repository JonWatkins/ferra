//! The status bar as shipped: one bar along the bottom of the screen, the
//! full width (under the activity bar and the sidebar too), worded as VS
//! Code's.

mod common;

use std::fs;

use common::{app_as_shipped, keys};
use ferra::App;
use ratatui::{Terminal, backend::TestBackend, style::Color};

const W: u16 = 100;
const H: u16 = 12;
/// The bar's colour in the default theme (`ui.statusbar`).
const BAR: Color = Color::Rgb(0x3d, 0x59, 0xa1);

/// The status bar's row: its text, and each cell's background.
fn bar(app: &mut App) -> (String, Vec<Color>) {
    let mut terminal = Terminal::new(TestBackend::new(W, H)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let y = H - 2;
    let text = (0..W).map(|x| buffer[(x, y)].symbol()).collect();
    let bgs = (0..W).map(|x| buffer[(x, y)].bg).collect();
    (text, bgs)
}

fn open(app: &mut App, name: &str, text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    app.open(&path);
    dir
}

#[test]
fn one_bar_along_the_bottom_the_full_width() {
    let mut app = app_as_shipped();
    keys(&mut app, "<C-b><C-S-e>");
    let (text, bgs) = bar(&mut app);
    // It starts at the left edge (under the activity bar and sidebar),
    // with the sidebar's mode (VS Code's keys show none while typing).
    assert!(text.starts_with(" SIDEBAR "), "{text:?}");
    // Past the mode, the bar's colour all the way across.
    assert!(bgs[10..].iter().all(|&bg| bg == BAR), "{bgs:?}");
    keys(&mut app, "<Esc>");
    let (text, bgs) = bar(&mut app);
    assert!(!text.contains("INSERT"), "{text:?}");
    assert!(bgs.iter().all(|&bg| bg == BAR), "{bgs:?}");
}

#[test]
fn worded_as_vs_codes() {
    let mut app = app_as_shipped();
    let _dir = open(&mut app, "main.rs", "fn main() {\n    let x = 1;\n}\n");
    keys(&mut app, "<Down><End><Home>");
    let (text, _) = bar(&mut app);
    for part in [
        "main.rs",
        "⊗ 0  ⚠ 0",
        "Ln 2, Col 5",
        "Spaces: 4",
        "UTF-8",
        " LF ",
        " Rust ",
    ] {
        assert!(text.contains(part), "{part:?} in {text:?}");
    }
    keys(&mut app, "x");
    assert!(bar(&mut app).0.contains("main.rs ●"), "unsaved");
}

#[test]
fn languages_by_their_names() {
    for (file, name) in [
        ("a.ts", " TypeScript "),
        ("a.cpp", " C++ "),
        ("a.json", " JSON "),
        ("a.sh", " Shell Script "),
        ("notes.txt", " Plain Text "),
    ] {
        let mut app = app_as_shipped();
        let _dir = open(&mut app, file, "x\n");
        let (text, _) = bar(&mut app);
        assert!(text.contains(name), "{file}: {text:?}");
    }
}

#[test]
fn tabs_show_their_size() {
    let mut app = app_as_shipped();
    let _dir = open(
        &mut app,
        "a.go",
        "package main\n\nfunc main() {\n\tx()\n}\n",
    );
    let (text, _) = bar(&mut app);
    assert!(text.contains("Tab Size: 4"), "{text:?}");
}

#[test]
fn nerd_icons_and_the_classic_style() {
    let mut app = common::app_with_config(
        "[editor]\nstatusline = \"global\"\nstatusline-style = \"vscode\"\nicons = \"nerd\"\n\
         [editor.statusline-layout]\nleft = [\"mode\", \"diagnostics\"]\nright = []\n",
    );
    let (text, _) = bar(&mut app);
    assert!(text.contains("\u{ea87} 0  \u{ea6c} 0"), "{text:?}");

    let mut classic = common::app();
    let screen = common::render(&mut classic, W, H);
    let status = &screen.rows[(H - 2) as usize];
    assert!(status.trim_end().ends_with(" 1:1"), "{status:?}");
    assert!(!status.contains("Ln "), "{status:?}");
}

/// Clicks the status bar where `text` is.
fn click_bar(app: &mut App, text: &str) {
    let (row, _) = bar(app);
    let byte = row
        .find(text)
        .unwrap_or_else(|| panic!("{text:?} not in {row:?}"));
    let x = row[..byte].chars().count() as u16 + 1;
    for kind in [
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
    ] {
        app.handle_mouse(crossterm::event::MouseEvent {
            kind,
            column: x,
            row: H - 2,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
    }
}

#[test]
fn clicking_the_position_asks_for_a_line() {
    let mut app = app_as_shipped();
    let _dir = open(&mut app, "notes.txt", "one\ntwo\nthree\n");
    click_bar(&mut app, "Ln 1");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Input);
    keys(&mut app, "3:2<CR>");
    assert_eq!(common::cursor(&app), (2, 1));
}

#[test]
fn clicking_the_indentation_line_endings_and_encoding_offers_others() {
    let mut app = app_as_shipped();
    let _dir = open(&mut app, "notes.txt", "one\n");
    click_bar(&mut app, "Spaces: 4");
    assert_eq!(app.editor().popup().unwrap().name, "menu");
    keys(&mut app, "<CR>");
    assert!(bar(&mut app).0.contains("Spaces: 2"), "{}", bar(&mut app).0);

    click_bar(&mut app, " LF ");
    keys(&mut app, "<Down><CR>");
    assert!(bar(&mut app).0.contains("CRLF"), "{}", bar(&mut app).0);
    assert!(app.editor().document.is_modified(), "to be saved with them");

    click_bar(&mut app, "UTF-8");
    assert_eq!(app.editor().popup().unwrap().name, "menu");
    keys(&mut app, "<Esc>");
}

#[test]
fn clicking_the_language_picks_another() {
    let mut app = app_as_shipped();
    let _dir = open(&mut app, "notes.txt", "x = 1\n");
    assert!(bar(&mut app).0.contains("Plain Text"));
    click_bar(&mut app, "Plain Text");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Picker);
    keys(&mut app, "toml<CR>");
    assert!(bar(&mut app).0.contains("TOML"), "{}", bar(&mut app).0);
    // And back.
    keys(&mut app, "<C-g>language auto<CR>");
    assert!(bar(&mut app).0.contains("Plain Text"));
}

#[test]
fn clicking_the_problems_lists_them() {
    let mut app = app_as_shipped();
    let _dir = open(&mut app, "notes.txt", "x\n");
    click_bar(&mut app, "⊗ 0");
    // None: it says so.
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(message.to_lowercase().contains("no "), "{message}");
}
