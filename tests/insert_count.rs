//! Counts on the insert commands (`3ihi<Esc>`, `5o`), end to end.

mod common;

use common::{app, keys, text};
use ferra::App;

fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn i_with_a_count_types_the_text_that_many_times() {
    let mut app = with_text("x");
    keys(&mut app, "3ihi<Esc>");
    assert_eq!(text(&app), "hihihix");
    let mut app = common::app();
    keys(&mut app, "20i-<Esc>");
    assert_eq!(text(&app), "-".repeat(20));
}

#[test]
fn a_capital_a_and_i_take_counts() {
    let mut app = with_text("ab");
    keys(&mut app, "2a.<Esc>");
    assert_eq!(text(&app), "a..b");
    keys(&mut app, "3A!<Esc>");
    assert_eq!(text(&app), "a..b!!!");
    keys(&mut app, "2I><Esc>");
    assert_eq!(text(&app), ">>a..b!!!");
}

#[test]
fn o_and_capital_o_put_each_copy_on_a_line() {
    let mut app = with_text("top<CR>bottom");
    keys(&mut app, "3oitem<Esc>");
    assert_eq!(text(&app), "top\nitem\nitem\nitem\nbottom");
    keys(&mut app, "G2Oabove<Esc>");
    assert_eq!(text(&app), "top\nitem\nitem\nitem\nabove\nabove\nbottom");
}

#[test]
fn copies_keep_the_indentation() {
    let mut app = app();
    keys(&mut app, "i    x<Esc>2oy<Esc>");
    assert_eq!(text(&app), "    x\n    y\n    y");
}

#[test]
fn text_over_several_lines_repeats_whole() {
    let mut app = app();
    keys(&mut app, "2ia<CR>b <Esc>");
    assert_eq!(text(&app), "a\nb a\nb ");
}

#[test]
fn dot_repeats_with_the_count_and_undo_takes_it_all() {
    let mut app = with_text("");
    keys(&mut app, "3ix<Esc>");
    keys(&mut app, ".");
    assert_eq!(text(&app), "xxxxxx");
    keys(&mut app, "u");
    assert_eq!(text(&app), "xxx", "one undo for a counted insert");
}

#[test]
fn no_count_types_it_once() {
    let mut app = with_text("");
    keys(&mut app, "ihi<Esc>ahi<Esc>");
    assert_eq!(text(&app), "hihi");
}
