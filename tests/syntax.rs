//! Syntax highlighting end-to-end: real files, the default theme, and live
//! updates while typing.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{app, app_with_config, keys};
use ratatui::{Terminal, backend::TestBackend, style::Color};
use terminal_editor::App;

const KEYWORD: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
const STRING: Color = Color::Rgb(0x9e, 0xce, 0x6a);

fn file(dir: &tempfile::TempDir, name: &str, contents: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, contents).unwrap();
    path
}

/// Screen columns on text row `y` drawn in `color` (the gutter is 5 wide,
/// so text column 0 is screen column 5).
fn cells_in(app: &mut App, y: u16, color: Color) -> Vec<u16> {
    let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..40).filter(|&x| buffer[(x, y)].fg == color).collect()
}

fn status_row(app: &mut App) -> String {
    common::render(app, 60, 6).rows[4].clone()
}

#[test]
fn rust_files_are_highlighted_with_the_default_theme() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&file(
        &dir,
        "main.rs",
        "fn main() {\n    let s = \"hi\";\n}\n",
    ));

    assert_eq!(cells_in(&mut app, 0, KEYWORD), vec![5, 6], "`fn`");
    assert_eq!(cells_in(&mut app, 1, KEYWORD), vec![9, 10, 11], "`let`");
    assert_eq!(
        cells_in(&mut app, 1, STRING),
        vec![17, 18, 19, 20],
        "\"hi\""
    );
    assert!(
        status_row(&mut app).contains(" rust "),
        "language in statusline"
    );
}

#[test]
fn highlighting_follows_edits_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&file(&dir, "new.rs", ""));
    assert!(cells_in(&mut app, 0, KEYWORD).is_empty());

    keys(&mut app, "ifn x() {}<Esc>");
    assert_eq!(cells_in(&mut app, 0, KEYWORD), vec![5, 6]);

    keys(&mut app, "u");
    assert!(
        cells_in(&mut app, 0, KEYWORD).is_empty(),
        "undo clears it again"
    );
}

#[test]
fn other_languages_are_detected() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&file(&dir, "Cargo.toml", "name = \"demo\"\n"));
    assert!(!cells_in(&mut app, 0, STRING).is_empty());
    assert!(status_row(&mut app).contains(" toml "));
}

#[test]
fn unknown_files_are_plain() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&file(&dir, "notes.txt", "fn main() {}\n"));
    assert!(cells_in(&mut app, 0, KEYWORD).is_empty());
}

#[test]
fn each_buffer_has_its_own_highlights() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open_all(&[
        file(&dir, "a.rs", "fn a() {}\n"),
        file(&dir, "b.txt", "fn b() {}\n"),
    ]);
    assert_eq!(cells_in(&mut app, 0, KEYWORD), vec![5, 6]);
    keys(&mut app, "]b");
    assert!(cells_in(&mut app, 0, KEYWORD).is_empty());
    keys(&mut app, "]b");
    assert_eq!(cells_in(&mut app, 0, KEYWORD), vec![5, 6]);
}

#[test]
fn can_be_turned_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config("[editor]\nsyntax-highlighting = false");
    app.open(&file(&dir, "main.rs", "fn main() {}\n"));
    assert!(cells_in(&mut app, 0, KEYWORD).is_empty());
}

#[test]
fn themes_can_style_any_scope() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config("[theme]\n\"keyword\" = \"red\"");
    app.open(&file(&dir, "main.rs", "fn main() {}\n"));
    assert_eq!(cells_in(&mut app, 0, Color::Red), vec![5, 6]);
}

#[test]
fn selection_and_search_draw_over_syntax_colours() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&file(&dir, "main.rs", "fn main() {}\n"));
    keys(&mut app, "vl");
    let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let cell = &terminal.backend().buffer()[(5, 0)];
    assert_eq!(cell.fg, KEYWORD, "keeps its colour");
    assert_eq!(
        cell.bg,
        Color::Rgb(0x33, 0x46, 0x7c),
        "and shows the selection"
    );
}

#[test]
fn a_script_without_an_extension_is_recognised_by_its_shebang() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&file(
        &dir,
        "deploy",
        "#!/usr/bin/env bash\nif true; then echo hi; fi\n",
    ));
    assert!(
        status_row(&mut app).contains(" bash "),
        "{}",
        status_row(&mut app)
    );
    assert!(
        !cells_in(&mut app, 1, KEYWORD).is_empty(),
        "`if` is a keyword"
    );
}

#[test]
fn typing_a_shebang_into_a_new_file_picks_the_language() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.open(&dir.path().join("new-script"));
    assert!(!status_row(&mut app).contains(" bash "));
    keys(&mut app, "i#!/bin/sh<Esc>");
    assert!(status_row(&mut app).contains(" bash "));
}

#[test]
fn config_can_add_file_names_and_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with_config("[languages.python]\nextensions = [\"py\", \"star\"]");
    app.open(&file(&dir, "BUILD.star", "def f():\n    return 1\n"));
    assert!(
        status_row(&mut app).contains(" python "),
        "{}",
        status_row(&mut app)
    );
}

#[test]
fn new_languages_are_highlighted() {
    let dir = tempfile::tempdir().unwrap();
    for (name, text, language) in [
        ("main.go", "func main() {}\n", "go"),
        ("app.ts", "function f(): void {}\n", "typescript"),
        ("lib.c", "int f(void) { return 0; }\n", "c"),
    ] {
        let mut app = app();
        app.open(&file(&dir, name, text));
        assert!(
            status_row(&mut app).contains(&format!(" {language} ")),
            "{name}"
        );
        assert!(
            !cells_in(&mut app, 0, KEYWORD).is_empty(),
            "{name} has keywords coloured"
        );
    }
}
