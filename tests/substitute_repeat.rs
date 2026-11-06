//! Repeating `:s` (`&`, `:&`, `:&&`, `g&`, bare `:s`) and confirming each
//! replacement (the `c` flag), end to end.

mod common;

use common::{app_with_config, keys, text};
use terminal_editor::App;

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
fn ampersand_repeats_on_the_current_line_without_flags() {
    let mut app = with_text("a a<CR>a a");
    keys(&mut app, ":s/a/b/g<CR>");
    assert_eq!(text(&app), "b b\na a");
    keys(&mut app, "j&");
    assert_eq!(text(&app), "b b\nb a", "no g: one replacement");
}

#[test]
fn colon_ampersands_keep_or_drop_the_flags() {
    let mut app = with_text("a a<CR>a a<CR>a a");
    keys(&mut app, ":s/a/b/g<CR>j:&&<CR>");
    assert_eq!(text(&app), "b b\nb b\na a");
    keys(&mut app, "j:&<CR>");
    assert_eq!(text(&app), "b b\nb b\nb a");
}

#[test]
fn a_bare_s_repeats_too() {
    let mut app = with_text("x1<CR>x2");
    keys(&mut app, ":s/x/y/<CR>j:s<CR>");
    assert_eq!(text(&app), "y1\ny2");
}

#[test]
fn ranges_apply_to_the_repeat() {
    let mut app = with_text("a<CR>a<CR>a");
    keys(&mut app, ":s/a/b/<CR>:%&<CR>");
    assert_eq!(text(&app), "b\nb\nb");
}

#[test]
fn g_ampersand_repeats_everywhere_with_the_last_search() {
    let mut app = with_text("a a<CR>c c<CR>a c");
    keys(&mut app, ":s/a/b/g<CR>/c<CR>g&");
    assert_eq!(text(&app), "b b\nb b\na b", "c → b on every line, g kept");
}

#[test]
fn repeating_with_nothing_to_repeat_is_an_error() {
    let mut app = with_text("abc");
    keys(&mut app, "&");
    assert_eq!(message(&app), "No previous substitute regular expression");
}

#[test]
fn confirm_asks_about_each_match() {
    let mut app = with_text("a a<CR>a");
    keys(&mut app, ":%s/a/b/gc<CR>");
    assert_eq!(message(&app), "replace with b (y/n/a/q/l)?");
    keys(&mut app, "yny");
    assert_eq!(text(&app), "b a\nb");
    assert_eq!(message(&app), "2 substitutions on 2 lines");
    keys(&mut app, "u");
    assert_eq!(text(&app), "a a\na", "the whole session is one undo step");
}

#[test]
fn confirm_all_and_quit() {
    let mut app = with_text("a a<CR>a a");
    keys(&mut app, ":%s/a/b/gc<CR>na");
    assert_eq!(text(&app), "a b\nb b");

    let mut app = with_text("a a<CR>a a");
    keys(&mut app, ":%s/a/b/gc<CR>yq");
    assert_eq!(text(&app), "b a\na a");
    // The cursor stays on the match it was asking about, as in Vim.
    keys(&mut app, "x");
    assert_eq!(text(&app), "b \na a", "back to normal keys after q");
}

#[test]
fn escape_ends_confirming_and_keeps_what_was_done() {
    let mut app = with_text("a a a");
    keys(&mut app, ":s/a/b/gc<CR>y<Esc>");
    assert_eq!(text(&app), "b a a");
    assert_eq!(message(&app), "1 substitution on 1 line");
    keys(&mut app, "u");
    assert_eq!(text(&app), "a a a");
}

#[test]
fn confirm_inside_g_style_repeats() {
    let mut app = with_text("x x");
    keys(&mut app, ":s/x/y/gc<CR>a");
    assert_eq!(text(&app), "y y");
    keys(&mut app, "0:&&<CR>");
    assert_eq!(message(&app), "Pattern not found: x");
}
