//! Macros (`q`, `@`) and marks (`m`, `'`, `` ` ``) end to end.

mod common;

use std::fs;

use common::{app, app_with_config, cursor, keys, render, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// An app containing `content` (typed without auto-indent), cursor at start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn record_and_play_a_macro() {
    let mut app = with_text("a<CR>b<CR>c<CR>d<CR>e");
    keys(&mut app, "qaA;<Esc>jq");
    assert_eq!(text(&app), "a;\nb\nc\nd\ne");
    keys(&mut app, "@a");
    assert_eq!(text(&app), "a;\nb;\nc\nd\ne");
    keys(&mut app, "@@");
    assert_eq!(text(&app), "a;\nb;\nc;\nd\ne", "@@ repeats the last macro");
    keys(&mut app, "2@a");
    assert_eq!(text(&app), "a;\nb;\nc;\nd;\ne;", "with a count");
}

#[test]
fn recording_shows_in_the_statusline() {
    let mut app = app();
    keys(&mut app, "qz");
    assert!(render(&mut app, 60, 4).rows[2].contains("recording @z"));
    keys(&mut app, "q");
    assert!(!render(&mut app, 60, 4).rows[2].contains("recording"));
}

#[test]
fn macros_are_register_text_that_can_be_edited() {
    let mut app = with_text("x");
    keys(&mut app, "qbA!<Esc>q");
    keys(&mut app, "o<Esc>\"bp");
    assert_eq!(
        text(&app),
        "x!\nA!<Esc>",
        "the macro pasted as key notation"
    );

    // Edit it into a different macro and yank it back.
    keys(&mut app, "0f!r?0\"by$dd");
    keys(&mut app, "gg@b");
    assert_eq!(text(&app), "x!?");
}

#[test]
fn uppercase_appends_to_a_macro() {
    let mut app = with_text("one<CR>two");
    keys(&mut app, "qaA1<Esc>qqAjq");
    assert_eq!(
        app.editor().registers.get(Some('a')).unwrap().text,
        "A1<Esc>j"
    );
    // Recording `j` moved the cursor to `two`, so the macro runs there.
    keys(&mut app, "@a");
    assert_eq!(text(&app), "one1\ntwo1");
}

#[test]
fn a_failing_command_stops_the_macro() {
    let mut app = with_text("abc");
    // Record: an unknown command, then `x`.
    keys(&mut app, "qe:nope<CR>xq");
    assert_eq!(text(&app), "bc");
    keys(&mut app, "@e");
    assert_eq!(text(&app), "bc", "`x` didn't run after the error");
    assert_eq!(message(&app), "Unknown command: nope");
}

#[test]
fn a_macro_calling_itself_is_stopped() {
    let mut app = with_text(&"y".repeat(150));
    // `x`, then play this same macro: recursion until the depth limit.
    keys(&mut app, "qrx@rq");
    keys(&mut app, "@r");
    assert_eq!(message(&app), "Macro recursion too deep");
    let left = text(&app).len();
    assert!(
        left > 0 && left < 150,
        "deleted some, then stopped: {left} left"
    );
}

#[test]
fn at_colon_repeats_the_last_command_line() {
    let mut app = with_text("a a<CR>a a");
    keys(&mut app, ":s/a/b/g<CR>j@:");
    assert_eq!(text(&app), "b b\nb b");
}

#[test]
fn empty_and_invalid_registers() {
    let mut app = app();
    keys(&mut app, "@k");
    assert_eq!(message(&app), "Register k is empty");
    keys(&mut app, "@@");
    assert!(
        message(&app).contains("Register k is empty")
            || message(&app).contains("No previous macro")
    );
    keys(&mut app, "q%");
    assert!(
        message(&app).contains("Invalid register"),
        "{}",
        message(&app)
    );
}

#[test]
fn marks_jump_to_lines_and_positions() {
    let mut app = with_text("first<CR>  second line<CR>third");
    keys(&mut app, "jwwma");
    keys(&mut app, "G'a");
    assert_eq!(cursor(&app), (1, 2), "line: first non-blank");
    keys(&mut app, "G`a");
    assert_eq!(cursor(&app), (1, 9), "exact position");
}

#[test]
fn marks_work_with_operators_and_visual_mode() {
    let mut app = with_text("1<CR>2<CR>3<CR>4<CR>5");
    keys(&mut app, "jjmaggd'a");
    assert_eq!(text(&app), "4\n5");

    let mut app = with_text("keep (drop this) keep");
    keys(&mut app, "f(mbf)v`bd");
    assert_eq!(text(&app), "keep  keep");
}

#[test]
fn marks_move_with_edits_above_them() {
    let mut app = with_text("a<CR>b<CR>target");
    keys(&mut app, "Gmt");
    keys(&mut app, "ggOnew 1<CR>new 2<Esc>");
    keys(&mut app, "gg't");
    assert_eq!(cursor(&app), (4, 0));
    assert_eq!(text(&app).lines().nth(4), Some("target"));
}

#[test]
fn global_marks_jump_between_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "alpha\nbeta\n").unwrap();
    fs::write(&b, "other\n").unwrap();
    let mut app = app();
    app.open_all(&[a, b]);

    keys(&mut app, "jmA]b");
    assert_eq!(text(&app), "other");
    keys(&mut app, "'A");
    assert_eq!(text(&app), "alpha\nbeta", "switched back to a.txt");
    assert_eq!(cursor(&app), (1, 0));

    keys(&mut app, "]bd'A");
    assert_eq!(message(&app), "Mark is in another buffer");
    assert_eq!(text(&app), "other", "nothing deleted");
}

#[test]
fn unset_marks_are_errors() {
    let mut app = app();
    keys(&mut app, "'x");
    assert_eq!(message(&app), "Mark not set: x");
    keys(&mut app, "m!");
    assert!(message(&app).contains("Invalid mark"));
}
