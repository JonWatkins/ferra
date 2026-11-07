//! Character-search, bracket and paragraph motions end-to-end, with and
//! without operators.

mod common;

use common::{app, cursor, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

/// An app containing `content`, with the cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn f_and_t_move_on_the_line() {
    let mut app = with_text("one, two, three");
    keys(&mut app, "f,");
    assert_eq!(cursor(&app), (0, 3));
    keys(&mut app, "0t,");
    assert_eq!(cursor(&app), (0, 2));
    keys(&mut app, "$F,");
    assert_eq!(cursor(&app), (0, 8));
    keys(&mut app, "$T,");
    assert_eq!(cursor(&app), (0, 9));
}

#[test]
fn counts_find_the_nth_match() {
    let mut app = with_text("a-b-c-d");
    keys(&mut app, "3f-");
    assert_eq!(cursor(&app), (0, 5));
}

#[test]
fn semicolon_and_comma_repeat() {
    let mut app = with_text("a-b-c-d");
    keys(&mut app, "f-;");
    assert_eq!(cursor(&app), (0, 3));
    keys(&mut app, ";");
    assert_eq!(cursor(&app), (0, 5));
    keys(&mut app, ",");
    assert_eq!(cursor(&app), (0, 3));
}

#[test]
fn semicolon_after_t_moves_on() {
    let mut app = with_text("a-b-c");
    keys(&mut app, "t-");
    assert_eq!(cursor(&app), (0, 0));
    keys(&mut app, ";");
    assert_eq!(cursor(&app), (0, 2));
}

#[test]
fn failed_search_does_not_move() {
    let mut app = with_text("abc");
    keys(&mut app, "lfz");
    assert_eq!(cursor(&app), (0, 1));
}

#[test]
fn search_char_can_be_a_bound_key() {
    let mut app = with_text("a:b");
    keys(&mut app, "f:");
    assert_eq!(cursor(&app), (0, 1));
    assert_eq!(
        app.editor().mode(),
        Mode::Normal,
        "':' was the argument, not a command"
    );
}

#[test]
fn escape_cancels_the_search() {
    let mut app = with_text("abc");
    keys(&mut app, "f<Esc>l");
    assert_eq!(cursor(&app), (0, 1));
}

#[test]
fn delete_and_change_with_char_searches() {
    let mut app = with_text("call(a, b);");
    keys(&mut app, "df(");
    assert_eq!(text(&app), "a, b);");

    let mut app = with_text("call(a, b);");
    keys(&mut app, "f(lct)x<Esc>");
    assert_eq!(text(&app), "call(x);");

    let mut app = with_text("one two three");
    // Back to the nearest `t` (in "three"); F is exclusive, so the `e`
    // under the cursor stays.
    keys(&mut app, "$dFt");
    assert_eq!(text(&app), "one two e");
}

#[test]
fn failed_search_cancels_the_operator() {
    let mut app = with_text("abc");
    keys(&mut app, "dfz");
    assert_eq!(text(&app), "abc");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "x");
    assert_eq!(text(&app), "bc", "the next key works normally");
}

#[test]
fn semicolon_repeats_inside_an_operator() {
    let mut app = with_text("a,b,c,d");
    // `;` repeats `f,` inclusively: from the first comma through the next.
    keys(&mut app, "f,d;");
    assert_eq!(text(&app), "ac,d");
}

#[test]
fn visual_mode_extends_with_char_search() {
    let mut app = with_text("keep (drop this) keep");
    keys(&mut app, "f(vf)d");
    assert_eq!(text(&app), "keep  keep");
}

#[test]
fn pending_search_is_shown() {
    let mut app = with_text("abc");
    keys(&mut app, "d2f");
    let status = render(&mut app, 30, 5).rows[3].clone();
    assert!(status.contains("d2f"), "{status:?}");
    keys(&mut app, "c");
    let status = render(&mut app, 30, 5).rows[3].clone();
    assert!(!status.contains("d2f"), "{status:?}");
}

#[test]
fn percent_jumps_between_brackets() {
    let mut app = with_text("if (a[0] == b) {");
    keys(&mut app, "%");
    assert_eq!(
        cursor(&app),
        (0, 13),
        "from line start, the first bracket's match"
    );
    keys(&mut app, "%");
    assert_eq!(cursor(&app), (0, 3));
}

#[test]
fn delete_to_matching_bracket() {
    let mut app = with_text("x = f(a, (b)) + 1");
    keys(&mut app, "f(d%");
    assert_eq!(text(&app), "x = f + 1");
}

#[test]
fn percent_across_lines() {
    let mut app = with_text("fn main() {<CR>body();<CR>}");
    keys(&mut app, "$%");
    assert_eq!(cursor(&app), (2, 0));
}

#[test]
fn paragraph_motions() {
    let mut app = with_text("a<CR>b<CR><CR>c<CR>d<CR><CR>e");
    keys(&mut app, "}");
    assert_eq!(cursor(&app), (2, 0));
    keys(&mut app, "}");
    assert_eq!(cursor(&app), (5, 0));
    keys(&mut app, "2{");
    assert_eq!(cursor(&app), (0, 0));
}

#[test]
fn delete_paragraph_removes_whole_lines() {
    let mut app = with_text("one<CR>two<CR><CR>three");
    keys(&mut app, "d}");
    assert_eq!(text(&app), "\nthree");
}

#[test]
fn dw_on_last_word_keeps_the_line() {
    let mut app = with_text("foo<CR>bar");
    keys(&mut app, "dw");
    assert_eq!(text(&app), "\nbar");
}

#[test]
fn dw_across_lines_stops_at_the_word() {
    let mut app = with_text("a b<CR>c d");
    keys(&mut app, "d3w");
    assert_eq!(text(&app), "d");
}

#[test]
fn r_replaces_characters() {
    let mut app = with_text("hello");
    keys(&mut app, "rj");
    assert_eq!(text(&app), "jello");
    keys(&mut app, "l3ra");
    assert_eq!(text(&app), "jaaao");
    assert_eq!(cursor(&app), (0, 3));
    keys(&mut app, "u");
    assert_eq!(text(&app), "jello");
}
