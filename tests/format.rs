//! `gq` and `gw`: reflowing text, end to end.

mod common;

use std::fs;

use common::{app, app_with_config, cursor, keys, text};
use ferra::App;

fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\ntext-width = 20\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn gq_reflows_a_paragraph_to_the_width() {
    let mut app = with_text("the quick brown fox jumps<CR>over the lazy dog");
    keys(&mut app, "gqip");
    assert_eq!(text(&app), "the quick brown fox\njumps over the lazy\ndog");
    assert_eq!(cursor(&app), (2, 0), "on the last line");
    keys(&mut app, "u");
    assert_eq!(
        text(&app),
        "the quick brown fox jumps\nover the lazy dog",
        "one undo"
    );
}

#[test]
fn gqq_and_gqgq_do_the_current_line() {
    let mut app = with_text("aaa bbb ccc ddd eee fff ggg<CR>next");
    keys(&mut app, "gqq");
    assert_eq!(text(&app), "aaa bbb ccc ddd eee\nfff ggg\nnext");
    let mut app = with_text("aaa bbb ccc ddd eee fff ggg");
    keys(&mut app, "gqgq");
    assert_eq!(text(&app), "aaa bbb ccc ddd eee\nfff ggg");
}

#[test]
fn comments_keep_their_marker_and_indent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    fs::write(
        &path,
        "fn f() {\n    // a comment that is much too long for twenty columns\n}\n",
    )
    .unwrap();
    let mut app = app_with_config("[editor]\ntext-width = 30");
    app.open(&path);
    keys(&mut app, "jgqq");
    assert_eq!(
        text(&app),
        "fn f() {\n    // a comment that is much\n    // too long for twenty\n    // columns\n}"
    );
}

#[test]
fn gw_leaves_the_cursor_where_it_was() {
    let mut app = with_text("one two three four five six seven");
    keys(&mut app, "wwgwip");
    assert_eq!(text(&app), "one two three four\nfive six seven");
    assert_eq!(cursor(&app), (0, 8));
}

#[test]
fn visual_gq_and_dot() {
    let mut app = with_text("a b c d e f g h i j k l m n<CR><CR>n o p q r s t u v w x y z");
    keys(&mut app, "Vgq");
    assert_eq!(text(&app).lines().next(), Some("a b c d e f g h i j"));
    keys(&mut app, "Gzt0.");
    assert_eq!(
        text(&app).lines().last(),
        Some("x y z"),
        "the next paragraph, by ."
    );
}

#[test]
fn list_items_line_up_under_their_text() {
    let mut app = with_text("- first item that wraps around<CR>- second");
    keys(&mut app, "gqip");
    assert_eq!(text(&app), "- first item that\n  wraps around\n- second");
}

#[test]
fn text_width_can_be_set_per_buffer() {
    let mut app = app();
    keys(&mut app, ":setlocal tw=10<CR>ione two three four<Esc>gqq");
    assert_eq!(text(&app), "one two\nthree four");
}
