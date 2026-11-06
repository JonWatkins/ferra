//! `:g/pattern/command` and `:v`, end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use terminal_editor::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn delete_every_matching_line() {
    let mut app = with_text("keep<CR>TODO a<CR>keep 2<CR>TODO b<CR>TODO c");
    keys(&mut app, ":g/TODO/d<CR>");
    assert_eq!(text(&app), "keep\nkeep 2");
    assert_eq!(message(&app), "3 fewer lines");
}

#[test]
fn adjacent_matches_are_all_deleted() {
    // Deleting a line moves the next one up into its row: tracking rows
    // (not just counting them) is what makes this work.
    let mut app = with_text("x1<CR>x2<CR>x3<CR>y");
    keys(&mut app, ":g/x/d<CR>");
    assert_eq!(text(&app), "y");
}

#[test]
fn v_and_g_bang_take_the_lines_that_dont_match() {
    let mut app = with_text("a1<CR>b<CR>a2<CR>c");
    keys(&mut app, ":v/a/d<CR>");
    assert_eq!(text(&app), "a1\na2");

    let mut app = with_text("a1<CR>b<CR>a2<CR>c");
    keys(&mut app, ":g!/a/d<CR>");
    assert_eq!(text(&app), "a1\na2");
}

#[test]
fn substitute_on_matching_lines_only() {
    let mut app = with_text("let x = 1;<CR>// x here<CR>let y = x;");
    keys(&mut app, ":g/^let/s/x/z/g<CR>");
    assert_eq!(text(&app), "let z = 1;\n// x here\nlet y = z;");
}

#[test]
fn a_range_limits_the_lines() {
    let mut app = with_text("x<CR>x<CR>x<CR>x");
    keys(&mut app, ":2,3g/x/d<CR>");
    assert_eq!(text(&app), "x\nx");
}

#[test]
fn the_whole_run_is_one_undo_step() {
    let mut app = with_text("x<CR>y<CR>x<CR>y");
    keys(&mut app, ":g/x/d<CR>");
    assert_eq!(text(&app), "y\ny");
    keys(&mut app, "u");
    assert_eq!(text(&app), "x\ny\nx\ny");
}

#[test]
fn no_command_counts_and_goes_to_the_last_match() {
    let mut app = with_text("x<CR>y<CR>  x<CR>y");
    keys(&mut app, ":g/x<CR>");
    assert_eq!(message(&app), "2 matching lines");
    assert_eq!(cursor(&app), (2, 2));
}

#[test]
fn no_match_and_nesting_are_errors() {
    let mut app = with_text("abc");
    keys(&mut app, ":g/zzz/d<CR>");
    assert_eq!(message(&app), "Pattern not found: zzz");

    keys(&mut app, ":g/a/g/b/d<CR>");
    assert_eq!(message(&app), "Cannot use :g inside :g");
    assert_eq!(text(&app), "abc");
}

#[test]
fn an_empty_pattern_uses_the_last_search() {
    let mut app = with_text("foo<CR>bar<CR>foo");
    keys(&mut app, "/foo<CR>:g//d<CR>");
    assert_eq!(text(&app), "bar");
}

#[test]
fn the_pattern_becomes_the_last_search() {
    let mut app = with_text("a<CR>b<CR>a");
    keys(&mut app, ":g/b<CR>ggn");
    assert_eq!(cursor(&app), (1, 0));
}
