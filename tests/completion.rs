//! Insert-mode word completion (`<C-n>`, `<C-p>`), end to end.

mod common;

use common::{app, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// "foobar foobaz" then a line "fo", in insert mode after the "fo".
fn typing_fo() -> App {
    let mut app = app();
    keys(&mut app, "ifoobar foobaz<CR>fo");
    app
}

fn last_line(app: &App) -> String {
    text(app).lines().last().unwrap_or_default().to_string()
}

#[test]
fn ctrl_n_goes_through_the_matches_and_back_to_what_was_typed() {
    let mut app = typing_fo();
    keys(&mut app, "<C-n>");
    assert_eq!(last_line(&app), "foobar");
    keys(&mut app, "<C-n>");
    assert_eq!(last_line(&app), "foobaz");
    keys(&mut app, "<C-n>");
    assert_eq!(last_line(&app), "fo", "round to what was typed");
    keys(&mut app, "<C-n>");
    assert_eq!(last_line(&app), "foobar");
    keys(&mut app, "<C-p><C-p>");
    assert_eq!(last_line(&app), "foobaz", "back the other way");
}

#[test]
fn ctrl_p_starts_with_the_nearest_match_before() {
    let mut app = typing_fo();
    keys(&mut app, "<C-p>");
    assert_eq!(last_line(&app), "foobaz");
}

#[test]
fn typing_on_keeps_the_match() {
    let mut app = typing_fo();
    keys(&mut app, "<C-n>!");
    assert_eq!(last_line(&app), "foobar!");
    keys(&mut app, " fooba<C-n><C-y>");
    assert_eq!(last_line(&app), "foobar! foobar");
    assert!(app.editor().completion().is_none(), "<C-y> ends it");
}

#[test]
fn ctrl_e_puts_back_what_was_typed_and_esc_keeps_the_match() {
    let mut app = typing_fo();
    keys(&mut app, "<C-n><C-n><C-e>");
    assert_eq!(last_line(&app), "fo");
    keys(&mut app, "<C-n><Esc>");
    assert_eq!(last_line(&app), "foobar");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(
        app.editor()
            .registers
            .get(Some('.'))
            .map(|r| r.text.clone()),
        Some("foobar foobaz\nfoobar".into()),
        "\". has the completed word"
    );
}

#[test]
fn nothing_to_complete_says_so() {
    let mut app = app();
    keys(&mut app, "izzz<C-n>");
    assert_eq!(message(&app), "No completions for \"zzz\"");
    assert_eq!(text(&app), "zzz");
}

#[test]
fn words_come_from_the_other_buffers_too() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.txt");
    std::fs::write(&other, "elsewhere_word\n").unwrap();
    let mut app = app();
    app.open(&other);
    app.open(&dir.path().join("new.txt"));
    keys(&mut app, "ielse<C-n>");
    assert_eq!(text(&app), "elsewhere_word");
}

#[test]
fn the_menu_shows_the_matches_by_the_cursor() {
    let mut app = typing_fo();
    keys(&mut app, "<C-n>");
    let rows = render(&mut app, 40, 10).rows;
    assert!(rows[2].contains(" foobar "), "{rows:?}");
    assert!(rows[3].contains(" foobaz "), "{rows:?}");
    keys(&mut app, "<C-y>");
    let rows = render(&mut app, 40, 10).rows;
    assert!(!rows[2].contains("foobar"), "gone once done: {rows:?}");
}
