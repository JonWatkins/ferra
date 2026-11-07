//! `:set`, end to end: changes take effect at once.

mod common;

use common::{app, app_with_config, keys, render, text};
use ferra::App;
use ferra::clipboard::MemoryClipboard;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn command_line(app: &App) -> String {
    app.editor().command_line.input().to_string()
}

#[test]
fn a_change_takes_effect_at_once_and_is_shown() {
    let mut app = app();
    keys(&mut app, "ia<Esc>:set ts=2<CR>");
    assert_eq!(message(&app), "tab-width=2");
    keys(&mut app, ">>");
    assert_eq!(text(&app), "  a");
    keys(&mut app, ":set noexpand-tab<CR>>>");
    assert_eq!(text(&app), "\t\ta", "two levels of two columns: a tab");
}

#[test]
fn vim_switches_redraw_the_gutter() {
    let mut app = app();
    keys(&mut app, "ione<CR>two<Esc>");
    keys(&mut app, ":set nonu<CR>");
    assert!(render(&mut app, 30, 5).rows[0].starts_with("one"));
    keys(&mut app, ":set nu<CR>");
    let rows = render(&mut app, 30, 5).rows;
    assert!(rows[0].trim_start().starts_with("1 one"), "{rows:?}");
    assert_eq!(message(&app), "line-numbers=absolute");
}

#[test]
fn plain_set_lists_what_changed() {
    let mut app = app();
    keys(&mut app, ":set<CR>");
    assert_eq!(message(&app), "All options are at their defaults");
    keys(&mut app, ":set so=1 nocul<CR>:set<CR>");
    assert_eq!(message(&app), "cursorline=false  scroll-off=1");
    keys(&mut app, ":set so? cul?<CR>");
    assert_eq!(message(&app), "scroll-off=1  cursorline=false");
}

#[test]
fn mistakes_are_reported_and_change_nothing() {
    let mut app = app();
    keys(&mut app, ":set ts=8 bogus<CR>");
    assert_eq!(message(&app), "Unknown option: bogus");
    assert_eq!(app.editor().options.tab_width, 4);
}

#[test]
fn the_clipboard_option_applies_to_the_registers() {
    let clipboard = MemoryClipboard::default();
    let mut app = app_with_config("");
    app.use_clipboard(Box::new(clipboard.clone()));
    keys(&mut app, "iword<Esc>yiw");
    assert_eq!(clipboard.contents(), "", "only \"+ by default");
    keys(&mut app, ":set clipboard=unnamed<CR>yiw");
    assert_eq!(clipboard.contents(), "word");
}

#[test]
fn option_names_complete() {
    let mut app = app();
    keys(&mut app, ":set expa<Tab>");
    assert_eq!(command_line(&app), "set expand-tab");
    keys(&mut app, "<Esc>:set noexpa<Tab>");
    assert_eq!(command_line(&app), "set noexpand-tab");
    keys(&mut app, "<Esc>:set search.c<Tab>");
    assert_eq!(command_line(&app), "set search.case");
}

#[test]
fn kitty_keyboard_waits_for_the_next_start() {
    let mut app = app();
    keys(&mut app, ":set nokitty-keyboard<CR>");
    assert_eq!(message(&app), "kitty-keyboard=false (from the next start)");
}
