//! Indent and case operators, joining lines, auto-indent and tabs, end to
//! end.

mod common;

use common::{app, app_with_config, cursor, keys, render, text};
use ferra::App;

/// An app containing `content` (typed without auto-indent), cursor at start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn indent_and_outdent_lines() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, ">>");
    assert_eq!(text(&app), "    a\nb\nc");
    keys(&mut app, "j>j");
    assert_eq!(text(&app), "    a\n    b\n    c");
    keys(&mut app, "<lt>G");
    assert_eq!(text(&app), "    a\nb\nc");
    keys(&mut app, "gg<lt><lt>");
    assert_eq!(text(&app), "a\nb\nc");
}

#[test]
fn indent_with_counts_text_objects_and_repeat() {
    let mut app = with_text("a<CR>b<CR><CR>c");
    keys(&mut app, "2>>");
    assert_eq!(text(&app), "    a\n    b\n\nc");
    keys(&mut app, ".");
    assert_eq!(text(&app), "        a\n        b\n\nc");
    keys(&mut app, "u");
    assert_eq!(text(&app), "    a\n    b\n\nc", "one undo step");
}

#[test]
fn visual_indent() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, "Vj>");
    assert_eq!(text(&app), "    a\n    b\nc");
    keys(&mut app, "gv<lt>");
    assert_eq!(text(&app), "a\nb\nc");
}

#[test]
fn indent_with_tabs_when_configured() {
    let mut app = app_with_config("[editor]\nexpand-tab = false\nauto-indent = false");
    keys(&mut app, "ix<Esc>>>");
    assert_eq!(text(&app), "\tx");
}

#[test]
fn case_operators() {
    let mut app = with_text("hello world");
    keys(&mut app, "gUiw");
    assert_eq!(text(&app), "HELLO world");
    keys(&mut app, "wg~e");
    assert_eq!(text(&app), "HELLO WORLD");
    keys(&mut app, "guu");
    assert_eq!(text(&app), "hello world");
    keys(&mut app, "gUU");
    assert_eq!(text(&app), "HELLO WORLD");
    keys(&mut app, "g~~");
    assert_eq!(text(&app), "hello world");
}

#[test]
fn tilde_and_visual_case() {
    let mut app = with_text("abc def");
    keys(&mut app, "~~");
    assert_eq!(text(&app), "ABc def");
    assert_eq!(cursor(&app), (0, 2));
    keys(&mut app, "wveU");
    assert_eq!(text(&app), "ABc DEF");
    keys(&mut app, "0vlu");
    assert_eq!(text(&app), "abc DEF");
}

#[test]
fn join_lines() {
    let mut app = with_text("let x =<CR>    1 +<CR>    2;");
    keys(&mut app, "J");
    assert_eq!(text(&app), "let x = 1 +\n    2;");
    assert_eq!(cursor(&app), (0, 7));
    keys(&mut app, "u3J");
    assert_eq!(text(&app), "let x = 1 + 2;");
}

#[test]
fn auto_indent_while_typing() {
    let mut app = app();
    keys(
        &mut app,
        "ifn main() {<CR>let x = 1;<CR>if x {<CR>go();<CR>}<CR>}<Esc>",
    );
    assert_eq!(
        text(&app),
        "fn main() {\n    let x = 1;\n    if x {\n        go();\n    }\n}"
    );
}

#[test]
fn o_and_capital_o_keep_indentation() {
    let mut app = app();
    keys(&mut app, "i    body<Esc>oafter<Esc>Obefore<Esc>");
    assert_eq!(text(&app), "    body\n    before\n    after");
}

#[test]
fn tabs_render_and_the_cursor_lines_up() {
    let mut app = app_with_config("[editor]\nline-numbers = \"none\"\ntab-width = 4");
    keys(&mut app, "i\tx<Esc>");
    let screen = render(&mut app, 20, 4);
    assert!(screen.rows[0].starts_with("    x"), "{:?}", screen.rows[0]);
    assert_eq!(
        screen.cursor,
        (4, 0),
        "on the x, after the tab's four cells"
    );
}
