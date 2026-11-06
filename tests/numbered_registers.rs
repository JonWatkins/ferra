//! The numbered delete registers `"1`-`"9`, the small-delete register `"-`
//! and `"1p` followed by `.`, end to end.

mod common;

use common::{app_with_config, keys, text};
use terminal_editor::App;

fn register(app: &App, name: char) -> Option<String> {
    app.editor()
        .registers
        .get(Some(name))
        .map(|r| r.text.clone())
}

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn line_deletes_stack_up_in_the_numbered_registers() {
    let mut app = with_text("one<CR>two<CR>three<CR>end");
    keys(&mut app, "dddddd");
    assert_eq!(text(&app), "end");
    assert_eq!(register(&app, '1').as_deref(), Some("three"));
    assert_eq!(register(&app, '2').as_deref(), Some("two"));
    assert_eq!(register(&app, '3').as_deref(), Some("one"));
    keys(&mut app, "\"3p");
    assert_eq!(text(&app), "end\none");
}

#[test]
fn small_deletes_dont_push_out_line_deletes() {
    let mut app = with_text("keep me<CR>a few words");
    keys(&mut app, "ddxxdw");
    assert_eq!(register(&app, '-').as_deref(), Some("few "));
    assert_eq!(register(&app, '1').as_deref(), Some("keep me"));
    keys(&mut app, "\"1P");
    assert_eq!(text(&app), "keep me\nwords", "the line came back");
}

#[test]
fn the_yank_register_survives_deletes() {
    let mut app = with_text("copy<CR>junk");
    keys(&mut app, "yyjdd\"0p");
    assert_eq!(text(&app), "copy\ncopy");
}

#[test]
fn dot_after_a_numbered_paste_walks_back_through_deletes() {
    let mut app = with_text("one<CR>two<CR>three<CR>end");
    keys(&mut app, "dddddd");
    keys(&mut app, "\"1p");
    assert_eq!(text(&app), "end\nthree");
    keys(&mut app, "u.");
    assert_eq!(text(&app), "end\ntwo", "\"2 this time");
    keys(&mut app, "u.");
    assert_eq!(text(&app), "end\none", "then \"3");
}

#[test]
fn dot_repeating_other_commands_keeps_the_register() {
    let mut app = with_text("abc");
    keys(&mut app, "\"1x.");
    assert_eq!(text(&app), "c");
    assert_eq!(
        register(&app, '1').as_deref(),
        Some("b"),
        "x went to \"1 twice"
    );
    assert_eq!(
        register(&app, '2'),
        None,
        "writing \"1 directly doesn't shift"
    );
}

#[test]
fn a_bracket_jump_delete_is_numbered_even_on_one_line() {
    let mut app = with_text("f(a, b) rest");
    keys(&mut app, "ld%");
    assert_eq!(text(&app), "f rest");
    assert_eq!(register(&app, '1').as_deref(), Some("(a, b)"));
    assert_eq!(register(&app, '-'), None);
}

#[test]
fn deletes_across_lines_and_from_ex_and_visual_mode_are_numbered() {
    let mut app = with_text("ab<CR>cd<CR>ef<CR>gh");
    keys(&mut app, "lvjd");
    assert_eq!(register(&app, '1').as_deref(), Some("b\ncd"));
    keys(&mut app, ":2d<CR>");
    assert_eq!(register(&app, '1').as_deref(), Some("ef"));
    assert_eq!(register(&app, '2').as_deref(), Some("b\ncd"));
}

#[test]
fn named_and_black_hole_deletes_leave_the_numbers_alone() {
    let mut app = with_text("one<CR>two<CR>three");
    keys(&mut app, "dd\"add\"_dd");
    assert_eq!(register(&app, '1').as_deref(), Some("one"));
    assert_eq!(register(&app, '2'), None);
}
