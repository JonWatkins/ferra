//! Moving windows: `<C-w>x`, `<C-w>r`/`R`, `<C-w>H`/`J`/`K`/`L`, end to end.

mod common;

use std::fs;

use common::{app, keys};
use ferra::App;

/// What each window shows, in reading order, with the active one marked.
fn windows(app: &App) -> Vec<String> {
    let editor = app.editor();
    let active = editor.windows().active();
    editor
        .windows()
        .layout()
        .windows()
        .into_iter()
        .map(|id| {
            let (document, _) = editor.window_view(id).unwrap();
            let text = document.buffer().text();
            if id == active {
                format!("*{text}")
            } else {
                text
            }
        })
        .collect()
}

/// Three windows stacked top to bottom showing "a", "b" and "c", with the
/// cursor in "a".
fn three_stacked() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::write(dir.path().join(name), format!("{name}\n")).unwrap();
    }
    let mut app = app();
    app.open(&dir.path().join("a"));
    let path = |n: &str| dir.path().join(n).display().to_string();
    keys(&mut app, &format!(":sp {}<CR>", path("c")));
    keys(&mut app, "<C-w>k");
    keys(&mut app, &format!(":sp {}<CR>", path("b")));
    keys(&mut app, "<C-w>k");
    (dir, app)
}

#[test]
fn the_setup_is_as_described() {
    let (_dir, app) = three_stacked();
    assert_eq!(windows(&app), ["*a", "b", "c"]);
}

#[test]
fn exchange_swaps_with_the_next_window() {
    let (_dir, mut app) = three_stacked();
    keys(&mut app, "<C-w>x");
    assert_eq!(
        windows(&app),
        ["*b", "a", "c"],
        "the cursor stays on top, now in the other window"
    );
    keys(&mut app, "<C-w>j<C-w>j<C-w>x");
    assert_eq!(
        windows(&app),
        ["b", "c", "*a"],
        "the last swaps with the one before; the cursor stays at the bottom"
    );
}

#[test]
fn rotate_moves_every_window_along() {
    let (_dir, mut app) = three_stacked();
    keys(&mut app, "<C-w>r");
    assert_eq!(
        windows(&app),
        ["c", "*a", "b"],
        "the cursor moves with its window"
    );
    keys(&mut app, "<C-w>R<C-w>R");
    assert_eq!(windows(&app), ["b", "c", "*a"]);
}

#[test]
fn rotating_across_nested_splits_is_refused() {
    let (_dir, mut app) = three_stacked();
    // Split the top window side by side, then rotate from the column below:
    // that column now holds a split.
    keys(&mut app, "<C-w>v<C-w>j<C-w>r");
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(message.starts_with("Can't rotate"), "{message}");
}

#[test]
fn capital_l_makes_a_full_height_column_on_the_right() {
    let (_dir, mut app) = three_stacked();
    keys(&mut app, "<C-w>L");
    assert_eq!(windows(&app), ["b", "c", "*a"]);
    let sizes = app.editor().windows().sizes();
    let (_, right) = sizes[2];
    assert_eq!(right.height, 22, "full height");
    assert!(right.x > 0);
    // And back to the top, full width.
    keys(&mut app, "<C-w>K");
    let sizes = app.editor().windows().sizes();
    assert_eq!(windows(&app), ["*a", "b", "c"]);
    assert_eq!(sizes[0].1.width, 80);
}

#[test]
fn a_lone_window_stays_put() {
    let mut app = app();
    keys(&mut app, "<C-w>H<C-w>r");
    assert_eq!(app.editor().windows().count(), 1);
}
