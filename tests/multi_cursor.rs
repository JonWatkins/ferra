//! Multiple cursors with VS Code's keys: Ctrl+D, Ctrl+Shift+L,
//! Ctrl+Shift+Up/Down and Ctrl+click add them; typing, deleting, moving,
//! copying and pasting happen at each; Esc goes back to one.

mod common;

use common::{app_with_config, keys, render, text};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::clipboard::MemoryClipboard;

/// An app with VS Code's keys, `content` in, the cursor at the start.
fn with_text(content: &str) -> (App, MemoryClipboard) {
    let clipboard = MemoryClipboard::default();
    let mut app = app_with_config(
        "[editor]\nkeymap = \"vscode\"\nauto-indent = false\nauto-pairs = false\n",
    );
    app.use_clipboard(Box::new(clipboard.clone()));
    for (i, line) in content.split('\n').enumerate() {
        if i > 0 {
            keys(&mut app, "<CR>");
        }
        keys(&mut app, line);
    }
    keys(&mut app, "<C-Home>");
    (app, clipboard)
}

fn cursor_count(app: &App) -> usize {
    1 + app.editor().extra_cursors().len()
}

#[test]
fn ctrl_d_selects_matches_and_typing_replaces_them_all() {
    let (mut app, _) = with_text("let cat = cat + cat;");
    keys(&mut app, "<C-Right><C-Right>");
    keys(&mut app, "<C-d><C-d><C-d>");
    assert_eq!(cursor_count(&app), 3);
    keys(&mut app, "dog");
    assert_eq!(text(&app), "let dog = dog + dog;");
    // One undo step takes them all back.
    keys(&mut app, "<C-z>");
    assert_eq!(text(&app), "let cat = cat + cat;");
    keys(&mut app, "<C-y>");
    assert_eq!(text(&app), "let dog = dog + dog;");
}

#[test]
fn cursors_above_and_below_type_delete_and_move() {
    let (mut app, _) = with_text("one\ntwo\nthree");
    keys(&mut app, "<C-S-Down><C-S-Down>");
    assert_eq!(cursor_count(&app), 3);
    keys(&mut app, "- <End>!<BS>.");
    assert_eq!(text(&app), "- one.\n- two.\n- three.");
    // Esc: back to one.
    keys(&mut app, "<Esc>");
    assert_eq!(cursor_count(&app), 1);
    keys(&mut app, "x");
    assert_eq!(text(&app), "- one.\n- two.\n- three.x");
}

#[test]
fn select_all_matches_and_each_cursor_copies_and_pastes_its_own() {
    let (mut app, _) = with_text("a1 b2 a1");
    keys(&mut app, "<C-S-Right><C-S-l>");
    assert_eq!(cursor_count(&app), 2);
    keys(&mut app, "z");
    assert_eq!(text(&app), "z b2 z");

    // Copied at three cursors: a line each on the clipboard.
    let (mut app, clipboard) = with_text("ab\ncd\nef");
    keys(&mut app, "<C-S-Down><C-S-Down><S-End><C-c>");
    assert_eq!(clipboard.contents(), "ab\ncd\nef");
    // Pasted at as many cursors: each its own line.
    keys(&mut app, "<End> <C-v>");
    assert_eq!(text(&app), "ab ab\ncd cd\nef ef");
}

#[test]
fn ctrl_click_adds_a_cursor_and_a_click_goes_back_to_one() {
    let (mut app, _) = with_text("aaa\nbbb");
    let screen = render(&mut app, 40, 10);
    let y = screen.rows.iter().position(|r| r.contains("bbb")).unwrap() as u16;
    let x = screen.rows[y as usize].find("bbb").unwrap() as u16;
    let click = |app: &mut App, modifiers| {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            app.handle_mouse(MouseEvent {
                kind,
                column: x + 1,
                row: y,
                modifiers,
            });
        }
    };
    click(&mut app, KeyModifiers::CONTROL);
    assert_eq!(cursor_count(&app), 2);
    keys(&mut app, "_");
    assert_eq!(text(&app), "_aaa\nb_bb");
    // The extra cursor is drawn.
    let screen = render(&mut app, 40, 10);
    assert!(screen.rows.join("\n").contains("_aaa"));
    click(&mut app, KeyModifiers::NONE);
    assert_eq!(cursor_count(&app), 1);
}

#[test]
fn extra_cursors_look_like_the_real_one() {
    let (mut app, _) = with_text("function\nfunction");
    keys(&mut app, "<End><C-S-Down>");
    // The first line's cursor is an extra one now: past the end, a bar.
    let screen = render(&mut app, 40, 10);
    assert!(
        screen.rows.iter().any(|r| r.contains("function▏")),
        "{}",
        screen.rows.join("\n")
    );
}
