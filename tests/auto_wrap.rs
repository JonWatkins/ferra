//! `auto-wrap`: breaking lines past `text-width` as they're typed.

mod common;

use std::fs;

use common::{app_with_config, keys, text};
use ferra::App;

fn wrapping(mode: &str, width: usize) -> App {
    app_with_config(&format!(
        "[editor]\nauto-wrap = \"{mode}\"\ntext-width = {width}"
    ))
}

#[test]
fn typing_past_the_width_breaks_at_the_last_space() {
    let mut app = wrapping("all", 20);
    keys(&mut app, "ithe quick brown fox jumps over the dog<Esc>");
    assert_eq!(text(&app), "the quick brown fox\njumps over the dog");
}

#[test]
fn comments_mode_wraps_comments_only_and_keeps_the_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    fs::write(&path, "fn f() {\n}\n").unwrap();
    let mut app = wrapping("comments", 24);
    app.open(&path);
    keys(&mut app, "o// a comment that runs past the width<Esc>");
    assert_eq!(
        text(&app),
        "fn f() {\n    // a comment that\n    // runs past the\n    // width\n}"
    );
    keys(&mut app, "olet x = some_function(argument, another);<Esc>");
    assert!(
        text(&app).contains("    let x = some_function(argument, another);"),
        "code isn't wrapped: {}",
        text(&app)
    );
}

#[test]
fn list_items_continue_under_their_text() {
    let mut app = wrapping("all", 15);
    keys(&mut app, "i- one two three four five<Esc>");
    assert_eq!(text(&app), "- one two three\n  four five");
}

#[test]
fn a_word_too_long_to_fit_is_left_alone() {
    let mut app = wrapping("all", 10);
    keys(&mut app, "isupercalifragilistic<Esc>");
    assert_eq!(text(&app), "supercalifragilistic");
}

#[test]
fn off_by_default() {
    let mut app = app_with_config("[editor]\ntext-width = 10");
    keys(&mut app, "ia b c d e f g h i j<Esc>");
    assert_eq!(text(&app), "a b c d e f g h i j");
}

#[test]
fn commit_messages_wrap_at_72() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("COMMIT_EDITMSG");
    fs::write(&path, "").unwrap();
    let mut app = app_with_config("");
    app.open(&path);
    let words = "word ".repeat(20);
    keys(&mut app, &format!("i{}<Esc>", words.trim_end()));
    let text = text(&app);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text:?}");
    assert!(lines[0].len() <= 72);
}

#[test]
fn one_undo_takes_back_the_insert() {
    let mut app = wrapping("all", 10);
    keys(&mut app, "ione two three four<Esc>u");
    assert_eq!(text(&app), "");
}
