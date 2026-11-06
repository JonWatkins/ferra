//! Visual block mode (`<C-v>`), end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use terminal_editor::App;
use terminal_editor::editor::Mode;

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
fn delete_a_column() {
    let mut app = with_text("abcd<CR>efgh<CR>ijkl");
    keys(&mut app, "l<C-v>jjld");
    assert_eq!(text(&app), "ad\neh\nil");
    assert_eq!(cursor(&app), (0, 1));
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn yank_and_paste_a_block_elsewhere() {
    let mut app = with_text("ab12<CR>cd34");
    keys(&mut app, "ll<C-v>jly");
    assert_eq!(register(&app, '"').as_deref(), Some("12\n34"));
    keys(&mut app, "0P");
    assert_eq!(text(&app), "12ab12\n34cd34");
}

#[test]
fn insert_on_every_row() {
    let mut app = with_text("one<CR>two<CR>three");
    keys(&mut app, "<C-v>jjI// <Esc>");
    assert_eq!(text(&app), "// one\n// two\n// three");
    assert_eq!(cursor(&app), (0, 0));
    keys(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo\nthree", "one undo step");
}

#[test]
fn append_to_every_row_end_with_dollar() {
    let mut app = with_text("a<CR>bbb<CR>cc");
    keys(&mut app, "<C-v>jj$A;<Esc>");
    assert_eq!(text(&app), "a;\nbbb;\ncc;");
}

#[test]
fn append_after_the_block_pads_short_rows() {
    let mut app = with_text("abc<CR>a<CR>abc");
    keys(&mut app, "l<C-v>jjlA|<Esc>");
    assert_eq!(text(&app), "abc|\na  |\nabc|");
}

#[test]
fn change_a_column() {
    let mut app = with_text("let a = 1;<CR>let b = 2;");
    keys(&mut app, "4l<C-v>jcx<Esc>");
    assert_eq!(text(&app), "let x = 1;\nlet x = 2;");
}

#[test]
fn capital_d_and_c_go_to_each_row_end() {
    let mut app = with_text("abcd<CR>abcdef");
    keys(&mut app, "l<C-v>jD");
    assert_eq!(text(&app), "a\na");

    let mut app = with_text("abcd<CR>abcdef");
    keys(&mut app, "ll<C-v>jCZ<Esc>");
    assert_eq!(text(&app), "abZ\nabZ");
}

#[test]
fn case_operators_touch_only_the_block() {
    let mut app = with_text("abcd<CR>abcd");
    keys(&mut app, "l<C-v>jlU");
    assert_eq!(text(&app), "aBCd\naBCd");
}

#[test]
fn indent_shifts_the_blocks_lines() {
    let mut app = with_text("a<CR>b");
    keys(&mut app, "<C-v>j>");
    assert_eq!(text(&app), "    a\n    b");
}

#[test]
fn line_commands_see_whole_lines() {
    let mut app = with_text("a1<CR>b2<CR>c3");
    keys(&mut app, "l<C-v>jJ");
    assert_eq!(text(&app), "a1 b2\nc3");

    let mut app = with_text("a1<CR>b2<CR>c3");
    keys(&mut app, "l<C-v>jX");
    assert_eq!(text(&app), "c3");
}

#[test]
fn o_and_capital_o_move_between_corners() {
    let mut app = with_text("abcd<CR>abcd");
    keys(&mut app, "<C-v>jll");
    assert_eq!(cursor(&app), (1, 2));
    keys(&mut app, "O");
    assert_eq!(cursor(&app), (1, 0), "other side, same row");
    keys(&mut app, "o");
    assert_eq!(cursor(&app), (0, 2), "opposite corner");
    keys(&mut app, "d");
    assert_eq!(text(&app), "d\nd");
}

#[test]
fn switching_between_visual_kinds() {
    let mut app = with_text("abc<CR>abc");
    keys(&mut app, "v<C-v>");
    assert_eq!(app.editor().mode(), Mode::Visual);
    keys(&mut app, "<C-v>");
    assert_eq!(app.editor().mode(), Mode::Normal, "<C-v> again leaves");
    keys(&mut app, "<C-v>jlV");
    keys(&mut app, "d");
    assert_eq!(text(&app), "", "V turned the block into lines");
}

#[test]
fn tabs_are_measured_in_display_columns() {
    let mut app = app_with_config("[editor]\nauto-indent = false\nexpand-tab = false");
    keys(&mut app, "i\tx<CR>abcdx<Esc>gg0");
    // The tab covers columns 0-3, so selecting it and going down takes
    // the first four characters of the next line.
    keys(&mut app, "<C-v>jd");
    assert_eq!(text(&app), "x\nx");
}

#[test]
fn a_one_line_register_fills_every_row() {
    let mut app = with_text("a-b<CR>c-d");
    keys(&mut app, "yl"); // "a"
    keys(&mut app, "l<C-v>jp");
    assert_eq!(text(&app), "aab\ncad");
}

#[test]
fn gv_restores_a_block() {
    let mut app = with_text("abc<CR>abc");
    keys(&mut app, "<C-v>jl<Esc>gvd");
    assert_eq!(text(&app), "c\nc");
}

#[test]
fn dot_repeats_a_block_insert() {
    let mut app = with_text("a<CR>b<CR>c<CR>d");
    keys(&mut app, "<C-v>jI-<Esc>jj.");
    assert_eq!(text(&app), "-a\n-b\n-c\n-d");
}
