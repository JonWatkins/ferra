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
    // As shipped: the tests' usual chrome is a change from the defaults.
    // (With VS Code's keys, as shipped, Ctrl+G opens the `:` line.)
    let mut app = common::app_as_shipped();
    keys(&mut app, "<C-g>set<CR>");
    assert_eq!(message(&app), "All options are at their defaults");
    keys(&mut app, "<C-g>set so=1 nocul<CR><C-g>set<CR>");
    assert_eq!(message(&app), "cursorline=false  scroll-off=1");
    keys(&mut app, "<C-g>set so? cul?<CR>");
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

const LONG: &str = "the quick brown fox jumps over the lazy dog and then some more words";

#[test]
fn setlocal_wrap_is_one_windows_own() {
    let mut app = app();
    keys(
        &mut app,
        &format!("i{LONG}<Esc>0:split<CR>:setlocal nowrap<CR>"),
    );
    let rows = render(&mut app, 40, 12).rows;
    // Two windows of 4 rows (with a status line each) over the command line.
    let wrapped = |row: &str| row.trim().starts_with("over") || row.contains(" over ");
    assert!(!rows[1].contains("over"), "the active window doesn't wrap");
    assert!(
        rows[6..10].iter().any(|r| wrapped(r)),
        "the other does: {rows:?}"
    );
    keys(&mut app, "<C-w>w:set wrap?<CR>");
    assert_eq!(message(&app), "wrap=true");
    keys(&mut app, "<C-w>w:set wrap?<CR>");
    assert_eq!(message(&app), "wrap=false");
}

#[test]
fn a_buffers_own_tab_width_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "x\n").unwrap();
    std::fs::write(&b, "y\n").unwrap();
    let mut app = app();
    app.open(&a);
    keys(&mut app, ":setlocal ts=2<CR>");
    keys(&mut app, &format!(":e {}<CR>:set ts?<CR>", b.display()));
    assert_eq!(message(&app), "tab-width=4", "b has the global value");
    keys(&mut app, ">>");
    assert_eq!(text(&app), "    y");
    keys(&mut app, &format!(":e {}<CR>>>", a.display()));
    assert_eq!(text(&app), "  x", "a keeps its own");
    keys(&mut app, ":setlocal<CR>");
    assert_eq!(message(&app), "tab-width=2");
}

#[test]
fn set_drops_the_local_value_and_setglobal_keeps_it() {
    let mut app = app();
    keys(
        &mut app,
        ":setlocal ts=2<CR>:setglobal ts=6<CR>:set ts?<CR>",
    );
    assert_eq!(message(&app), "tab-width=2", "its own value still");
    keys(&mut app, ":setglobal ts?<CR>");
    assert_eq!(message(&app), "tab-width=6");
    keys(&mut app, ":set ts=8<CR>:setlocal<CR>");
    assert_eq!(message(&app), "No options set for this window or buffer");
    keys(&mut app, ":set ts?<CR>");
    assert_eq!(message(&app), "tab-width=8");
}

#[test]
fn global_only_options_are_refused_locally() {
    let mut app = app();
    keys(&mut app, ":setlocal clipboard=unnamed<CR>");
    assert!(
        message(&app).starts_with("clipboard is the same everywhere"),
        "{}",
        message(&app)
    );
    assert_eq!(
        app.editor().options.clipboard,
        ferra::editor::registers::ClipboardMode::Registers
    );
}

#[test]
fn a_split_starts_with_its_windows_own_options() {
    let mut app = app();
    keys(&mut app, "ia<Esc>:setlocal nonu<CR>:split<CR>:set nu?<CR>");
    assert_eq!(message(&app), "line-numbers=none");
    let rows = render(&mut app, 40, 12).rows;
    assert!(
        rows[0].starts_with('a') && rows[6].starts_with('a'),
        "{rows:?}"
    );
}
