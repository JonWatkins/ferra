//! End-to-end editing behaviour, driven by key presses with the default keymap.

mod common;

use common::{app, cursor, keys, text};
use terminal_editor::editor::Mode;

#[test]
fn starts_in_normal_mode_where_keys_are_commands() {
    let mut app = app();
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "jkhl");
    assert_eq!(text(&app), "");
}

#[test]
fn insert_text_and_return_to_normal_mode() {
    let mut app = app();
    keys(&mut app, "ihello<CR>world<Esc>");
    assert_eq!(text(&app), "hello\nworld");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(cursor(&app), (1, 4), "Esc steps back onto the last char");
}

#[test]
fn command_keys_are_typed_literally_in_insert_mode() {
    let mut app = app();
    keys(&mut app, "iq:jk0dd<Esc>");
    assert_eq!(text(&app), "q:jk0dd");
}

#[test]
fn append_variants() {
    let mut app = app();
    keys(&mut app, "ibc<Esc>Ia<Esc>Ad<Esc>");
    assert_eq!(text(&app), "abcd");
    keys(&mut app, "0aX<Esc>");
    assert_eq!(text(&app), "aXbcd");
}

#[test]
fn open_lines_above_and_below() {
    let mut app = app();
    keys(&mut app, "imiddle<Esc>otop?<Esc>");
    keys(&mut app, "kObefore<Esc>");
    assert_eq!(text(&app), "before\nmiddle\ntop?");
}

#[test]
fn counts_repeat_motions() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<CR>d<CR>e<Esc>gg");
    keys(&mut app, "3j");
    assert_eq!(cursor(&app), (3, 0));
    keys(&mut app, "2k");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn goto_line_with_count_and_gg_and_g() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<Esc>");
    keys(&mut app, "gg");
    assert_eq!(cursor(&app), (0, 0));
    keys(&mut app, "G");
    assert_eq!(cursor(&app), (2, 0));
    keys(&mut app, "2gg");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn word_motions() {
    let mut app = app();
    keys(&mut app, "ione two three<Esc>0");
    keys(&mut app, "w");
    assert_eq!(cursor(&app), (0, 4));
    keys(&mut app, "e");
    assert_eq!(cursor(&app), (0, 6));
    keys(&mut app, "2b");
    assert_eq!(cursor(&app), (0, 0));
}

#[test]
fn line_motions() {
    let mut app = app();
    keys(&mut app, "i    indented<Esc>");
    keys(&mut app, "0");
    assert_eq!(cursor(&app), (0, 0));
    keys(&mut app, "^");
    assert_eq!(cursor(&app), (0, 4));
    keys(&mut app, "$");
    assert_eq!(cursor(&app), (0, 11));
}

#[test]
fn delete_chars_and_lines() {
    let mut app = app();
    keys(&mut app, "ione<CR>two<CR>three<Esc>gg");
    keys(&mut app, "2x");
    assert_eq!(text(&app), "e\ntwo\nthree");
    keys(&mut app, "jdd");
    assert_eq!(text(&app), "e\nthree");
    keys(&mut app, "dd");
    assert_eq!(text(&app), "e", "dd on the last line moves the cursor up");
    keys(&mut app, "5dd");
    assert_eq!(
        text(&app),
        "",
        "count larger than remaining lines is clamped"
    );
}

#[test]
fn insert_mode_backspace_and_delete() {
    let mut app = app();
    keys(&mut app, "iab<CR>cd<BS><BS><BS><Esc>");
    assert_eq!(text(&app), "ab");
    keys(&mut app, "0i<Del><Esc>");
    assert_eq!(text(&app), "b");
}

#[test]
fn tab_inserts_spaces() {
    let mut app = app();
    keys(&mut app, "i<Tab>x<Esc>");
    assert_eq!(text(&app), "    x");
}

#[test]
fn colon_q_quits() {
    let mut app = app();
    keys(&mut app, ":q");
    assert_eq!(app.editor().mode(), Mode::Command);
    assert!(!app.should_quit());
    keys(&mut app, "<CR>");
    assert!(app.should_quit());
}

#[test]
fn colon_number_jumps_to_line() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<Esc>:2<CR>");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn escape_cancels_command_line() {
    let mut app = app();
    keys(&mut app, ":q<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(!app.should_quit());
}

#[test]
fn unknown_command_shows_error() {
    let mut app = app();
    keys(&mut app, ":nope<CR>");
    let message = app.editor().message().expect("an error message");
    assert_eq!(message.text, "Unknown command: nope");
}

#[test]
fn next_command_clears_message() {
    let mut app = app();
    keys(&mut app, ":nope<CR>");
    keys(&mut app, "j");
    assert!(app.editor().message().is_none());
}
