//! `.` repeats the last change, as in Vim.

mod common;

use common::{app, cursor, keys, text};
use terminal_editor::App;
use terminal_editor::editor::Mode;

/// An app containing `content`, with the cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn repeats_simple_deletes() {
    let mut app = with_text("abcdef");
    keys(&mut app, "x..");
    assert_eq!(text(&app), "def");
}

#[test]
fn repeats_operator_with_motion() {
    let mut app = with_text("one two three four");
    keys(&mut app, "dw..");
    assert_eq!(text(&app), "four");
}

#[test]
fn keeps_the_original_count() {
    let mut app = with_text("abcdefgh");
    keys(&mut app, "2x.");
    assert_eq!(text(&app), "efgh");
}

#[test]
fn a_new_count_replaces_the_original() {
    let mut app = with_text("abcdefgh");
    keys(&mut app, "2x3.");
    assert_eq!(text(&app), "fgh");
    keys(&mut app, "x1.");
    assert_eq!(text(&app), "h", "the replacement count isn't remembered");
}

#[test]
fn repeats_insert_sessions() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, "A;<Esc>j.j.");
    assert_eq!(text(&app), "a;\nb;\nc;");
}

#[test]
fn repeats_change_with_typed_text() {
    let mut app = with_text("old old old");
    keys(&mut app, "cwnew<Esc>w.w.");
    assert_eq!(text(&app), "new new new");
}

#[test]
fn repeats_open_line() {
    let mut app = with_text("x");
    keys(&mut app, "o- item<Esc>..");
    assert_eq!(text(&app), "x\n- item\n- item\n- item");
}

#[test]
fn repeats_commands_with_a_char_argument() {
    let mut app = with_text("f(a) g(b) h(c)");
    keys(&mut app, "f(lct)x<Esc>");
    keys(&mut app, "f(l.f(l.");
    assert_eq!(text(&app), "f(x) g(x) h(x)");

    let mut app = with_text("aaaa");
    keys(&mut app, "rbl.l.");
    assert_eq!(text(&app), "bbba");
}

#[test]
fn repeats_line_operations() {
    let mut app = with_text("1<CR>2<CR>3<CR>4");
    keys(&mut app, "dd.");
    assert_eq!(text(&app), "3\n4");
}

#[test]
fn repeats_with_the_same_register() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, "\"qdd.");
    keys(&mut app, "\"qp");
    assert_eq!(text(&app), "c\nb");
}

#[test]
fn repeats_visual_operations() {
    let mut app = with_text("abcdef");
    keys(&mut app, "vld.");
    assert_eq!(text(&app), "ef");
}

#[test]
fn movement_undo_and_yank_dont_replace_the_last_change() {
    let mut app = with_text("abcdef");
    keys(&mut app, "x");
    keys(&mut app, "lhyyu");
    assert_eq!(text(&app), "abcdef");
    keys(&mut app, ".");
    assert_eq!(text(&app), "bcdef", "still repeats x");
}

#[test]
fn command_line_commands_are_not_repeated() {
    let mut app = with_text("abc");
    keys(&mut app, "x");
    keys(&mut app, ":goto_line 1<CR>");
    keys(&mut app, ".");
    assert_eq!(text(&app), "c");
}

#[test]
fn repeat_is_one_undo_step() {
    let mut app = with_text("one two");
    keys(&mut app, "cwONE<Esc>w.");
    assert_eq!(text(&app), "ONE ONE");
    keys(&mut app, "u");
    assert_eq!(text(&app), "ONE two");
}

#[test]
fn repeating_doesnt_replace_the_last_change() {
    let mut app = with_text("abcdef");
    keys(&mut app, "x...");
    assert_eq!(text(&app), "ef");
}

#[test]
fn nothing_to_repeat_is_harmless() {
    // A fresh app: no change has been made yet (with_text's typing would
    // itself be a repeatable insert).
    let mut app = app();
    keys(&mut app, ".");
    assert_eq!(text(&app), "");
    assert_eq!(cursor(&app), (0, 0));
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn the_insert_that_typed_the_text_is_itself_repeatable() {
    let mut app = with_text("abc");
    keys(&mut app, "$.");
    assert_eq!(text(&app), "ababcc");
}

#[test]
fn failed_change_is_not_recorded() {
    let mut app = with_text("abc");
    keys(&mut app, "x");
    keys(&mut app, "dfz");
    keys(&mut app, ".");
    assert_eq!(text(&app), "c", "repeats x, not the failed dfz");
}

#[test]
fn switching_buffers_is_not_the_last_change() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let mut app = app();
    app.open(&a);
    keys(&mut app, "x");
    keys(&mut app, &format!(":e {}<CR>", b.display()));
    keys(&mut app, "xx");
    // The buffers have had different numbers of edits, which used to make
    // `]b` look like an edit.
    keys(&mut app, "]b");
    assert_eq!(text(&app), "aa");
    keys(&mut app, ".");
    assert_eq!(text(&app), "a", "`.` repeats the x, in this buffer");
}
