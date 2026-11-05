//! Undo/redo boundaries match Vim: one step per normal-mode command, and one
//! per insert session (including the command that started it).

mod common;

use common::{app, cursor, keys, text};

#[test]
fn insert_session_is_one_step() {
    let mut app = app();
    keys(&mut app, "ihello<CR>world<BS><BS><Esc>");
    assert_eq!(text(&app), "hello\nwor");
    keys(&mut app, "u");
    assert_eq!(text(&app), "");
}

#[test]
fn separate_sessions_are_separate_steps() {
    let mut app = app();
    keys(&mut app, "ione<Esc>otwo<Esc>");
    keys(&mut app, "u");
    assert_eq!(text(&app), "one");
    keys(&mut app, "u");
    assert_eq!(text(&app), "");
}

#[test]
fn redo_restores_undone_changes() {
    let mut app = app();
    keys(&mut app, "ione<Esc>otwo<Esc>uu");
    keys(&mut app, "<C-r>");
    assert_eq!(text(&app), "one");
    keys(&mut app, "<C-r>");
    assert_eq!(text(&app), "one\ntwo");
}

#[test]
fn counts_undo_multiple_steps() {
    let mut app = app();
    keys(&mut app, "iabcd<Esc>xxx");
    assert_eq!(text(&app), "a");
    keys(&mut app, "3u");
    assert_eq!(text(&app), "abcd");
    keys(&mut app, "2<C-r>");
    assert_eq!(text(&app), "ab");
}

#[test]
fn counted_command_is_one_step() {
    let mut app = app();
    keys(&mut app, "iabcdef<Esc>0");
    keys(&mut app, "3x");
    keys(&mut app, "u");
    assert_eq!(text(&app), "abcdef");
}

#[test]
fn change_and_typed_text_undo_together() {
    let mut app = app();
    keys(&mut app, "ifoo bar<Esc>0");
    keys(&mut app, "cwbaz<Esc>");
    assert_eq!(text(&app), "baz bar");
    keys(&mut app, "u");
    assert_eq!(text(&app), "foo bar");
}

#[test]
fn undo_restores_deleted_lines_and_cursor() {
    let mut app = app();
    keys(&mut app, "ione<CR>two<CR>three<Esc>");
    keys(&mut app, "kdd");
    assert_eq!(text(&app), "one\nthree");
    keys(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo\nthree");
    assert_eq!(cursor(&app).0, 1);
}

#[test]
fn paste_is_undoable() {
    let mut app = app();
    keys(&mut app, "iline<Esc>yy3p");
    assert_eq!(text(&app), "line\nline\nline\nline");
    keys(&mut app, "u");
    assert_eq!(text(&app), "line");
}

#[test]
fn new_change_discards_redo() {
    let mut app = app();
    keys(&mut app, "ia<Esc>u");
    keys(&mut app, "ib<Esc>");
    keys(&mut app, "<C-r>");
    assert_eq!(text(&app), "b");
    assert_eq!(
        app.editor().message().unwrap().text,
        "Already at newest change"
    );
}

#[test]
fn movement_does_not_create_undo_steps() {
    let mut app = app();
    keys(&mut app, "ione<CR>two<Esc>");
    keys(&mut app, "kjhlwbgg");
    keys(&mut app, "u");
    assert_eq!(text(&app), "");
}

#[test]
fn undo_with_nothing_to_undo_shows_message() {
    let mut app = app();
    keys(&mut app, "u");
    assert_eq!(
        app.editor().message().unwrap().text,
        "Already at oldest change"
    );
}
