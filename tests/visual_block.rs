//! Visual block mode (`<C-v>`), end to end.

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
fn shifting_starts_at_the_blocks_left_edge() {
    let mut app = with_text("ab<CR>cd");
    keys(&mut app, "l<C-v>j>");
    assert_eq!(text(&app), "a    b\nc    d");
    assert_eq!(cursor(&app), (0, 1));
}

#[test]
fn outdent_only_takes_whitespace_inside_the_block() {
    // Six spaces between a and b; the block starts at the third.
    let mut app = with_text("a      b");
    keys(&mut app, "3l<C-v><");
    assert_eq!(text(&app), "a  b", "the two spaces left of the block stay");
    keys(&mut app, "<C-v><");
    assert_eq!(text(&app), "a  b", "nothing more to take");
}

#[test]
fn shifting_skips_rows_that_stop_short_of_the_block() {
    let mut app = with_text("abc<CR>a<CR>ab  <CR>abc");
    keys(&mut app, "ll<C-v>3j>");
    assert_eq!(text(&app), "ab    c\na\nab  \nab    c");
}

#[test]
fn shifting_with_tabs_aligns_to_tab_stops() {
    let mut app = app_with_config("[editor]\nauto-indent = false\nexpand-tab = false");
    keys(&mut app, "iabcd<Esc>0l<C-v>>");
    assert_eq!(text(&app), "a\t bcd", "a tab to column 4, then a space");
}

/// Three rows with a tab (cells 1-3) in the middle one, for blocks that cut
/// it.
fn with_a_tab() -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false\nexpand-tab = false");
    keys(&mut app, "iabcdef<CR>a\tb<CR>abcdef<Esc>gg0");
    app
}

#[test]
fn deleting_part_of_a_tab_keeps_the_rest_as_spaces() {
    let mut app = with_a_tab();
    keys(&mut app, "ll<C-v>jjld");
    assert_eq!(text(&app), "abef\na b\nabef", "b stays in line with e");
    assert_eq!(register(&app, '"').as_deref(), Some("cd\n  \ncd"));
}

#[test]
fn yanking_part_of_a_tab_gives_spaces() {
    let mut app = with_a_tab();
    keys(&mut app, "ll<C-v>jjly");
    assert_eq!(register(&app, '"').as_deref(), Some("cd\n  \ncd"));
    assert_eq!(text(&app), "abcdef\na\tb\nabcdef", "unchanged");
}

#[test]
fn inserting_in_the_middle_of_a_tab_splits_it() {
    let mut app = with_a_tab();
    keys(&mut app, "ll<C-v>jjI|<Esc>");
    assert_eq!(text(&app), "ab|cdef\na |  b\nab|cdef");

    let mut app = with_a_tab();
    keys(&mut app, "ll<C-v>jjlcX<Esc>");
    assert_eq!(text(&app), "abXef\na Xb\nabXef");
}

#[test]
fn cutting_wide_characters_leaves_spaces() {
    let mut app = with_text("abcde<CR>日本x<CR>abcde");
    keys(&mut app, "l<C-v>jjld");
    assert_eq!(text(&app), "ade\n  x\nade", "x stays in line with e");
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
fn gv_remembers_a_block_to_each_rows_end() {
    let mut app = with_text("a<CR>bbb");
    keys(&mut app, "<C-v>j$<Esc>gvA;<Esc>");
    assert_eq!(text(&app), "a;\nbbb;", "not padded to the longest row");

    keys(&mut app, "gg0<C-v>j<Esc>gvA!<Esc>");
    assert_eq!(text(&app), "a!;\nb!bb;", "a later block without $ doesn't");
}

#[test]
fn dot_repeats_a_block_insert() {
    let mut app = with_text("a<CR>b<CR>c<CR>d");
    keys(&mut app, "<C-v>jI-<Esc>jj.");
    assert_eq!(text(&app), "-a\n-b\n-c\n-d");
}
