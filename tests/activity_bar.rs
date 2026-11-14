//! The activity bar down the left: its icons, the shown view marked, and
//! clicks showing views, hiding the sidebar and opening the settings.

mod common;

use common::{app_with_config, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::editor::Mode;

const W: u16 = 60;
const H: u16 = 20;

/// As shipped, but for the menu bar above (its own tests have it), so the
/// bar's rows are where these tests count them.
fn app_as_shipped() -> App {
    app_with_config(
        "[editor]\nactivity-bar = true\nstatusline = \"global\"\nstatusline-style = \"vscode\"\n",
    )
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

/// Where `icon` is in the bar (its first five columns).
fn icon_at(app: &mut App, icon: &str) -> (u16, u16) {
    let screen = render(app, W, H);
    for (y, row) in screen.rows.iter().enumerate() {
        let bar: String = row.chars().take(5).collect();
        if let Some(byte) = bar.find(icon) {
            return (bar[..byte].chars().count() as u16, y as u16);
        }
    }
    panic!("{icon} isn't in the bar:\n{}", screen.rows.join("\n"));
}

#[test]
fn the_bar_shows_the_views_and_the_settings() {
    let mut app = app_as_shipped();
    let screen = render(&mut app, W, H);
    // Files and Source Control at the top, in blocks of three rows with
    // the icon in the middle; the settings at the bottom (above the status
    // line and command line).
    assert_eq!(icon_at(&mut app, "☰"), (2, 1));
    assert_eq!(icon_at(&mut app, "⌕"), (2, 4), "Search");
    assert_eq!(icon_at(&mut app, "⎇"), (2, 7));
    let (_, gear) = icon_at(&mut app, "⚙");
    assert!(gear >= H - 6, "{gear}: {:?}", screen.rows);
    // The windows start after it.
    assert!(screen.rows[0][..].chars().nth(5).is_some());
    assert!(
        !screen.rows[0].chars().take(5).any(|c| c.is_ascii_digit()),
        "no line numbers in the bar: {:?}",
        screen.rows[0]
    );
}

#[test]
fn clicks_show_views_and_hide_the_sidebar() {
    let mut app = app_as_shipped();
    let files = icon_at(&mut app, "☰");
    click(&mut app, files.0, files.1);
    assert!(app.editor().sidebar().is_some(), "opened");
    assert_eq!(app.editor().mode(), Mode::Sidebar);
    let rows = render(&mut app, W, H).rows;
    assert!(rows[files.1 as usize].starts_with('▎'), "marked: {rows:?}");

    let git = icon_at(&mut app, "⎇");
    click(&mut app, git.0, git.1);
    assert!(app.editor().git_view_shown(), "switched to Source Control");

    // The shown one again: the sidebar goes.
    click(&mut app, git.0, git.1);
    assert!(app.editor().sidebar().is_none(), "hidden");
    assert_eq!(app.editor().mode(), Mode::Normal);

    // Anywhere in an icon's block counts.
    click(&mut app, files.0 + 1, files.1 + 1);
    assert!(app.editor().sidebar().is_some());
    assert!(!app.editor().git_view_shown());
}

#[test]
fn plugin_views_get_icons_and_the_bar_replaces_the_tabs() {
    let mut app = app_as_shipped();
    app.load_plugin(
        "todos.lua",
        r#"editor.view("todos", { title = "Todos", lines = function() return { "one" } end })
           editor.view("notes", { title = "Notes", icon = "✎", lines = function() return {} end })"#,
    )
    .unwrap();
    // Its own icon, or its title's first letter.
    let todos = icon_at(&mut app, "T");
    icon_at(&mut app, "✎");
    click(&mut app, todos.0, todos.1);
    let rows = render(&mut app, W, H).rows;
    assert!(rows.iter().any(|r| r.contains("one")), "{rows:?}");
    assert!(
        !rows[0].contains(" Files ") && !rows[0].contains(" Todos "),
        "no tabs row: {:?}",
        rows[0]
    );
}

#[test]
fn the_bar_can_be_turned_off() {
    let mut app = app_with_config("[editor]\nactivity-bar = false\n");
    let rows = render(&mut app, W, H).rows;
    assert!(rows.iter().all(|r| !r.contains('☰')));
    // The sidebar still opens.
    keys(&mut app, ":sidebar_view files<CR>");
    assert!(app.editor().sidebar().is_some());
}

#[test]
fn nerd_icons_are_vs_codes_codicons() {
    let mut app = common::app_with_config("[editor]\nactivity-bar = true\nicons = \"nerd\"\n");
    icon_at(&mut app, "\u{eaf0}");
    icon_at(&mut app, "\u{ea6d}");
    icon_at(&mut app, "\u{ea68}");
    icon_at(&mut app, "\u{eb51}");
}
