//! The breadcrumbs and the Outline view without a language server (their
//! symbols are tested with the fake one in `lsp.rs`).

mod common;

use std::path::Path;

use common::{app_with_config, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::editor::Mode;

fn app() -> App {
    let mut app = app_with_config("[editor]\nbreadcrumbs = true\nlsp = false\n");
    app.open(Path::new("src/editor/outline.rs"));
    app
}

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

#[test]
fn the_breadcrumbs_show_the_path_and_a_folder_shows_in_the_tree() {
    let mut app = app();
    let rows = render(&mut app, 90, 20).rows;
    assert_eq!(rows[0].trim(), "src › editor › outline.rs");
    assert!(rows[1].contains("//!"), "the text under them: {}", rows[1]);
    let x = rows[0].find("editor").unwrap() as u16;
    click(&mut app, x + 1, 0);
    assert_eq!(app.editor().mode(), Mode::Sidebar);
    let row = app.editor().sidebar().unwrap().selected_row().unwrap();
    assert_eq!(row.name, "editor");
    assert!(row.is_dir);
}

#[test]
fn off_there_are_none() {
    let mut app = app_with_config("[editor]\nbreadcrumbs = false\n");
    app.open(Path::new("src/editor/outline.rs"));
    let rows = render(&mut app, 90, 20).rows;
    assert!(rows[0].contains("//!"), "{}", rows[0]);
}

#[test]
fn the_outline_says_where_symbols_come_from() {
    let mut app = app();
    keys(&mut app, "<Space>e:sidebar_view outline<CR>");
    assert!(app.editor().outline_shown());
    let screen = render(&mut app, 100, 20).rows.join("\n");
    assert!(screen.contains("No symbols yet"), "{screen}");
    // It's still there after the plugins load again.
    keys(&mut app, ":sidebar_view files<CR>");
    assert!(app.editor().sidebar_view_names().contains(&"outline"));
}
