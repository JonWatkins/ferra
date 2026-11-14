//! The menu bar as shipped: its titles along the top, menus opened by
//! F10, Alt (or ⌥) and a letter, or a click, and their items running
//! commands.

mod common;

use common::{app_as_shipped, app_with_config, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::editor::Mode;

const W: u16 = 80;
const H: u16 = 24;

fn click(app: &mut App, x: u16, y: u16) {
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.handle_mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    }
}

fn screen(app: &mut App) -> Vec<String> {
    render(app, W, H).rows
}

/// Where `text` is on screen.
fn find(app: &mut App, text: &str) -> (u16, u16) {
    let rows = screen(app);
    for (y, row) in rows.iter().enumerate() {
        if let Some(byte) = row.find(text) {
            return (row[..byte].chars().count() as u16, y as u16);
        }
    }
    panic!("{text:?} isn't on screen:\n{}", rows.join("\n"));
}

fn open_menu(app: &App) -> Option<&'static str> {
    app.editor()
        .open_menu()
        .map(|i| ferra::editor::menu_bar::MENUS[i].title)
}

#[test]
fn the_bar_is_along_the_top_and_the_rest_under_it() {
    let mut app = app_as_shipped();
    let rows = screen(&mut app);
    assert!(
        rows[0].starts_with("  File  Edit  View  Go  Git  Terminal  Help"),
        "{:?}",
        rows[0]
    );
    // A line under it; the activity bar starts after that.
    assert!(rows[1].chars().all(|c| c == '─'), "{:?}", rows[1]);
    assert_eq!(find(&mut app, "☰"), (2, 3));
}

#[test]
fn f10_opens_file_and_arrows_go_along() {
    let mut app = app_as_shipped();
    keys(&mut app, "<F10>");
    assert_eq!(open_menu(&app), Some("File"));
    assert_eq!(app.editor().mode(), Mode::Popup);
    let rows = screen(&mut app).join("\n");
    assert!(rows.contains("Open File…"), "{rows}");
    // The item's keys, VS Code's as shipped.
    let open = if cfg!(target_os = "macos") {
        "⌃O"
    } else {
        "Ctrl+O"
    };
    assert!(rows.contains(open), "the item's keys: {rows}");
    keys(&mut app, "<Right>");
    assert_eq!(open_menu(&app), Some("Edit"));
    keys(&mut app, "<Left><Left>");
    assert_eq!(open_menu(&app), Some("Help"), "round the end");
    keys(&mut app, "<Esc>");
    assert_eq!(open_menu(&app), None);
    assert_eq!(app.editor().mode(), Mode::Insert, "back to typing");

    // With Vim's keys, Vim's are shown.
    let mut app = app_with_config("[editor]\nmenu-bar = true\n");
    keys(&mut app, "<F10>");
    let rows = screen(&mut app).join("\n");
    assert!(rows.contains("Space f"), "{rows}");
}

#[test]
fn enter_runs_the_item() {
    let mut app = app_as_shipped();
    // View: Command Palette, a separator, Explorer.
    keys(&mut app, "<A-v>jj<CR>");
    assert_eq!(open_menu(&app), None, "closed");
    assert!(app.editor().sidebar().is_some(), "Explorer: the sidebar");
    assert_eq!(app.editor().mode(), Mode::Sidebar);
    // A separator runs nothing (and the menu stays).
    keys(&mut app, "<Esc><A-v>j<CR>");
    assert_eq!(open_menu(&app), Some("View"));
}

#[test]
fn alt_letters_and_the_option_keys_characters() {
    // Alt and the letter, with either keymap.
    for (keys_typed, menu) in [
        ("<A-f>", "File"),
        ("<A-d>", "Edit"),
        ("<A-o>", "Go"),
        ("<A-g>", "Git"),
    ] {
        for (name, mut app) in [("vscode", app_as_shipped()), ("vim", app_with_config(""))] {
            keys(&mut app, keys_typed);
            assert_eq!(open_menu(&app), Some(menu), "{name}: {keys_typed}");
        }
    }
    // The characters ⌥ types: in Vim's normal mode.
    for (keys_typed, menu) in [("ƒ", "File"), ("∂", "Edit"), ("ø", "Go"), ("˙", "Help")] {
        let mut app = app_with_config("");
        keys(&mut app, keys_typed);
        assert_eq!(open_menu(&app), Some(menu), "{keys_typed}");
    }
    // Typing, they're text.
    let mut app = app_with_config("");
    keys(&mut app, "iƒ<Esc>");
    assert_eq!(common::text(&app), "ƒ");
    assert_eq!(open_menu(&app), None);
    let mut app = app_as_shipped();
    keys(&mut app, "ƒ");
    assert_eq!(common::text(&app), "ƒ");
    assert_eq!(open_menu(&app), None);
}

#[test]
fn clicks_open_switch_close_and_run() {
    let mut app = app_as_shipped();
    let (x, y) = find(&mut app, "View");
    click(&mut app, x, y);
    assert_eq!(open_menu(&app), Some("View"));
    let (x, y) = find(&mut app, "Git ");
    click(&mut app, x, y);
    assert_eq!(open_menu(&app), Some("Git"), "switched");
    click(&mut app, x, y);
    assert_eq!(open_menu(&app), None, "the open one's title closes it");

    let (x, y) = find(&mut app, "View");
    click(&mut app, x, y);
    let (x, y) = find(&mut app, "Search");
    click(&mut app, x, y);
    assert_eq!(open_menu(&app), None);
    assert!(app.editor().search_view_shown(), "View → Search");
}

#[test]
fn it_can_be_turned_off() {
    let mut app = app_with_config("[editor]\nmenu-bar = false\n");
    assert!(!screen(&mut app)[0].contains("File  Edit"));
    // F10 still opens the menus.
    keys(&mut app, "<F10>");
    assert_eq!(open_menu(&app), Some("File"));
}
