//! Searching in visual mode extends the selection (`v/foo<CR>`), end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use ferra::App;
use ferra::core::{Position, Range};
use ferra::editor::Mode;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn slash_extends_a_character_selection() {
    let mut app = with_text("one two three");
    keys(&mut app, "v/thr<CR>");
    assert_eq!(app.editor().mode(), Mode::Visual);
    assert_eq!(cursor(&app), (0, 8));
    keys(&mut app, "d");
    assert_eq!(text(&app), "hree", "inclusive of the match's first char");
}

#[test]
fn question_mark_extends_backwards() {
    let mut app = with_text("one two three");
    keys(&mut app, "$v?two<CR>d");
    assert_eq!(text(&app), "one ");
}

#[test]
fn slash_extends_a_line_selection() {
    let mut app = with_text("a<CR>b<CR>three<CR>d");
    keys(&mut app, "V/thr<CR>d");
    assert_eq!(text(&app), "d");
}

#[test]
fn the_selection_follows_the_match_while_typing() {
    let mut app = with_text("one two three");
    keys(&mut app, "v/thr");
    assert_eq!(app.editor().mode(), Mode::Command);
    assert_eq!(
        app.editor().selection_range(),
        Some(Range::Chars {
            start: Position::new(0, 0),
            end: Position::new(0, 9),
        })
    );
}

#[test]
fn escape_returns_to_the_selection_as_it_was() {
    let mut app = with_text("one two three");
    keys(&mut app, "vl/thr<Esc>");
    assert_eq!(app.editor().mode(), Mode::Visual);
    assert_eq!(cursor(&app), (0, 1));
    keys(&mut app, "d");
    assert_eq!(text(&app), "e two three");
}

#[test]
fn backspacing_out_of_the_prompt_also_returns_to_visual() {
    let mut app = with_text("one two");
    keys(&mut app, "vl/t<BS><BS>");
    assert_eq!(app.editor().mode(), Mode::Visual);
    assert_eq!(cursor(&app), (0, 1));
}

#[test]
fn a_failed_search_keeps_the_selection() {
    let mut app = with_text("one two");
    keys(&mut app, "vl/zzz<CR>");
    assert_eq!(message(&app), "Pattern not found: zzz");
    assert_eq!(app.editor().mode(), Mode::Visual);
    assert_eq!(cursor(&app), (0, 1));
}

#[test]
fn n_keeps_extending_after_the_search() {
    let mut app = with_text("a x b x c");
    keys(&mut app, "v/x<CR>n");
    assert_eq!(cursor(&app), (0, 6));
    keys(&mut app, "d");
    assert_eq!(text(&app), " c");
}

#[test]
fn normal_mode_searches_are_unaffected() {
    let mut app = with_text("one two");
    keys(&mut app, "/two<CR>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "/zz<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(app.editor().selection().is_none());
}
