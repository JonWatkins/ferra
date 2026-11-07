//! Resizing windows (`<C-w>+ - > < _ | =`, `:resize`, `:vertical resize`),
//! end to end. Tests don't draw, so the windows fill the default 80x22.

mod common;

use common::{app, keys};
use ferra::App;

fn heights(app: &App) -> Vec<u16> {
    let sizes = app.editor().windows().sizes();
    sizes.iter().map(|(_, a)| a.height).collect()
}

fn widths(app: &App) -> Vec<u16> {
    let sizes = app.editor().windows().sizes();
    sizes.iter().map(|(_, a)| a.width).collect()
}

#[test]
fn taller_and_shorter_with_counts() {
    let mut app = app();
    keys(&mut app, "<C-w>s");
    assert_eq!(heights(&app), [11, 10], "21 rows after the separator");
    // The new window is the active one, below; as the last window it
    // grows from the one above.
    keys(&mut app, "3<C-w>+");
    assert_eq!(heights(&app), [8, 13]);
    keys(&mut app, "<C-w>-");
    assert_eq!(heights(&app), [9, 12]);
}

#[test]
fn wider_and_narrower() {
    let mut app = app();
    keys(&mut app, "<C-w>v");
    assert_eq!(widths(&app), [40, 39]);
    keys(&mut app, "10<C-w>>");
    assert_eq!(widths(&app), [30, 49]);
    keys(&mut app, "5<C-w><lt>");
    assert_eq!(widths(&app), [35, 44]);
}

#[test]
fn maximise_and_equalize() {
    let mut app = app();
    keys(&mut app, "<C-w>s<C-w>_");
    assert_eq!(heights(&app), [1, 20]);
    keys(&mut app, "5<C-w>_");
    assert_eq!(heights(&app), [16, 5], "a count sets the height");
    keys(&mut app, "<C-w>=");
    assert_eq!(heights(&app), [11, 10]);

    // Split the bottom window side by side and widen the new right half.
    keys(&mut app, "<C-w>v<C-w>|");
    assert_eq!(widths(&app), [80, 1, 78]);
}

#[test]
fn resize_commands() {
    let mut app = app();
    keys(&mut app, "<C-w>s:resize 6<CR>");
    assert_eq!(heights(&app), [15, 6]);
    keys(&mut app, ":res +2<CR>");
    assert_eq!(heights(&app), [13, 8]);
    keys(&mut app, ":res -3<CR>");
    assert_eq!(heights(&app), [16, 5]);
    keys(&mut app, ":resize<CR>");
    assert_eq!(heights(&app), [1, 20]);
}

#[test]
fn vertical_resize_and_split() {
    let mut app = app();
    keys(&mut app, ":vertical split<CR>");
    assert_eq!(widths(&app), [40, 39]);
    keys(&mut app, ":vert resize 20<CR>");
    assert_eq!(widths(&app), [59, 20]);
}

#[test]
fn nothing_to_resize_against_does_nothing() {
    let mut app = app();
    keys(&mut app, "5<C-w>+");
    assert_eq!(heights(&app), [22]);
    keys(&mut app, "<C-w>s5<C-w>>");
    assert_eq!(heights(&app), [11, 10], "no side-by-side split: no change");
}

#[test]
fn closing_a_window_shares_equally_again() {
    let mut app = app();
    keys(&mut app, "<C-w>s<C-w>s:resize 3<CR>");
    keys(&mut app, "<C-w>c");
    assert_eq!(heights(&app), [11, 10]);
}

#[test]
fn bad_sizes_are_reported() {
    let mut app = app();
    keys(&mut app, "<C-w>s:resize big<CR>");
    let message = app.editor().message().map(|m| m.text.clone());
    assert_eq!(message.as_deref(), Some("Invalid size: big"));
}
