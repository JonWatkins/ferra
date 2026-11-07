//! `:sort`, end to end.

mod common;

use common::{app, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn sorts_the_whole_file_as_one_change() {
    let mut app = with_text("cherry<CR>apple<CR>banana");
    keys(&mut app, "G:sort<CR>");
    assert_eq!(text(&app), "apple\nbanana\ncherry");
    assert_eq!(cursor(&app), (0, 0));
    keys(&mut app, "u");
    assert_eq!(text(&app), "cherry\napple\nbanana", "one undo");
}

#[test]
fn a_visual_selection_sorts_just_those_lines() {
    let mut app = with_text("z<CR>c<CR>b<CR>a");
    keys(&mut app, "jVj:sort<CR>");
    assert_eq!(text(&app), "z\nb\nc\na");
}

#[test]
fn numbers_reversed_and_unique() {
    let mut app = with_text("item 10<CR>item 9<CR>item 100");
    keys(&mut app, ":sort! n<CR>");
    assert_eq!(text(&app), "item 100\nitem 10\nitem 9");
    let mut app = with_text("b<CR>a<CR>b<CR>a<CR>c");
    keys(&mut app, ":sort u<CR>");
    assert_eq!(text(&app), "a\nb\nc");
    assert_eq!(message(&app), "2 fewer lines");
}

#[test]
fn by_what_follows_a_pattern_or_the_last_search() {
    let mut app = with_text("b = 2<CR>a = 3<CR>c = 1");
    keys(&mut app, ":sort /= /<CR>");
    assert_eq!(text(&app), "c = 1\nb = 2\na = 3");
    keys(&mut app, "u/= <CR>:sort! //<CR>");
    assert_eq!(text(&app), "a = 3\nb = 2\nc = 1");
}

#[test]
fn a_bad_flag_is_reported() {
    let mut app = with_text("b<CR>a");
    keys(&mut app, ":sort q<CR>");
    assert_eq!(message(&app), "Unknown :sort flag: q");
    assert_eq!(text(&app), "b\na");
}
