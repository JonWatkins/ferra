//! Smaller Vim commands end to end: `gJ`, `{count}%`, `<C-r><C-w>` and
//! `<C-r><C-a>` on the command line, and `~` in `:s` replacements.

mod common;

use common::{app_with_config, cursor, keys, text};
use terminal_editor::App;
use terminal_editor::editor::Mode;

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

fn numbered(n: usize) -> App {
    let lines: Vec<String> = (1..=n).map(|i| format!("  {i}")).collect();
    with_text(&lines.join("<CR>"))
}

#[test]
fn g_capital_j_joins_exactly() {
    let mut app = with_text("foo<CR>    bar<CR>baz");
    keys(&mut app, "gJ");
    assert_eq!(text(&app), "foo    bar\nbaz", "indent kept, no space added");
    assert_eq!(cursor(&app), (0, 3));
    keys(&mut app, "u3gJ");
    assert_eq!(text(&app), "foo    barbaz", "a count joins that many lines");

    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, "VjgJ");
    assert_eq!(text(&app), "ab\nc");
}

#[test]
fn count_percent_goes_to_that_part_of_the_file() {
    let mut app = numbered(10);
    keys(&mut app, "50%");
    assert_eq!(cursor(&app), (4, 2), "line 5, first non-blank");
    keys(&mut app, "100%");
    assert_eq!(cursor(&app).0, 9);
    keys(&mut app, "1%");
    assert_eq!(cursor(&app).0, 0);
    keys(&mut app, "<C-o>");
    assert_eq!(cursor(&app).0, 9, "it's a jump");
}

#[test]
fn percent_without_a_count_still_matches_brackets() {
    let mut app = with_text("(a)");
    keys(&mut app, "%");
    assert_eq!(cursor(&app), (0, 2));
}

#[test]
fn delete_to_a_percent_is_linewise() {
    let mut app = numbered(10);
    keys(&mut app, "d50%");
    assert_eq!(text(&app).lines().next(), Some("  6"));
    let mut app = numbered(10);
    keys(&mut app, "50d%");
    assert_eq!(
        text(&app).lines().next(),
        Some("  6"),
        "the count before d works too"
    );
}

#[test]
fn ctrl_r_ctrl_w_inserts_the_word_under_the_cursor() {
    let mut app = with_text("let total = 1;<CR>total += 1;");
    keys(&mut app, "w:s/<C-r><C-w>/sum/<CR>");
    assert_eq!(text(&app), "let sum = 1;\ntotal += 1;");

    let mut app = with_text("x foo-bar y<CR>foo-bar");
    keys(&mut app, "w/<C-r><C-a><CR>");
    assert_eq!(cursor(&app), (1, 0), "the WORD, punctuation included");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn ctrl_r_ctrl_w_uses_where_the_search_began() {
    let mut app = with_text("alpha beta<CR>gamma beta");
    // While typing `/b` the preview moves the cursor to `beta`; the word
    // inserted is still the one the cursor was on, `alpha`.
    keys(&mut app, "/b<C-r><C-w>");
    assert_eq!(app.editor().command_line.input(), "balpha");
}

#[test]
fn tilde_reuses_the_previous_replacement() {
    let mut app = with_text("a b");
    keys(&mut app, ":s/a/X/<CR>:s/b/~~/<CR>");
    assert_eq!(text(&app), "X XX");
    keys(&mut app, ":s/X/[\\~]/<CR>");
    assert_eq!(text(&app), "[~] XX", "an escaped tilde is literal");
}

#[test]
fn repeating_doesnt_expand_a_tilde_twice() {
    let mut app = with_text("a<CR>b<CR>b");
    keys(&mut app, ":s/a/X/<CR>j:s/b/<lt>~>/<CR>");
    assert_eq!(text(&app), "X\n<X>\nb");
    keys(&mut app, "j&");
    assert_eq!(text(&app), "X\n<X>\n<X>", "not <<X>>");
}
