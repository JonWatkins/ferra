//! `gi`, `gf` and `<C-^>`: getting back to where you were.

mod common;

use std::fs;

use common::{app, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn file_name(app: &App) -> String {
    app.editor().document.name().to_string()
}

#[test]
fn gi_inserts_where_insert_mode_last_stopped() {
    let mut app = app();
    keys(&mut app, "ione two<CR>three<Esc>ggiX<Esc>Gl");
    keys(&mut app, "giY<Esc>");
    assert_eq!(text(&app), "XYone two\nthree");
    keys(&mut app, "G`^");
    assert_eq!(cursor(&app), (0, 2), "the ^ mark");
}

#[test]
fn gf_opens_the_file_under_the_cursor_at_its_line() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("b.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(dir.path().join("a.txt"), "see b.txt:3 and b.txt:2:2\n").unwrap();
    let mut app = app();
    app.open(&dir.path().join("a.txt"));
    keys(&mut app, "wgf");
    assert!(file_name(&app).ends_with("b.txt"), "{}", file_name(&app));
    assert_eq!(cursor(&app), (2, 0), "line 3, beside a.txt");
    keys(&mut app, "<C-^>");
    assert!(file_name(&app).ends_with("a.txt"));
    keys(&mut app, "$gf");
    assert_eq!(cursor(&app), (1, 1), "line 2, column 2");
}

#[test]
fn gf_says_when_theres_no_such_file() {
    let mut app = app();
    keys(&mut app, "inowhere.txt here<Esc>0gf");
    assert_eq!(message(&app), "Can't find file \"nowhere.txt\"");
    keys(&mut app, "A <Esc>gf");
    assert_eq!(message(&app), "No file name under the cursor");
}

#[test]
fn ctrl_caret_switches_to_the_alternate_buffer() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    let mut app = app();
    app.open(&dir.path().join("a.txt"));
    app.open(&dir.path().join("b.txt"));
    keys(&mut app, "<C-^>");
    assert!(file_name(&app).ends_with("a.txt"));
    keys(&mut app, "<C-6>");
    assert!(
        file_name(&app).ends_with("b.txt"),
        "Ctrl-6, as terminals send it"
    );
}
