//! `:normal`, alone, over ranges and from `:g`, end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use terminal_editor::App;
use terminal_editor::editor::Mode;

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
fn append_to_every_line() {
    let mut app = with_text("let a = 1<CR>let b = 2");
    keys(&mut app, ":%normal A;<CR>");
    assert_eq!(text(&app), "let a = 1;\nlet b = 2;");
    assert_eq!(app.editor().mode(), Mode::Normal, "the insert was ended");
    keys(&mut app, "u");
    assert_eq!(text(&app), "let a = 1\nlet b = 2", "one undo step");
}

#[test]
fn without_a_range_it_runs_at_the_cursor() {
    let mut app = with_text("abcd");
    keys(&mut app, "ll:norm x<CR>");
    assert_eq!(text(&app), "abd");
}

#[test]
fn with_a_range_each_line_starts_at_column_zero() {
    let mut app = with_text("abc<CR>abc<CR>abc");
    keys(&mut app, "l:2,3normal x<CR>");
    assert_eq!(text(&app), "abc\nbc\nbc");
}

#[test]
fn from_g_on_matching_lines() {
    let mut app = with_text("fn a()<CR>x<CR>fn b()");
    keys(&mut app, ":g/^fn/normal >><CR>");
    assert_eq!(text(&app), "    fn a()\nx\n    fn b()");
}

#[test]
fn deleting_lines_doesnt_shift_the_rest_onto_the_wrong_ones() {
    let mut app = with_text("x<CR>x<CR>x<CR>y");
    keys(&mut app, ":1,3normal dd<CR>");
    assert_eq!(text(&app), "y", "each dd hits one of the three x lines");

    let mut app = with_text("a1<CR>b<CR>a2<CR>b");
    keys(&mut app, ":g/a/normal ddp<CR>");
    assert_eq!(text(&app), "b\na1\nb\na2");
}

#[test]
fn key_notation_and_literal_angle_brackets() {
    let mut app = with_text("one<CR>two");
    keys(&mut app, ":%normal A!<lt>Esc>0x<CR>");
    assert_eq!(text(&app), "ne!\nwo!", "<Esc> ends the insert, then 0x");

    let mut app = with_text("x");
    keys(&mut app, ":normal i<lt>div><CR>");
    assert_eq!(text(&app), "<div>x", "not key notation: typed as it is");
}

#[test]
fn a_failing_command_stops_the_run() {
    let mut app = with_text("a<CR>b");
    keys(&mut app, ":%normal /zzz<lt>CR>x<CR>");
    assert_eq!(text(&app), "a\nb", "nothing after the failed search ran");
    assert_eq!(message(&app), "Pattern not found: zzz");
}

#[test]
fn the_last_change_can_be_repeated() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, ":normal A;<CR>j.");
    assert_eq!(text(&app), "a;\nb;\nc");
    assert_eq!(cursor(&app), (1, 1));
}
