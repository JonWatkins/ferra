//! Options set per language in `[languages.<name>]`, end to end.

mod common;

use std::fs;

use common::{app, app_with_config, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn open_in(dir: &tempfile::TempDir, name: &str, content: &str, config: &str) -> App {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    let mut app = app_with_config(config);
    app.open(&path);
    app
}

#[test]
fn makefiles_and_go_indent_with_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open_in(&dir, "Makefile", "all:\necho hi\n", "");
    keys(&mut app, "j>>");
    assert_eq!(text(&app), "all:\n\techo hi");
    let mut app = open_in(&dir, "main.go", "x\n", "");
    keys(&mut app, ":set et?<CR>");
    assert_eq!(message(&app), "expand-tab=false");
}

#[test]
fn commit_messages_reflow_at_72() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open_in(&dir, "COMMIT_EDITMSG", "x\n", "");
    keys(&mut app, ":set tw?<CR>");
    assert_eq!(message(&app), "text-width=72");
}

#[test]
fn a_users_own_language_options_apply() {
    let dir = tempfile::tempdir().unwrap();
    let config = "[languages.python]\noptions = { tab-width = 2, text-width = 60 }";
    let mut app = open_in(&dir, "a.py", "def f():\nx\n", config);
    keys(&mut app, "j>>");
    assert_eq!(text(&app), "def f():\n  x");
    let mut other = open_in(&dir, "a.txt", "x\n", config);
    keys(&mut other, ":set ts?<CR>");
    assert_eq!(message(&other), "tab-width=4", "only python's buffers");
}

#[test]
fn setlocal_and_set_beat_the_language() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open_in(&dir, "main.go", "x\n", "");
    keys(&mut app, ":setlocal et<CR>>>");
    assert_eq!(text(&app), "    x", ":setlocal wins");
    let mut app = open_in(&dir, "main.go", "x\n", "");
    keys(&mut app, ":set et<CR>>>");
    assert_eq!(text(&app), "    x", ":set does too, here");
}

#[test]
fn naming_a_file_picks_up_its_languages_options() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    keys(
        &mut app,
        &format!(":w {}<CR>", dir.path().join("Makefile").display()),
    );
    keys(&mut app, ":set et?<CR>");
    assert_eq!(message(&app), "expand-tab=false");
}

#[test]
fn only_buffer_options_can_be_set_per_language() {
    let bad = ferra::Config::from_toml("[languages.go]\noptions = { wrap = false }");
    assert!(
        bad.unwrap_err()
            .to_string()
            .contains("languages.go.options: wrap can't be set per language"),
    );
    let bad = ferra::Config::from_toml("[languages.go]\noptions = { tab-width = \"wide\" }");
    assert!(
        bad.unwrap_err()
            .to_string()
            .contains("languages.go.options: tab-width")
    );
}
