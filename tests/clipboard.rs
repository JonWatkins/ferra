//! The system clipboard (`"+`, `"*`, the `clipboard` option) and pasting
//! into the terminal, end to end. An in-memory clipboard stands in for the
//! system one, so these tests never touch the real clipboard.

mod common;

use common::{app_with_config, cursor, keys, text};
use terminal_editor::App;
use terminal_editor::clipboard::{Clipboard, MemoryClipboard};
use terminal_editor::editor::Mode;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// An app using an in-memory clipboard, with `content` typed in (no
/// auto-indent) and the cursor at the start.
fn with_clipboard(config: &str, content: &str) -> (App, MemoryClipboard) {
    let clipboard = MemoryClipboard::default();
    let config = format!("[editor]\nauto-indent = false\n{config}");
    let mut app = app_with_config(&config);
    app.use_clipboard(Box::new(clipboard.clone()));
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    (app, clipboard)
}

#[test]
fn plus_yank_copies_to_the_clipboard() {
    let (mut app, clipboard) = with_clipboard("", "first<CR>second");
    keys(&mut app, "\"+yy");
    assert_eq!(
        clipboard.contents(),
        "first\n",
        "a line ends with a line break"
    );
    keys(&mut app, "j\"*yiw");
    assert_eq!(clipboard.contents(), "second");
}

#[test]
fn plus_paste_brings_text_in() {
    let (mut app, clipboard) = with_clipboard("", "a<CR>b");
    clipboard.set_contents("from outside");
    keys(&mut app, "$\"+p");
    assert_eq!(text(&app), "afrom outside\nb");

    clipboard.set_contents("whole line\n");
    keys(&mut app, "\"+p");
    assert_eq!(
        text(&app),
        "afrom outside\nwhole line\nb",
        "pasted below, as a line"
    );
}

#[test]
fn plain_yanks_stay_internal_by_default() {
    let (mut app, clipboard) = with_clipboard("", "abc");
    keys(&mut app, "yy");
    assert_eq!(clipboard.contents(), "");
}

#[test]
fn unnamed_mode_shares_every_yank_and_paste() {
    let (mut app, clipboard) = with_clipboard("clipboard = \"unnamed\"", "one<CR>two");
    keys(&mut app, "dd");
    assert_eq!(clipboard.contents(), "one\n");
    clipboard.set_contents("pasted ");
    keys(&mut app, "P");
    assert_eq!(text(&app), "pasted two");
}

#[test]
fn off_mode_never_touches_the_clipboard() {
    let (mut app, clipboard) = with_clipboard("clipboard = \"off\"", "abc");
    keys(&mut app, "\"+yy\"+p");
    assert_eq!(clipboard.contents(), "");
    assert_eq!(text(&app), "abc\nabc", "\"+ still works as a register");
}

#[test]
fn visual_selection_to_the_clipboard() {
    let (mut app, clipboard) = with_clipboard("", "copy this part");
    keys(&mut app, "wve\"+y");
    assert_eq!(clipboard.contents(), "this");
}

struct BrokenClipboard;

impl Clipboard for BrokenClipboard {
    fn get(&mut self) -> Result<String, String> {
        Err("Clipboard: no display".into())
    }
    fn set(&mut self, _: &str) -> Result<(), String> {
        Err("Clipboard: no display".into())
    }
}

#[test]
fn a_broken_clipboard_reports_but_never_loses_the_text() {
    let mut app = app_with_config("");
    app.use_clipboard(Box::new(BrokenClipboard));
    keys(&mut app, "ikeep<Esc>\"+dd");
    assert_eq!(message(&app), "Clipboard: no display");
    assert_eq!(text(&app), "", "the delete still happened");
    keys(&mut app, "p");
    assert_eq!(
        text(&app),
        "\nkeep",
        "and the text is in the unnamed register"
    );
    keys(&mut app, "\"+p");
    assert_eq!(message(&app), "Clipboard: no display");
}

#[test]
fn pasting_in_insert_mode_keeps_the_text_exactly() {
    let mut app = app_with_config(""); // auto-indent on
    keys(&mut app, "ifn x() {<CR>");
    app.handle_paste("if a {\n\tb();\n}\n");
    keys(&mut app, "<Esc>");
    assert_eq!(
        text(&app),
        "fn x() {\n    if a {\n\tb();\n}\n",
        "no auto-indent applied to the pasted lines"
    );
    keys(&mut app, "u");
    assert_eq!(text(&app), "", "the typing and the paste undo together");
}

#[test]
fn a_paste_with_its_own_indent_replaces_the_auto_indent() {
    let mut app = app_with_config(""); // auto-indent on
    keys(&mut app, "ifn x() {<Esc>o");
    app.handle_paste("    let y = 1;");
    assert_eq!(text(&app), "fn x() {\n    let y = 1;", "not 8 spaces");

    let mut app = app_with_config("");
    keys(&mut app, "ifn x() {<Esc>o");
    app.handle_paste("let y = 1;");
    assert_eq!(
        text(&app),
        "fn x() {\n    let y = 1;",
        "unindented paste keeps the auto-indent"
    );
}

#[test]
fn pasting_in_normal_mode_inserts_before_the_cursor() {
    let (mut app, _) = with_clipboard("", "ab");
    keys(&mut app, "l");
    app.handle_paste("XY");
    assert_eq!(text(&app), "aXYb");
    assert_eq!(cursor(&app), (0, 2), "on the last pasted character");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "u");
    assert_eq!(text(&app), "ab", "one undo step");
}

#[test]
fn pasting_normalises_line_endings() {
    let (mut app, _) = with_clipboard("", "");
    app.handle_paste("one\r\ntwo\rthree");
    assert_eq!(text(&app), "one\ntwo\nthree");
}

#[test]
fn pasting_into_the_prompt_takes_the_first_line() {
    let (mut app, _) = with_clipboard("", "abc");
    keys(&mut app, ":");
    app.handle_paste("goto_line 1\nrm -rf /");
    assert_eq!(app.editor().command_line.input(), "goto_line 1");
}
