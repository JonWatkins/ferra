//! `shift-width`: a level of indentation narrower than a tab (Vim's
//! `shiftwidth`), so with tabs 8 wide and levels of 4 the levels are made
//! of tabs and spaces: 4 spaces, a tab, a tab and 4 spaces... 0 (the
//! default) makes a level `tab-width` wide.

mod common;

use std::fs;

use common::{app_with_config, cursor, keys, text};
use ferra::App;

/// Tabs 8 wide, levels of 4, indenting with tabs.
const MIXED: &str = "[editor]\ntab-width = 8\nshift-width = 4\nexpand-tab = false";

fn open_with(name: &str, content: &str, config: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    let mut app = app_with_config(config);
    app.open(&path);
    (dir, app)
}

#[test]
fn shifting_steps_a_level_made_of_tabs_and_spaces() {
    let mut app = app_with_config(MIXED);
    keys(&mut app, "ia<Esc>>>");
    assert_eq!(text(&app), "    a");
    keys(&mut app, ">>");
    assert_eq!(text(&app), "\ta", "two levels: one tab");
    keys(&mut app, ">>");
    assert_eq!(text(&app), "\t    a");
    keys(&mut app, "<<<<");
    assert_eq!(text(&app), "    a");
}

#[test]
fn tab_in_insert_mode_goes_to_the_next_level() {
    let mut app = app_with_config(MIXED);
    keys(&mut app, "i<Tab>");
    assert_eq!(text(&app), "    ");
    keys(&mut app, "<Tab>");
    assert_eq!(text(&app), "\t", "the spaces become a tab");
    keys(&mut app, "<Tab>x");
    assert_eq!(text(&app), "\t    x");
    assert_eq!(cursor(&app), (0, 6));
}

#[test]
fn tab_after_text_goes_to_the_next_level_stop() {
    let mut app = app_with_config(MIXED);
    keys(&mut app, "iab<Tab>");
    assert_eq!(text(&app), "ab  ", "to column 4");
    keys(&mut app, "<Tab>");
    assert_eq!(text(&app), "ab\t", "to column 8");
}

#[test]
fn insert_mode_shifts_use_the_level() {
    let mut app = app_with_config(MIXED);
    keys(&mut app, "ix<C-t>");
    assert_eq!(text(&app), "    x");
    keys(&mut app, "<C-t><C-t>");
    assert_eq!(text(&app), "\t    x");
    keys(&mut app, "<C-d>");
    assert_eq!(text(&app), "\tx");
    assert_eq!(cursor(&app), (0, 2), "still after `x`");
}

#[test]
fn a_new_line_after_a_bracket_is_a_level_deeper() {
    let mut app = app_with_config(MIXED);
    // Two Tabs make two levels: one tab.
    keys(&mut app, "i<Tab><Tab>if {<CR>");
    assert_eq!(text(&app), "\tif {\n\t    ");
    keys(&mut app, "x<CR>}");
    assert_eq!(
        text(&app),
        "\tif {\n\t    x\n\t}",
        "`}}` first on a line goes a level back"
    );
}

#[test]
fn rules_indent_by_the_level() {
    let (_dir, mut app) = open_with("a.rs", "fn f() {\nif a {\nb();\n}\n}", MIXED);
    keys(&mut app, "=G");
    assert_eq!(text(&app), "fn f() {\n    if a {\n\tb();\n    }\n}");
}

#[test]
fn with_spaces_a_level_is_shift_width_spaces() {
    let mut app = app_with_config("[editor]\ntab-width = 8\nshift-width = 2");
    keys(&mut app, "ia<Esc>>>");
    assert_eq!(text(&app), "  a");
    keys(&mut app, "o<Tab>b");
    assert_eq!(text(&app), "  a\n    b");
}

#[test]
fn set_shift_width_by_vims_names() {
    let mut app = app_with_config("[editor]\nexpand-tab = false\ntab-width = 8");
    keys(&mut app, "ia<Esc>:set sw=4<CR>>>");
    assert_eq!(text(&app), "    a");
    keys(&mut app, ":setlocal shiftwidth=0<CR>>>");
    assert_eq!(text(&app), "\t    a", "0: a level is a tab wide");
}
