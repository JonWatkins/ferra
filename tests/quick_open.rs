//! The quick-open box (Ctrl+P, the file picker): `>` first switches it to
//! commands, `@` to the file's symbols, `:` to a line, as in VS Code;
//! Backspace on the `>` goes back to files.

mod common;

use common::{app, cursor, keys, render};
use ferra::App;
use ferra::editor::Mode;

fn picker(app: &App) -> (String, Option<char>, String) {
    let picker = app.editor().picker().expect("a picker");
    (
        picker.title().to_string(),
        picker.prefix(),
        picker.query().to_string(),
    )
}

#[test]
fn greater_than_switches_to_commands_and_backspace_back() {
    let mut app = app();
    keys(&mut app, ":file_picker<CR>");
    assert_eq!(picker(&app).0, "Files");
    keys(&mut app, ">writ");
    assert_eq!(picker(&app), ("Commands".into(), Some('>'), "writ".into()));
    let shown = render(&mut app, 80, 20).rows.join("\n");
    assert!(shown.contains("> >writ"), "{shown}");
    let selected = app
        .editor()
        .picker()
        .unwrap()
        .selected_item()
        .unwrap()
        .label
        .clone();
    assert!(selected.starts_with("writ"), "{selected}");
    // Backspace the query, then the `>`: files again.
    keys(&mut app, "<BS><BS><BS><BS>");
    assert_eq!(picker(&app).1, Some('>'));
    keys(&mut app, "<BS>");
    assert_eq!(picker(&app), ("Files".into(), None, String::new()));
}

#[test]
fn colon_goes_to_a_line() {
    let mut app = app();
    keys(&mut app, "ione<CR>two<CR>three<Esc>:file_picker<CR>:");
    assert_eq!(app.editor().mode(), Mode::Input);
    assert_eq!(app.editor().input_box().unwrap().title, "Go to line");
    keys(&mut app, "2<CR>");
    assert_eq!(cursor(&app).0, 1);
}

#[test]
fn only_first_and_only_in_the_quick_open_box() {
    let mut app = app();
    keys(&mut app, ":file_picker<CR>a>");
    assert_eq!(
        picker(&app),
        ("Files".into(), None, "a>".into()),
        "typed, not switched"
    );
    // The command palette is the box with `>` in already: another `>`
    // is typed.
    keys(&mut app, "<Esc>:command_palette<CR>>");
    assert_eq!(picker(&app), ("Commands".into(), Some('>'), ">".into()));
}

#[test]
fn at_without_a_language_server_says_so() {
    let mut app = app();
    keys(&mut app, ":file_picker<CR>@");
    assert!(app.editor().picker().is_none());
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(!message.is_empty(), "says why");
    // The next picker is an ordinary one.
    keys(&mut app, ":ls<CR>");
    let picker = app.editor().picker().unwrap();
    assert!(!picker.is_quick_open() && picker.prefix().is_none());
}
