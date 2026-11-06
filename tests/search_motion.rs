//! Searches as operator motions: `d/foo<CR>`, `c?bar<CR>`, `yn`, `dN`,
//! end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use terminal_editor::App;
use terminal_editor::editor::Mode;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

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
fn delete_up_to_a_search_match() {
    let mut app = with_text("one two three");
    keys(&mut app, "d/thr<CR>");
    assert_eq!(text(&app), "three");
    assert_eq!(cursor(&app), (0, 0));
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn backward_search_deletes_back_to_the_match() {
    let mut app = with_text("one two three");
    keys(&mut app, "$d?two<CR>");
    assert_eq!(text(&app), "one e", "exclusive: the cursor's char stays");
}

#[test]
fn yank_and_change_take_search_motions_too() {
    let mut app = with_text("ab cd ef");
    keys(&mut app, "y/cd<CR>");
    assert_eq!(register(&app, '0').as_deref(), Some("ab "));
    assert_eq!(cursor(&app), (0, 0), "a yank doesn't move the cursor");

    keys(&mut app, "c/ef<CR>new <Esc>");
    assert_eq!(text(&app), "new ef");
}

#[test]
fn a_count_picks_which_match() {
    let mut app = with_text("a x b x c");
    keys(&mut app, "d2/x<CR>");
    assert_eq!(text(&app), "x c");
}

#[test]
fn n_and_capital_n_after_an_operator() {
    let mut app = with_text("a x b x c x");
    keys(&mut app, "/x<CR>");
    assert_eq!(cursor(&app), (0, 2));
    keys(&mut app, "dn");
    assert_eq!(text(&app), "a x c x");
    keys(&mut app, "$dN");
    assert_eq!(text(&app), "a x", "back from the last x to the one before");
}

#[test]
fn a_search_that_crosses_lines_from_the_line_start_takes_whole_lines() {
    let mut app = with_text("one<CR>two<CR>three");
    keys(&mut app, "d/thr<CR>");
    assert_eq!(text(&app), "three");
    assert_eq!(register(&app, '1').as_deref(), Some("one\ntwo"));
}

#[test]
fn a_failed_search_cancels_the_operator() {
    let mut app = with_text("one<CR>two");
    keys(&mut app, "d/zzz<CR>");
    assert_eq!(message(&app), "Pattern not found: zzz");
    assert_eq!(text(&app), "one\ntwo");
    keys(&mut app, "j");
    assert_eq!(cursor(&app), (1, 0), "j moves: no operator left waiting");
}

#[test]
fn escape_cancels_the_search_and_the_operator() {
    let mut app = with_text("one<CR>two");
    keys(&mut app, "d/tw<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(cursor(&app), (0, 0), "back where the search began");
    keys(&mut app, "j");
    assert_eq!(text(&app), "one\ntwo");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn dot_repeats_a_search_delete() {
    let mut app = with_text("a;b;c;d");
    keys(&mut app, "d/;<CR>");
    assert_eq!(text(&app), ";b;c;d");
    keys(&mut app, "l.");
    assert_eq!(text(&app), ";;c;d");
}

#[test]
fn within_one_line_the_delete_is_numbered() {
    let mut app = with_text("keep (this) gone");
    keys(&mut app, "d/gone<CR>");
    assert_eq!(register(&app, '1').as_deref(), Some("keep (this) "));
    assert_eq!(register(&app, '-'), None);
}

#[test]
fn the_search_becomes_the_last_search() {
    let mut app = with_text("x two y two");
    keys(&mut app, "d/two<CR>n");
    assert_eq!(text(&app), "two y two");
    assert_eq!(cursor(&app), (0, 6), "n finds the next two");
}
