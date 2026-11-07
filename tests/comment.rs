//! `gc`: toggling comments by language, end to end.

mod common;

use std::fs;

use common::{app_with_config, cursor, keys};
use ferra::App;

/// The buffer's text as it would be saved: the files here end with a
/// line break, which the buffer keeps separately.
fn text(app: &App) -> String {
    common::text(app) + "\n"
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// A file called `name` holding `content`, open, cursor at the start.
fn open(name: &str, content: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    let mut app = app_with_config("[editor]\nauto-indent = false");
    app.open(&path);
    (dir, app)
}

#[test]
fn gcc_toggles_the_current_line() {
    let (_dir, mut app) = open("a.rs", "let x = 1;\nlet y = 2;\n");
    keys(&mut app, "gcc");
    assert_eq!(text(&app), "// let x = 1;\nlet y = 2;\n");
    keys(&mut app, "gcc");
    assert_eq!(text(&app), "let x = 1;\nlet y = 2;\n");
}

#[test]
fn counts_motions_and_text_objects() {
    let (_dir, mut app) = open("a.py", "a = 1\nb = 2\nc = 3\n\nd = 4\n");
    keys(&mut app, "2gcc");
    assert_eq!(text(&app), "# a = 1\n# b = 2\nc = 3\n\nd = 4\n");
    keys(&mut app, "u");
    keys(&mut app, "gcap");
    assert_eq!(
        text(&app),
        "# a = 1\n# b = 2\n# c = 3\n\nd = 4\n",
        "a paragraph; the blank line after it stays blank"
    );
    keys(&mut app, "Ggck");
    assert_eq!(
        text(&app),
        "# a = 1\n# b = 2\n# c = 3\n\n# d = 4\n",
        "blank lines are skipped"
    );
}

#[test]
fn mixed_lines_get_commented_at_the_shallowest_indent() {
    let (_dir, mut app) = open("a.rs", "fn f() {\n    // done\n    let x;\n}\n");
    keys(&mut app, "jgcj");
    assert_eq!(
        text(&app),
        "fn f() {\n    // // done\n    // let x;\n}\n",
        "not every line was commented, so all are"
    );
    keys(&mut app, "gcj");
    assert_eq!(
        text(&app),
        "fn f() {\n    // done\n    let x;\n}\n",
        "and back"
    );

    let (_dir, mut app) = open("a.rs", "if a {\n    b();\n}\n");
    keys(&mut app, "gcG");
    assert_eq!(text(&app), "// if a {\n//     b();\n// }\n");
}

#[test]
fn visual_mode_and_dot() {
    let (_dir, mut app) = open("a.sh", "echo 1\necho 2\necho 3\n");
    keys(&mut app, "Vjgc");
    assert_eq!(text(&app), "# echo 1\n# echo 2\necho 3\n");
    keys(&mut app, "jj.");
    assert_eq!(
        text(&app),
        "# echo 1\n# echo 2\n# echo 3\n",
        "`.` repeats gcc-style on one line"
    );
}

#[test]
fn block_comment_languages_wrap_each_line() {
    let (_dir, mut app) = open("a.html", "<p>one</p>\n<p>two</p>\n");
    keys(&mut app, "gcj");
    assert_eq!(text(&app), "<!-- <p>one</p> -->\n<!-- <p>two</p> -->\n");
    keys(&mut app, "gcj");
    assert_eq!(text(&app), "<p>one</p>\n<p>two</p>\n");

    let (_dir, mut app) = open("a.css", "a { color: red; }\n");
    keys(&mut app, "gcc");
    assert_eq!(text(&app), "/* a { color: red; } */\n");
}

#[test]
fn one_undo_step_and_the_cursor_on_the_first_line() {
    let (_dir, mut app) = open("a.go", "a()\n  b()\n");
    keys(&mut app, "jgck");
    assert_eq!(text(&app), "// a()\n//   b()\n");
    assert_eq!(cursor(&app), (0, 0));
    keys(&mut app, "u");
    assert_eq!(text(&app), "a()\n  b()\n");
}

#[test]
fn files_without_comments_say_so() {
    let (_dir, mut app) = open("a.json", "{}\n");
    keys(&mut app, "gcc");
    assert_eq!(message(&app), "Can't comment: json has no comments");
    assert_eq!(text(&app), "{}\n");

    let mut app = app_with_config("");
    keys(&mut app, "ix<Esc>gcc");
    assert_eq!(message(&app), "Can't comment: no language for this file");
}

#[test]
fn the_comment_token_is_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.sql");
    fs::write(&path, "select 1;\n").unwrap();
    let mut app = app_with_config("[languages.sql]\nextensions = [\"sql\"]\ncomment = \"--\"");
    app.open(&path);
    keys(&mut app, "gcc");
    assert_eq!(
        text(&app),
        "-- select 1;\n",
        "no grammar, but comments still work"
    );
}
