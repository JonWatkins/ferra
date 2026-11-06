//! `:s` and line ranges end to end.

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

/// An app containing `content` (typed without auto-indent), cursor at start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn whole_file_global_substitution() {
    let mut app = with_text("foo foo<CR>bar<CR>foo");
    keys(&mut app, ":%s/foo/baz/g<CR>");
    assert_eq!(text(&app), "baz baz\nbar\nbaz");
    assert_eq!(message(&app), "3 substitutions on 2 lines");
    keys(&mut app, "u");
    assert_eq!(text(&app), "foo foo\nbar\nfoo", "one undo step");
}

#[test]
fn ranges_select_lines() {
    let mut app = with_text("x<CR>x<CR>x<CR>x");
    keys(&mut app, ":2,3s/x/y/<CR>");
    assert_eq!(text(&app), "x\ny\ny\nx");
    keys(&mut app, ":.,$s/./z/<CR>");
    assert_eq!(text(&app), "x\ny\nz\nz", "from the cursor (left on line 3) to the end");
}

#[test]
fn visual_colon_fills_in_the_selection_range() {
    let mut app = with_text("a<CR>a<CR>a");
    keys(&mut app, "jVj:");
    assert_eq!(app.editor().mode(), Mode::Command);
    assert_eq!(app.editor().command_line.input(), "'<,'>");
    keys(&mut app, "s/a/b/<CR>");
    assert_eq!(text(&app), "a\nb\nb");
}

#[test]
fn patterns_with_spaces_groups_and_other_delimiters() {
    let mut app = with_text("let first name = 1;");
    keys(&mut app, r":s/first name/full_name/<CR>");
    assert_eq!(text(&app), "let full_name = 1;");

    let mut app = with_text("path /usr/local/bin");
    keys(&mut app, ":s#/usr/local#/opt#<CR>");
    assert_eq!(text(&app), "path /opt/bin");

    let mut app = with_text("swap(a, b)");
    keys(&mut app, r":s/(\w+), (\w+)/\2, \1/<CR>");
    assert_eq!(text(&app), "swap(b, a)");
}

#[test]
fn n_finds_the_substituted_pattern_next() {
    let mut app = with_text("one<CR>two<CR>one");
    keys(&mut app, ":1s/one/ONE/<CR>");
    keys(&mut app, "gg/one<CR>");
    assert_eq!(cursor(&app), (2, 0), "smart case: `one` still finds the last line");
}

#[test]
fn errors() {
    let mut app = with_text("abc");
    keys(&mut app, ":s/zzz/y/<CR>");
    assert_eq!(message(&app), "Pattern not found: zzz");
    keys(&mut app, ":s/a/b/q<CR>");
    assert!(message(&app).contains("Unknown flag 'q'"), "{}", message(&app));
    keys(&mut app, ":s/(/x/<CR>");
    assert!(message(&app).starts_with("Invalid pattern"), "{}", message(&app));
    assert_eq!(text(&app), "abc");
}

#[test]
fn delete_and_yank_with_ranges() {
    let mut app = with_text("1<CR>2<CR>3<CR>4<CR>5");
    keys(&mut app, ":2,4d<CR>");
    assert_eq!(text(&app), "1\n5");
    keys(&mut app, ":%y<CR>Gp");
    assert_eq!(text(&app), "1\n5\n1\n5");
}

#[test]
fn ranges_alone_go_to_lines() {
    let mut app = with_text("a<CR>b<CR>c<CR>d");
    keys(&mut app, ":$<CR>");
    assert_eq!(cursor(&app), (3, 0));
    keys(&mut app, ":-2<CR>");
    assert_eq!(cursor(&app), (1, 0));
    keys(&mut app, ":.+1<CR>");
    assert_eq!(cursor(&app), (2, 0));
}

#[test]
fn substitute_commands_go_into_history() {
    let mut app = with_text("a a");
    keys(&mut app, ":s/a/b/<CR>:<Up><CR>");
    assert_eq!(text(&app), "b b", "the recalled :s ran again on the next `a`");
}
