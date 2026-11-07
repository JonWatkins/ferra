//! `|` between `:` commands, end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use ferra::App;

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
fn two_commands_on_one_line() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, ":%s/b/B/ | 1d<CR>");
    assert_eq!(text(&app), "B\nc");
}

#[test]
fn bars_in_substitute_patterns_are_alternation() {
    let mut app = with_text("cat dog cow");
    keys(&mut app, ":s/cat|dog/pet/g<CR>");
    assert_eq!(text(&app), "pet pet cow");

    let mut app = with_text("cat dog<CR>x");
    keys(&mut app, ":s/cat|dog/a|b/g|2d<CR>");
    assert_eq!(text(&app), "a|b a|b", "then the second command ran");
}

#[test]
fn g_runs_every_command_after_it_on_each_line() {
    let mut app = with_text("TODO a<CR>b<CR>TODO c");
    keys(&mut app, ":g/TODO/s/TODO/DONE/ | normal A!<CR>");
    assert_eq!(text(&app), "DONE a!\nb\nDONE c!");
}

#[test]
fn normal_keeps_its_bars() {
    let mut app = with_text("x");
    keys(&mut app, ":normal a|y<CR>");
    assert_eq!(text(&app), "x|y");
}

#[test]
fn an_escaped_bar_is_literal() {
    let mut app = with_text("x");
    keys(&mut app, ":s/x/1\\|2/<CR>");
    assert_eq!(text(&app), "1|2");
}

#[test]
fn a_range_alone_then_a_command() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, ":2|d<CR>");
    assert_eq!(text(&app), "a\nc");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn a_failing_command_stops_the_rest() {
    let mut app = with_text("a<CR>b");
    keys(&mut app, ":s/zzz/y/ | 2d<CR>");
    assert_eq!(message(&app), "Pattern not found: zzz");
    assert_eq!(text(&app), "a\nb");
}

#[test]
fn a_trailing_bar_is_harmless() {
    let mut app = with_text("a<CR>b");
    keys(&mut app, ":2d |<CR>");
    assert_eq!(text(&app), "a");
}
