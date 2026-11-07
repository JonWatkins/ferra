//! Vim's operator grammar end-to-end: `[count]["x]operator[count]motion`.

mod common;

use common::{app_with_config, cursor, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

/// An app containing `content`, with the cursor at the start. The text is
/// typed in, so auto-indent is off to keep it exactly as written.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn delete_word_motions() {
    let mut app = with_text("one two three four");
    keys(&mut app, "dw");
    assert_eq!(text(&app), "two three four");
    keys(&mut app, "d2w");
    assert_eq!(text(&app), "four");
}

#[test]
fn counts_before_and_after_operator_multiply() {
    let mut app = with_text("a b c d e f g");
    keys(&mut app, "2d2w");
    assert_eq!(text(&app), "e f g");
}

#[test]
fn dw_on_last_word_does_not_join_lines() {
    let mut app = with_text("foo bar<CR>baz");
    keys(&mut app, "wdw");
    assert_eq!(text(&app), "foo \nbaz");
}

#[test]
fn delete_to_end_of_word_is_inclusive() {
    let mut app = with_text("hello world");
    keys(&mut app, "de");
    assert_eq!(text(&app), " world");
}

#[test]
fn delete_to_line_end_and_start() {
    let mut app = with_text("hello world");
    keys(&mut app, "wd$");
    assert_eq!(text(&app), "hello ");
    keys(&mut app, "d0");
    assert_eq!(text(&app), " ");
}

#[test]
fn dd_with_counts() {
    let mut app = with_text("1<CR>2<CR>3<CR>4<CR>5");
    keys(&mut app, "dd");
    assert_eq!(text(&app), "2\n3\n4\n5");
    keys(&mut app, "2dd");
    assert_eq!(text(&app), "4\n5");
    keys(&mut app, "d5d");
    assert_eq!(text(&app), "");
}

#[test]
fn linewise_motions() {
    let mut app = with_text("1<CR>2<CR>3<CR>4<CR>5");
    keys(&mut app, "jdj");
    assert_eq!(text(&app), "1\n4\n5");
    keys(&mut app, "jdk");
    assert_eq!(text(&app), "1");

    let mut app = with_text("1<CR>2<CR>3");
    keys(&mut app, "jdG");
    assert_eq!(text(&app), "1");

    let mut app = with_text("1<CR>2<CR>3");
    keys(&mut app, "jdgg");
    assert_eq!(text(&app), "3");
}

#[test]
fn change_word_keeps_trailing_space() {
    let mut app = with_text("foo bar");
    keys(&mut app, "cwbaz<Esc>");
    assert_eq!(text(&app), "baz bar");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn change_line_and_to_end() {
    let mut app = with_text("keep<CR>  replace me<CR>keep");
    keys(&mut app, "jccnew<Esc>");
    assert_eq!(text(&app), "keep\nnew\nkeep");
    keys(&mut app, "0lC!<Esc>");
    assert_eq!(text(&app), "keep\nn!\nkeep");
}

#[test]
fn text_objects() {
    let mut app = with_text("let x = call(a, \"str\", [1, 2]);");
    keys(&mut app, "wwwwwdi(");
    assert_eq!(text(&app), "let x = call();");

    let mut app = with_text("say \"hello there\" now");
    keys(&mut app, "wwci\"bye<Esc>");
    assert_eq!(text(&app), "say \"bye\" now");

    let mut app = with_text("one two three");
    keys(&mut app, "wdaw");
    assert_eq!(text(&app), "one three");
}

#[test]
fn yank_and_paste_lines() {
    let mut app = with_text("first<CR>second");
    keys(&mut app, "yyjp");
    assert_eq!(text(&app), "first\nsecond\nfirst");
    keys(&mut app, "ggP");
    assert_eq!(text(&app), "first\nfirst\nsecond\nfirst");
}

#[test]
fn yank_word_and_paste_charwise() {
    let mut app = with_text("abc def");
    keys(&mut app, "yw$p");
    assert_eq!(text(&app), "abc defabc ");
    assert_eq!(cursor(&app), (0, 10));
}

#[test]
fn classic_idioms_xp_and_ddp() {
    let mut app = with_text("ab");
    keys(&mut app, "xp");
    assert_eq!(text(&app), "ba");

    let mut app = with_text("one<CR>two");
    keys(&mut app, "ddp");
    assert_eq!(text(&app), "two\none");
}

#[test]
fn paste_with_count() {
    let mut app = with_text("x");
    keys(&mut app, "yl3p");
    assert_eq!(text(&app), "xxxx");
}

#[test]
fn named_registers() {
    let mut app = with_text("alpha<CR>beta");
    keys(&mut app, "\"ayyj\"byy");
    keys(&mut app, "G\"ap");
    assert_eq!(text(&app), "alpha\nbeta\nalpha");
    keys(&mut app, "\"bP");
    assert_eq!(text(&app), "alpha\nbeta\nbeta\nalpha");
}

#[test]
fn uppercase_register_appends() {
    let mut app = with_text("one<CR>two<CR>");
    keys(&mut app, "gg\"ayyj\"Ayy");
    keys(&mut app, "G\"ap");
    assert_eq!(text(&app), "one\ntwo\n\none\ntwo");
}

#[test]
fn black_hole_register_preserves_unnamed() {
    let mut app = with_text("keep<CR>discard");
    keys(&mut app, "yyj\"_ddP");
    assert_eq!(text(&app), "keep\nkeep");
}

#[test]
fn yank_register_survives_deletes() {
    let mut app = with_text("yanked<CR>deleted");
    keys(&mut app, "yyjdd\"0p");
    assert_eq!(text(&app), "yanked\nyanked");
}

#[test]
fn escape_cancels_pending_operator() {
    let mut app = with_text("abc");
    keys(&mut app, "d");
    assert_eq!(app.editor().mode(), Mode::OperatorPending);
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "l");
    assert_eq!(
        text(&app),
        "abc",
        "l moves after cancelling, rather than deleting"
    );
    assert_eq!(cursor(&app), (0, 1));
}

#[test]
fn invalid_motion_cancels_operator() {
    let mut app = with_text("abc");
    keys(&mut app, "dz");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "x");
    assert_eq!(text(&app), "bc");
}

#[test]
fn pending_operator_is_shown_in_statusline() {
    let mut app = with_text("abc");
    keys(&mut app, "\"a2d3");
    let screen = render(&mut app, 30, 5);
    assert!(screen.rows[3].contains("\"a2d3"), "{:?}", screen.rows[3]);
    assert!(
        screen.rows[3].starts_with(" NORMAL "),
        "{:?}",
        screen.rows[3]
    );

    keys(&mut app, "<Esc>");
    let screen = render(&mut app, 30, 5);
    assert!(!screen.rows[3].contains('d'), "{:?}", screen.rows[3]);
}

#[test]
fn invalid_register_reports_error() {
    let mut app = with_text("abc");
    keys(&mut app, "\"%dd");
    assert_eq!(text(&app), "abc");
    assert!(
        app.editor()
            .message()
            .unwrap()
            .text
            .contains("Invalid register")
    );
}
