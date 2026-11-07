//! `:registers` / `<Space>r`, and pasting the read-only registers `"/`,
//! `":` and `"%`, end to end.

mod common;

use std::fs;

use common::{app_with_config, keys, text};
use ferra::App;
use ferra::editor::Mode;

fn listed(app: &App) -> Vec<String> {
    let picker = app.editor().picker().expect("picker open");
    picker
        .matches()
        .iter()
        .map(|m| picker.item(m).label.clone())
        .collect()
}

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
fn lists_registers_in_order() {
    let mut app = with_text("alpha beta<CR>gamma");
    keys(&mut app, "\"ayiwwdwjdd");
    keys(&mut app, ":registers<CR>");
    assert_eq!(app.editor().mode(), Mode::Picker);
    assert_eq!(
        listed(&app),
        [
            "\"\"  gamma",
            "\"1  gamma",
            "\"a  alpha",
            "\"-  beta",
            "\".  alpha beta⏎gamma",
            "\":  registers",
        ],
    );
}

#[test]
fn typing_filters_by_name_or_content_and_enter_pastes() {
    let mut app = with_text("keep<CR>other");
    keys(&mut app, "\"kyyjdd");
    keys(&mut app, "<Space>r");
    keys(&mut app, "keep<CR>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(text(&app), "keep\nkeep", "pasted \"k below the line");
    keys(&mut app, "u");
    assert_eq!(text(&app), "keep", "one undo step");
}

#[test]
fn escape_closes_without_pasting() {
    let mut app = with_text("abc");
    keys(&mut app, "yy:reg<CR><Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(text(&app), "abc");
}

#[test]
fn empty_registers_say_so() {
    let mut app = app_with_config("");
    keys(&mut app, "<Space>r");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(message(&app), "All registers are empty");
}

#[test]
fn last_search_and_command_can_be_pasted() {
    let mut app = with_text("find me");
    keys(&mut app, "/me<CR>");
    keys(&mut app, "o<Esc>\"/p");
    assert_eq!(text(&app), "find me\nme");
    keys(&mut app, ":noh<CR>o<Esc>\":p");
    assert_eq!(text(&app), "find me\nme\nnoh");
}

#[test]
fn the_file_name_can_be_pasted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, "").unwrap();
    let mut app = app_with_config("");
    app.open(&path);
    keys(&mut app, "\"%p");
    assert!(text(&app).ends_with("notes.txt"), "{}", text(&app));
}

#[test]
fn read_only_registers_cant_be_written() {
    let mut app = with_text("abc");
    keys(&mut app, "\"/yy");
    assert_eq!(message(&app), "Invalid register: \"/");
}

#[test]
fn long_and_multi_line_text_is_previewed_on_one_line() {
    let mut app = with_text("one<CR>two");
    keys(&mut app, "yj<Space>r");
    let picker = app.editor().picker().unwrap();
    let first = picker.selected_item().unwrap();
    assert_eq!(first.label, "\"\"  one⏎two");
    assert_eq!(first.detail, "2 lines");
}
