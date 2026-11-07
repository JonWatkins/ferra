//! Indenting each file the way it's already indented, and `:indent`.

mod common;

use std::fs;

use common::{app_with_config, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn open_with(name: &str, content: &str, config: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    let mut app = app_with_config(config);
    app.open(&path);
    (dir, app)
}

fn open(name: &str, content: &str) -> (tempfile::TempDir, App) {
    open_with(name, content, "")
}

const TWO_SPACES: &str = "fn f() {\n  if x {\n    y();\n  }\n}";

#[test]
fn a_two_space_file_is_indented_by_two() {
    let (_dir, mut app) = open("a.rs", TWO_SPACES);
    keys(&mut app, "jjoz();<Esc>");
    assert_eq!(text(&app), "fn f() {\n  if x {\n    y();\n    z();\n  }\n}");

    keys(&mut app, "u>>");
    assert_eq!(text(&app), "fn f() {\n  if x {\n      y();\n  }\n}");

    keys(&mut app, "ugg=G");
    assert_eq!(text(&app), TWO_SPACES, "= keeps the file's width");

    keys(&mut app, "GO<Tab>a<Esc>");
    assert_eq!(
        text(&app).lines().nth(4),
        Some("    a"),
        "Tab types one level"
    );
}

#[test]
fn closing_brackets_and_enter_use_the_files_width() {
    let (_dir, mut app) = open("a.js", "if (a) {\n  b();\n}\n");
    keys(&mut app, "Gofunction g() {<CR>c();<CR>}<Esc>");
    assert_eq!(text(&app), "if (a) {\n  b();\n}\nfunction g() {\n  c();\n}");
}

#[test]
fn a_tab_indented_file_gets_tabs_even_with_expand_tab() {
    let (_dir, mut app) = open("a.go", "func f() {\n\tif x {\n\t\ty()\n\t}\n}");
    keys(&mut app, "jjoz()<Esc>");
    assert_eq!(text(&app), "func f() {\n\tif x {\n\t\ty()\n\t\tz()\n\t}\n}");
    keys(&mut app, "<<");
    assert_eq!(text(&app).lines().nth(3), Some("\tz()"));
}

#[test]
fn a_space_indented_file_gets_spaces_without_expand_tab() {
    let (_dir, mut app) = open_with("a.rs", TWO_SPACES, "[editor]\nexpand-tab = false");
    keys(&mut app, "j>>");
    assert_eq!(text(&app).lines().nth(1), Some("    if x {"));
}

#[test]
fn files_with_nothing_indented_use_the_config() {
    let (_dir, mut app) = open_with("a.rs", "fn f() {\n}", "[editor]\ntab-width = 3");
    keys(&mut app, "ox<Esc>");
    assert_eq!(text(&app), "fn f() {\n   x\n}");
}

#[test]
fn detection_can_be_turned_off() {
    let (_dir, mut app) = open_with("a.rs", TWO_SPACES, "[editor]\ndetect-indent = false");
    keys(&mut app, "jj>>");
    assert_eq!(text(&app).lines().nth(2), Some("        y();"));
    keys(&mut app, ":indent<CR>");
    assert_eq!(message(&app), "Indenting with 4 spaces (from config)");
}

#[test]
fn indent_shows_and_chooses_the_style() {
    let (_dir, mut app) = open("a.rs", TWO_SPACES);
    keys(&mut app, ":indent<CR>");
    assert_eq!(message(&app), "Indenting with 2 spaces (detected)");

    keys(&mut app, ":indent tabs<CR>");
    assert_eq!(message(&app), "Indenting with tabs");
    keys(&mut app, "gg=G");
    assert_eq!(text(&app), "fn f() {\n\tif x {\n\t\ty();\n\t}\n}");
    keys(&mut app, ":indent<CR>");
    assert_eq!(message(&app), "Indenting with tabs (set with :indent)");

    keys(&mut app, ":indent 3<CR>=G");
    assert_eq!(text(&app), "fn f() {\n   if x {\n      y();\n   }\n}");
    keys(&mut app, ":indent spaces<CR>=G");
    assert_eq!(text(&app), "fn f() {\n    if x {\n        y();\n    }\n}");
    keys(&mut app, ":indent spaces 2<CR>=G");
    assert_eq!(text(&app), TWO_SPACES);

    keys(&mut app, ":indent 0<CR>");
    assert_eq!(message(&app), "Not an indent width (1 to 16): 0");
    keys(&mut app, ":indent sideways<CR>");
    assert_eq!(
        message(&app),
        "Usage: :indent [tabs | spaces [N] | N | auto]"
    );
}

#[test]
fn indent_auto_looks_at_the_text_again() {
    let (_dir, mut app) = open("new.py", "");
    keys(&mut app, ":indent<CR>");
    assert_eq!(message(&app), "Indenting with 4 spaces (from config)");
    keys(&mut app, "idef f():<Esc>o<Esc>0Di  pass<Esc>");
    keys(&mut app, ":indent tabs<CR>:indent auto<CR>");
    assert_eq!(message(&app), "Indenting with 2 spaces");
    keys(&mut app, ":indent<CR>");
    assert_eq!(message(&app), "Indenting with 2 spaces (detected)");
}

#[test]
fn reloading_looks_again() {
    let (dir, mut app) = open("a.rs", TWO_SPACES);
    fs::write(dir.path().join("a.rs"), "fn f() {\n\tx();\n}").unwrap();
    keys(&mut app, ":e<CR>:indent<CR>");
    assert_eq!(message(&app), "Indenting with tabs (detected)");
}

#[test]
fn visual_block_shifts_by_the_files_width() {
    let (_dir, mut app) = open("a.rs", TWO_SPACES);
    keys(&mut app, "jj<C-v>j>");
    assert_eq!(text(&app), "fn f() {\n  if x {\n      y();\n    }\n}");
}
