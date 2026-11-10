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

#[test]
fn todos_lists_the_files_todos_in_a_popup_and_goes_to_one() {
    let mut app = common::app_with_config("[editor]\nauto-indent = false");
    let source = fs::read_to_string("examples/plugins/todos.lua").unwrap();
    app.load_plugin("todos.lua", &source)
        .expect("todos.lua loads");
    keys(
        &mut app,
        "ifn a() {}\n  // TODO: first\nlet x = 1;\n  # FIXME broken\nlet URL = 1; // XXX here\n<Esc>gg0",
    );
    keys(&mut app, "<Space>T");
    let popup = app.editor().popup().expect("a popup");
    assert_eq!(popup.title, "TODOs");
    assert_eq!(popup.lines.len(), 3, "not fooled by URL");
    let texts: Vec<String> = popup.lines[1].iter().map(|s| s.text.clone()).collect();
    assert_eq!(texts.concat(), "   4  FIXME  broken");
    keys(&mut app, "j<CR>");
    assert!(app.editor().popup().is_none());
    assert_eq!(cursor(&app), (3, 2), "at the FIXME's line, past its indent");

    keys(&mut app, "ggdG<Space>T");
    assert_eq!(message(&app), "No TODOs in this file");
}

#[test]
fn todos_has_a_sidebar_view_that_follows_edits() {
    let mut app = common::app_with_config("[editor]\nauto-indent = false");
    let source = fs::read_to_string("examples/plugins/todos.lua").unwrap();
    app.load_plugin("todos.lua", &source)
        .expect("todos.lua loads");
    keys(&mut app, "ia\n// TODO: one\nb<Esc>");
    keys(&mut app, ":sidebar_view todos<CR>");
    let shown = common::render(&mut app, 70, 10).rows.join("\n");
    assert!(shown.contains("TODO  one"), "{shown}");

    keys(&mut app, "<Space>e");
    keys(&mut app, "Go// FIXME: two<Esc>");
    let shown = common::render(&mut app, 70, 10).rows.join("\n");
    assert!(
        shown.contains("FIXME  two"),
        "made again after the edit: {shown}"
    );

    keys(&mut app, "<Space>ej<CR>");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Normal);
    assert_eq!(cursor(&app), (3, 0), "at the FIXME");
}

#[test]
fn snippets_picks_a_snippet_and_asks_what_to_wrap_a_word_in() {
    let mut app = common::app_with_config("[editor]\nauto-indent = false");
    let source = fs::read_to_string("examples/plugins/snippets.lua").unwrap();
    app.load_plugin("snippets.lua", &source)
        .expect("snippets.lua loads");
    keys(&mut app, "<Space>itod<CR>");
    assert_eq!(text(&app), "// TODO: ");

    keys(&mut app, "ccsome word here<Esc>bb");
    keys(&mut app, "<Space>w**<CR>");
    assert_eq!(text(&app), "some **word** here");
    keys(&mut app, "0<Space>w<Esc>");
    assert_eq!(text(&app), "some **word** here", "cancelled");
}

#[test]
fn tidy_marks_trailing_whitespace_and_long_lines() {
    let mut app = common::app_with_config("[editor]\nauto-indent = false\ntext-width = 20");
    let source = fs::read_to_string("examples/plugins/tidy.lua").unwrap();
    app.load_plugin("tidy.lua", &source)
        .expect("tidy.lua loads");
    keys(
        &mut app,
        "itrailing   \nthis line is longer than twenty\nfine<Esc>",
    );
    let doc = &app.editor().document;
    let spaces: Vec<_> = doc
        .highlights_on(0)
        .map(|h| (h.start.col, h.end.col, h.style.clone()))
        .collect();
    assert_eq!(spaces, [(8, 11, "ui.decoration.error".to_string())]);
    let notes: Vec<_> = doc.end_of_line_text(1).map(|v| v.text.clone()).collect();
    assert_eq!(notes, ["← 11 over 20"], "31 chars");
    assert_eq!(doc.highlights_on(2).count(), 0);

    // Fixed as you type: the marks follow.
    keys(&mut app, "ggA<BS><BS><BS><Esc>");
    assert_eq!(app.editor().document.highlights_on(0).count(), 0);

    keys(&mut app, ":tidy<CR>");
    assert_eq!(message(&app), "Tidy marks off");
    assert_eq!(app.editor().document.end_of_line_text(1).count(), 0);
}
