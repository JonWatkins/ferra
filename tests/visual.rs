//! Visual mode end-to-end: selecting with motions and text objects, then
//! operating on the selection.

mod common;

use common::{app, cursor, keys, render, text};
use ferra::App;
use ferra::editor::Mode;
use ratatui::style::Color;

/// An app containing `content`, with the cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn v_selects_characters_and_d_deletes_them() {
    let mut app = with_text("hello world");
    keys(&mut app, "vlll");
    assert_eq!(app.editor().mode(), Mode::Visual);
    keys(&mut app, "d");
    assert_eq!(text(&app), "o world");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn selection_works_backwards() {
    let mut app = with_text("hello world");
    keys(&mut app, "$vbd");
    assert_eq!(text(&app), "hello ");
}

#[test]
fn counts_apply_to_motions() {
    let mut app = with_text("abcdefgh");
    keys(&mut app, "v3lx");
    assert_eq!(text(&app), "efgh");
}

#[test]
fn char_selection_across_lines() {
    let mut app = with_text("one<CR>two<CR>three");
    keys(&mut app, "lvjd");
    assert_eq!(text(&app), "oo\nthree");
}

#[test]
fn capital_v_selects_lines() {
    let mut app = with_text("one<CR>two<CR>three<CR>four");
    keys(&mut app, "jVjd");
    assert_eq!(text(&app), "one\nfour");
}

#[test]
fn text_objects_select() {
    let mut app = with_text("call(alpha, beta)");
    // `ww` moves inside the parentheses, onto `alpha`.
    keys(&mut app, "wwvi(c");
    assert_eq!(text(&app), "call()");
    assert_eq!(app.editor().mode(), Mode::Insert);
}

#[test]
fn o_swaps_ends_to_extend_the_other_way() {
    let mut app = with_text("abcdef");
    keys(&mut app, "lllvlohhd");
    assert_eq!(text(&app), "af");
}

#[test]
fn yank_selection_and_paste() {
    let mut app = with_text("copy me");
    keys(&mut app, "vey$p");
    assert_eq!(text(&app), "copy mecopy");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn line_shortcuts_in_char_selection() {
    let mut app = with_text("one<CR>two<CR>three");
    keys(&mut app, "lvjD");
    assert_eq!(text(&app), "three");

    let mut app = with_text("one<CR>two");
    keys(&mut app, "vYjp");
    assert_eq!(text(&app), "one\ntwo\none");
}

#[test]
fn paste_over_selection_swaps_words() {
    let mut app = with_text("left right");
    keys(&mut app, "yiw");
    keys(&mut app, "wviwp");
    assert_eq!(text(&app), "left left");
    keys(&mut app, "0viwp");
    assert_eq!(text(&app), "right left");
}

#[test]
fn named_register_in_visual_mode() {
    let mut app = with_text("keep this");
    keys(&mut app, "viw\"ay");
    keys(&mut app, "$\"ap");
    assert_eq!(text(&app), "keep thiskeep");
}

#[test]
fn v_v_and_escape_leave_visual_mode() {
    let mut app = with_text("abc");
    keys(&mut app, "vv");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "VV");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "vl<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(text(&app), "abc");
}

#[test]
fn v_switches_to_line_selection_and_back() {
    let mut app = with_text("ab<CR>cd<CR>ef");
    keys(&mut app, "lvjVd");
    assert_eq!(text(&app), "ef");
}

#[test]
fn gv_reselects_last_selection() {
    let mut app = with_text("one two three");
    keys(&mut app, "vey");
    keys(&mut app, "$gvd");
    assert_eq!(text(&app), " two three");
}

#[test]
fn visual_delete_is_one_undo_step() {
    let mut app = with_text("one<CR>two<CR>three");
    keys(&mut app, "Vjd");
    keys(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo\nthree");
    assert_eq!(cursor(&app).0, 0);
}

#[test]
fn unbound_key_keeps_selection() {
    let mut app = with_text("abc");
    keys(&mut app, "vlz");
    assert_eq!(app.editor().mode(), Mode::Visual);
    keys(&mut app, "d");
    assert_eq!(text(&app), "c");
}

#[test]
fn selection_is_highlighted_and_labelled() {
    let mut app = with_text("select me");
    keys(&mut app, "vll");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 5)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();

    // Gutter is 5 cells wide, so the text starts at x = 5.
    let selection_bg = Color::Rgb(0x33, 0x46, 0x7c);
    let highlighted: Vec<u16> = (0..30)
        .filter(|&x| buffer[(x, 0)].bg == selection_bg)
        .collect();
    assert_eq!(highlighted, vec![5, 6, 7]);

    let screen = render(&mut app, 30, 5);
    assert!(
        screen.rows[3].starts_with(" VISUAL "),
        "{:?}",
        screen.rows[3]
    );
    keys(&mut app, "V");
    let screen = render(&mut app, 30, 5);
    assert!(
        screen.rows[3].starts_with(" V-LINE "),
        "{:?}",
        screen.rows[3]
    );
}
