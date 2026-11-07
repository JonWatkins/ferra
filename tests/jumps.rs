//! The jump list (`<C-o>`, `<C-i>`, `''`) and change list (`g;`, `g,`,
//! `'.`) end to end.

mod common;

use std::fs;

use common::{app, app_with_config, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// Lines `1`..=`n`, typed without auto-indent, cursor on line 1.
fn numbered(n: usize) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    let text = (1..=n)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("<CR>");
    keys(&mut app, &format!("i{text}<Esc>gg"));
    app
}

#[test]
fn ctrl_o_and_ctrl_i_walk_the_jump_list() {
    let mut app = numbered(20);
    keys(&mut app, "5G");
    keys(&mut app, "15G");
    keys(&mut app, "<C-o>");
    assert_eq!(cursor(&app).0, 4);
    keys(&mut app, "<C-o>");
    assert_eq!(
        cursor(&app).0,
        0,
        "back past both jumps (gg's start included)"
    );
    keys(&mut app, "<C-i>");
    assert_eq!(cursor(&app).0, 4);
    keys(&mut app, "<Tab>");
    assert_eq!(cursor(&app).0, 14, "<Tab> is <C-i>");
}

#[test]
fn small_moves_are_not_jumps() {
    let mut app = numbered(20);
    keys(&mut app, "3jG");
    keys(&mut app, "kkwb");
    keys(&mut app, "<C-o>");
    assert_eq!(
        cursor(&app).0,
        3,
        "straight back to before `G`, not to the `k`s"
    );
}

#[test]
fn counts_go_back_several_jumps() {
    let mut app = numbered(30);
    keys(&mut app, "10G20G30G2<C-o>");
    assert_eq!(cursor(&app).0, 9);
}

#[test]
fn searches_and_brackets_are_jumps() {
    let mut app = numbered(10);
    keys(&mut app, "/7<CR>");
    assert_eq!(cursor(&app).0, 6);
    keys(&mut app, "<C-o>");
    assert_eq!(cursor(&app).0, 0);

    let mut app = common::app();
    keys(&mut app, "if(<CR>x<CR>)<Esc>gg%");
    assert_eq!(cursor(&app).0, 2);
    keys(&mut app, "<C-o>");
    assert_eq!(cursor(&app).0, 0);
}

#[test]
fn colon_line_numbers_are_jumps() {
    let mut app = numbered(10);
    keys(&mut app, ":8<CR><C-o>");
    assert_eq!(cursor(&app).0, 0);
}

#[test]
fn quote_quote_toggles_between_two_places() {
    let mut app = numbered(10);
    keys(&mut app, "3Gll"); // stays at column 0: one-character lines
    keys(&mut app, "8G''");
    assert_eq!(cursor(&app).0, 2);
    keys(&mut app, "''");
    assert_eq!(cursor(&app).0, 7);
    keys(&mut app, "``");
    assert_eq!(cursor(&app).0, 2);
}

#[test]
fn mark_jumps_are_recorded() {
    let mut app = numbered(10);
    keys(&mut app, "6GmaggG'a<C-o>");
    assert_eq!(cursor(&app).0, 9, "back to before 'a");
}

#[test]
fn the_jump_list_spans_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "alpha 1\nalpha 2\nalpha 3\n").unwrap();
    fs::write(&b, "beta\n").unwrap();
    let mut app = app();
    app.open(&a);
    keys(&mut app, "G");
    keys(&mut app, &format!(":e {}<CR>", b.display()));
    assert_eq!(text(&app), "beta");
    keys(&mut app, "<C-o>");
    assert_eq!(text(&app), "alpha 1\nalpha 2\nalpha 3");
    assert_eq!(cursor(&app).0, 2, "where we left a.txt");
    keys(&mut app, "<C-i>");
    assert_eq!(text(&app), "beta");
}

#[test]
fn messages_at_the_ends_of_the_list() {
    let mut app = numbered(3);
    keys(&mut app, "<C-i>");
    assert_eq!(message(&app), "At the end of the jump list");
}

#[test]
fn change_list_and_last_change_mark() {
    let mut app = numbered(10);
    keys(&mut app, "3GAx<Esc>7GAy<Esc>G");
    keys(&mut app, "g;");
    assert_eq!(cursor(&app).0, 6);
    keys(&mut app, "g;");
    assert_eq!(cursor(&app).0, 2);
    keys(&mut app, "g,");
    assert_eq!(cursor(&app).0, 6);
    keys(&mut app, "gg'.");
    assert_eq!(cursor(&app).0, 6, "'. is the last change");
}
