//! Split windows end to end: rendering, moving between windows, and how
//! windows interact with buffers and quitting.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{app, app_with_config, cursor, keys, render, text};
use terminal_editor::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn file(dir: &tempfile::TempDir, name: &str, contents: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, contents).unwrap();
    path
}

/// No line numbers, so screen columns are text columns.
fn plain_app() -> App {
    app_with_config("[editor]\nline-numbers = \"none\"\nauto-indent = false")
}

#[test]
fn vertical_split_shows_both_windows_with_a_separator() {
    let mut app = plain_app();
    keys(&mut app, "ileft text<Esc>:vs<CR>");
    // 21 columns: two 10-column windows and a separator.
    let screen = render(&mut app, 21, 5);
    assert_eq!(screen.rows[0], "left text │left text ");
    assert_eq!(screen.rows[1], "          │          ");
    assert_eq!(screen.cursor, (19, 0), "cursor in the new (right) window");
}

#[test]
fn horizontal_split_stacks_windows() {
    // With one global status line, stacked windows have a separator line
    // (per-window status lines replace it: see tests/statusline.rs).
    let mut app = app_with_config(
        "[editor]\nline-numbers = \"none\"\nauto-indent = false\nstatusline = \"global\"",
    );
    keys(&mut app, "itop<Esc>:sp<CR>");
    // 7 rows: 5 for windows (2 + separator + 2), statusline, command line.
    let screen = render(&mut app, 10, 7);
    assert!(screen.rows[0].starts_with("top"));
    assert_eq!(screen.rows[2], "──────────");
    assert!(screen.rows[3].starts_with("top"));
    assert_eq!(screen.cursor, (2, 3), "in the lower, new window");
}

#[test]
fn each_window_has_its_own_cursor_on_a_shared_buffer() {
    let mut app = plain_app();
    keys(&mut app, "i1<CR>2<CR>3<Esc>gg<C-w>vG");
    assert_eq!(cursor(&app), (2, 0));
    keys(&mut app, "<C-w>h");
    assert_eq!(cursor(&app), (0, 0), "the left window kept its place");
    keys(&mut app, "<C-w>l");
    assert_eq!(cursor(&app), (2, 0));
}

#[test]
fn edits_appear_in_every_window_on_the_buffer() {
    let mut app = plain_app();
    keys(&mut app, "<C-w>vishared<Esc>");
    let screen = render(&mut app, 21, 4);
    assert_eq!(screen.rows[0], "shared    │shared    ");
}

#[test]
fn windows_can_show_different_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = plain_app();
    app.open(&file(&dir, "a.txt", "alpha\n"));
    keys(
        &mut app,
        &format!(":vs {}<CR>", dir.path().join("b.txt").display()),
    );
    keys(&mut app, "ibeta<Esc>");
    let screen = render(&mut app, 21, 4);
    assert_eq!(screen.rows[0], "alpha     │beta      ");
    // The statusline follows the active window (checked at a width where
    // the long temp path isn't shortened away).
    let status = render(&mut app, 200, 4).rows[2].clone();
    assert!(status.contains("b.txt"), "{status:?}");

    keys(&mut app, "<C-w>h");
    assert_eq!(text(&app), "alpha");
    let status = render(&mut app, 200, 4).rows[2].clone();
    assert!(status.contains("a.txt"), "{status:?}");
}

#[test]
fn cursorline_and_selection_only_in_the_active_window() {
    let mut app = app_with_config("[editor]\nline-numbers = \"none\"");
    keys(&mut app, "iab<Esc><C-w>v0vl");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(21, 4)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let selection = ratatui::style::Color::Rgb(0x33, 0x46, 0x7c);
    let cursorline = ratatui::style::Color::Rgb(0x1f, 0x22, 0x29);
    assert_eq!(buffer[(11, 0)].bg, selection, "active (right) window");
    assert_ne!(buffer[(0, 0)].bg, selection, "inactive (left) window");
    assert_ne!(
        buffer[(5, 0)].bg,
        cursorline,
        "no cursor line when inactive"
    );
}

#[test]
fn moving_between_windows_by_direction_and_in_turn() {
    let mut app = plain_app();
    keys(&mut app, "<C-w>v<C-w>s");
    // [0 | 1]
    // [  | 2]  (window 2 active)
    keys(&mut app, "ithree<Esc>");
    keys(&mut app, "<C-w>k");
    assert_eq!(text(&app), "three", "all three show the same buffer");
    keys(&mut app, "<C-w>w<C-w>w");
    assert_eq!(
        app.editor().windows().active().0,
        0,
        "wrapped round to the first"
    );
    keys(&mut app, "<C-w>l");
    assert_eq!(app.editor().windows().active().0, 1);
}

#[test]
fn q_closes_windows_then_quits() {
    let mut app = plain_app();
    keys(&mut app, ":vs<CR>:sp<CR>");
    assert_eq!(app.editor().windows().count(), 3);
    keys(&mut app, ":q<CR>");
    assert_eq!(app.editor().windows().count(), 2);
    keys(&mut app, "<C-w>q");
    assert_eq!(app.editor().windows().count(), 1);
    assert!(!app.should_quit());
    keys(&mut app, ":q<CR>");
    assert!(app.should_quit(), "the last window quits");
}

#[test]
fn closing_a_window_keeps_its_unsaved_buffer() {
    let mut app = plain_app();
    keys(&mut app, ":vs<CR>iunsaved<Esc>:q<CR>");
    assert_eq!(app.editor().windows().count(), 1);
    assert_eq!(text(&app), "unsaved");
    keys(&mut app, ":q<CR>");
    assert!(!app.should_quit(), "the changes are still protected");
    keys(&mut app, ":qa!<CR>");
    assert!(app.should_quit());
}

#[test]
fn only_and_close() {
    let mut app = plain_app();
    keys(&mut app, ":vs<CR>:vs<CR>:only<CR>");
    assert_eq!(app.editor().windows().count(), 1);
    keys(&mut app, ":close<CR>");
    assert_eq!(message(&app), "Cannot close the last window");
}

#[test]
fn deleting_a_buffer_shown_elsewhere_updates_that_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = plain_app();
    app.open_all(&[
        file(&dir, "a.txt", "alpha\n"),
        file(&dir, "b.txt", "beta\n"),
    ]);
    keys(&mut app, "<C-w>v:bd<CR>");
    // Both windows showed a.txt; it's gone, so both now show b.txt.
    let screen = render(&mut app, 21, 4);
    assert_eq!(screen.rows[0], "beta      │beta      ");
}

#[test]
fn syntax_highlighting_in_every_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = plain_app();
    app.open(&file(&dir, "main.rs", "fn main() {}\n"));
    keys(&mut app, ":vs<CR>");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(41, 4)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let keyword = ratatui::style::Color::Rgb(0xbb, 0x9a, 0xf7);
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 0)].fg, keyword, "left window");
    assert_eq!(buffer[(21, 0)].fg, keyword, "right window");
}

#[test]
fn window_keys_appear_in_the_palette() {
    let mut app = app();
    keys(&mut app, "<Space>psplit_vert<CR>");
    assert_eq!(app.editor().windows().count(), 2);
}

#[test]
fn tiny_terminals_with_splits_dont_panic() {
    let mut app = plain_app();
    keys(&mut app, "ix<Esc>:vs<CR>:vs<CR>:sp<CR>:sp<CR>");
    for (w, h) in [(1, 1), (1, 3), (2, 3), (3, 4), (5, 2)] {
        render(&mut app, w, h);
    }
}
