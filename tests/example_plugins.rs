//! The example plugins in `examples/plugins/` load and work, so they can't
//! silently fall behind the plugin API.

mod common;

use std::fs;
use std::path::Path;

use common::{app, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// An app with `toolkit.lua` loaded and `content` typed in (auto-indent off,
/// so the text is exactly as written), cursor at the start.
fn with_toolkit(content: &str) -> App {
    let mut app = common::app_with_config("[editor]\nauto-indent = false");
    let source = fs::read_to_string("examples/plugins/toolkit.lua").unwrap();
    app.load_plugin("toolkit.lua", &source)
        .expect("toolkit.lua loads");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn every_example_plugin_loads() {
    let mut count = 0;
    for entry in fs::read_dir("examples/plugins").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "lua") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = fs::read_to_string(&path).unwrap();
            app()
                .load_plugin(&name, &source)
                .unwrap_or_else(|e| panic!("{e}"));
            count += 1;
        }
    }
    assert!(count > 0, "no example plugins found");
}

#[test]
fn todo_inserts_an_indented_comment_ready_to_type() {
    let mut app = with_toolkit("fn main() {<CR>    body();<CR>}");
    keys(&mut app, "j<Space>tfix this<Esc>");
    assert_eq!(
        text(&app),
        "fn main() {\n    // TODO: fix this\n    body();\n}"
    );

    keys(&mut app, ":todo #<CR>");
    assert!(text(&app).contains("    # TODO: \n"), "{}", text(&app));
}

#[test]
fn duplicate_line_with_count() {
    let mut app = with_toolkit("one<CR>two");
    keys(&mut app, "l2<Space>d");
    assert_eq!(text(&app), "one\none\none\ntwo");
    assert_eq!(cursor(&app), (1, 1), "on the first copy, same column");
    keys(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo", "one undo step");
}

#[test]
fn sort_paragraph_sorts_only_the_cursors_paragraph() {
    let mut app = with_toolkit("c<CR>a<CR>b<CR><CR>z<CR>y");
    keys(&mut app, "j<Space>s");
    assert_eq!(text(&app), "a\nb\nc\n\nz\ny");
    assert_eq!(message(&app), "Sorted 3 lines");

    keys(&mut app, "4G<Space>s");
    assert_eq!(message(&app), "not in a paragraph");
}

#[test]
fn word_count_and_date() {
    let mut app = with_toolkit("hello big<CR>world");
    keys(&mut app, ":word_count<CR>");
    assert_eq!(message(&app), "2 lines, 3 words, 15 chars");

    // `o` opens a line below the first, so the date lands on line 2.
    keys(&mut app, "o<Esc>:insert_date<CR>");
    let date = app.editor().document.buffer().line(1).into_owned();
    let is_date = date.len() == 10
        && date.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    assert!(is_date, "{date:?}");
}

#[test]
fn after_save_hook_reports_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    fs::write(&path, "a\nb\n").unwrap();
    let mut app = with_toolkit("");
    app.open(Path::new(&path));
    keys(&mut app, ":w<CR>");
    assert_eq!(message(&app), "Saved notes.md (2 lines)");
}
