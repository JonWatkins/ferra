//! `<C-r>{register}` while typing, and the `".` register (the last
//! inserted text), end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use ferra::App;
use ferra::editor::Mode;

fn register(app: &App, name: char) -> Option<String> {
    app.editor()
        .registers
        .get(Some(name))
        .map(|r| r.text.clone())
}

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn insert_a_register_while_typing() {
    let mut app = with_text("hello world");
    keys(&mut app, "yiwA <C-r>\"!<Esc>");
    assert_eq!(text(&app), "hello world hello!");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn a_line_register_brings_its_line_break() {
    let mut app = with_text("first<CR>second");
    keys(&mut app, "yyjA<CR><C-r>\"<Esc>");
    assert_eq!(text(&app), "first\nsecond\nfirst\n");
}

#[test]
fn inserted_text_isnt_auto_indented() {
    let mut app = app_with_config("");
    keys(&mut app, "ia<CR>b<Esc>ggyj"); // "a\nb", linewise
    keys(&mut app, "Gofn x() {<CR><C-r>\"<Esc>");
    assert_eq!(
        text(&app),
        "a\nb\nfn x() {\n    a\nb\n",
        "only the first line follows the indent"
    );
}

#[test]
fn read_only_registers_work_too() {
    let mut app = with_text("find me");
    keys(&mut app, "/me<CR>o<C-r>/<Esc>");
    assert_eq!(text(&app), "find me\nme");
}

#[test]
fn an_empty_register_says_so_and_typing_goes_on() {
    let mut app = with_text("");
    keys(&mut app, "i<C-r>q");
    let message = app.editor().message().map(|m| m.text.clone());
    assert_eq!(message.as_deref(), Some("Register q is empty"));
    assert_eq!(app.editor().mode(), Mode::Insert);
    keys(&mut app, "x<Esc>");
    assert_eq!(text(&app), "x");
}

#[test]
fn escape_after_ctrl_r_cancels_it() {
    let mut app = with_text("");
    keys(&mut app, "iab<C-r><Esc>");
    assert_eq!(text(&app), "ab");
}

#[test]
fn search_and_command_lines_take_the_first_line() {
    let mut app = with_text("one two<CR>two");
    keys(&mut app, "wyiwgg0/<C-r>\"<CR>");
    assert_eq!(cursor(&app), (0, 4), "searched for the yanked word");

    let mut app = with_text("2");
    keys(&mut app, "yy:<C-r>\"<CR>");
    assert_eq!(app.editor().mode(), Mode::Normal, ":2 ran");
}

#[test]
fn dot_register_holds_the_last_insert() {
    let mut app = with_text("");
    keys(&mut app, "ihello<CR>wor<BS><BS>orld<Esc>");
    assert_eq!(
        register(&app, '.').as_deref(),
        Some("hello\nworld"),
        "backspaces undone"
    );
    keys(&mut app, "o<C-r>.<Esc>");
    assert_eq!(text(&app), "hello\nworld\nhello\nworld");
    assert_eq!(
        register(&app, '.').as_deref(),
        Some("hello\nworld"),
        "what <C-r>. inserted"
    );
}

#[test]
fn dot_register_is_read_only_and_pasteable() {
    let mut app = with_text("");
    keys(&mut app, "iabc<Esc>\".p");
    assert_eq!(text(&app), "abcabc");
    keys(&mut app, "\".yy");
    let message = app.editor().message().map(|m| m.text.clone());
    assert_eq!(message.as_deref(), Some("Invalid register: \"."));
}

#[test]
fn an_insert_that_types_nothing_keeps_the_previous_one() {
    let mut app = with_text("");
    keys(&mut app, "iabc<Esc>i<Esc>");
    assert_eq!(register(&app, '.').as_deref(), Some("abc"));
}
