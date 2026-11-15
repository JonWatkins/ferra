//! The tab bar as shipped: a tab per open buffer along the top of the
//! windows; a click shows one, its × or a middle click closes it.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{app_as_shipped, app_with_config, keys, render, text};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::editor::Mode;

const W: u16 = 100;
const H: u16 = 20;

fn files(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("{name}\n")).unwrap();
            path
        })
        .collect();
    (dir, paths)
}

fn click(app: &mut App, button: MouseButton, x: u16, y: u16) {
    for kind in [MouseEventKind::Down(button), MouseEventKind::Up(button)] {
        app.handle_mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    }
}

/// The tab bar's row, and where `text` is on it.
fn tab(app: &mut App, text: &str) -> (u16, u16) {
    let rows = render(app, W, H).rows;
    let y = rows
        .iter()
        .position(|r| r.contains('│') && r.contains(text))
        .unwrap();
    let byte = rows[y].find(text).unwrap();
    (rows[y][..byte].chars().count() as u16, y as u16)
}

fn bar(app: &mut App) -> String {
    let (_, y) = tab(app, "×");
    render(app, W, H).rows[y as usize].clone()
}

#[test]
fn a_tab_per_buffer_the_active_one_marked() {
    let (_dir, paths) = files(&["a.txt", "b.txt"]);
    let mut app = app_as_shipped();
    for path in &paths {
        app.open(path);
    }
    let row = bar(&mut app);
    assert!(row.contains("a.txt ×"), "{row}");
    assert!(row.contains("▎ b.txt ×"), "the active one: {row}");
    // Unsaved changes: ●.
    keys(&mut app, "x");
    assert!(bar(&mut app).contains("b.txt ●"), "{}", bar(&mut app));
}

#[test]
fn clicking_a_tab_shows_it_and_its_cross_closes_it() {
    let (_dir, paths) = files(&["a.txt", "b.txt", "c.txt"]);
    let mut app = app_as_shipped();
    for path in &paths {
        app.open(path);
    }
    let (x, y) = tab(&mut app, "a.txt");
    click(&mut app, MouseButton::Left, x, y);
    assert_eq!(text(&app), "a.txt");
    assert_eq!(app.editor().mode(), Mode::Insert);

    // The × after b.txt's name.
    let (x, y) = tab(&mut app, "b.txt");
    click(&mut app, MouseButton::Left, x + 6, y);
    assert!(
        !bar(&mut app).contains("b.txt"),
        "closed: {}",
        bar(&mut app)
    );
    // A middle click closes too.
    let (x, y) = tab(&mut app, "c.txt");
    click(&mut app, MouseButton::Middle, x, y);
    assert!(!bar(&mut app).contains("c.txt"));
    assert_eq!(text(&app), "a.txt");
}

#[test]
fn closing_a_tab_with_unsaved_changes_asks() {
    let (_dir, paths) = files(&["a.txt", "b.txt"]);
    let mut app = app_as_shipped();
    for path in &paths {
        app.open(path);
    }
    keys(&mut app, "x");
    let (x, y) = tab(&mut app, "b.txt");
    click(&mut app, MouseButton::Left, x + 6, y);
    assert_eq!(app.editor().mode(), Mode::Popup);
    keys(&mut app, "<Esc>");
    assert!(bar(&mut app).contains("b.txt ●"));
}

#[test]
fn the_same_name_twice_shows_the_folders() {
    let (_dir, paths) = files(&["src/mod.rs", "ui/mod.rs"]);
    let mut app = app_as_shipped();
    for path in &paths {
        app.open(path);
    }
    let row = bar(&mut app);
    assert!(
        row.contains("src/mod.rs") && row.contains("ui/mod.rs"),
        "{row}"
    );
}

#[test]
fn many_tabs_scroll_to_the_active_one() {
    let names: Vec<String> = (0..12).map(|i| format!("file-number-{i}.txt")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (_dir, paths) = files(&refs);
    let mut app = app_as_shipped();
    for path in &paths {
        app.open(path);
    }
    let row = bar(&mut app);
    assert!(
        row.contains("file-number-11.txt"),
        "the last opened shows: {row}"
    );
    assert!(!row.contains("file-number-0.txt"), "{row}");
}

#[test]
fn it_can_be_turned_off() {
    let (_dir, paths) = files(&["a.txt"]);
    let mut app = app_with_config("[editor]\ntab-bar = false\n");
    app.open(&paths[0]);
    let rows = render(&mut app, W, H).rows;
    assert!(!rows.iter().any(|r| r.contains("a.txt ×")));
    // On, the text moves down a row.
    let mut app = app_with_config("[editor]\ntab-bar = true\n");
    app.open(&paths[0]);
    let rows = render(&mut app, W, H).rows;
    assert!(rows[0].contains("a.txt ×"), "{}", rows[0]);
    assert!(rows[1].contains("a.txt"), "{}", rows[1]);
}

fn run(app: &mut App, command: &str) {
    app.execute(command, ferra::commands::Invocation::default());
}

/// Window `n`'s (from the left) part of the tab row, `W` wide split in two.
fn halves(app: &mut App) -> (String, String) {
    let rows = render(app, W, H).rows;
    let row = rows.iter().find(|r| r.contains('×')).unwrap();
    let chars: Vec<char> = row.chars().collect();
    let mid = chars.len() / 2;
    (
        chars[..mid].iter().collect(),
        chars[mid..].iter().collect(),
    )
}

#[test]
fn each_window_has_its_own_tabs() {
    let (_dir, paths) = files(&["a.txt", "b.txt", "c.txt"]);
    let mut app = app_as_shipped();
    app.open(&paths[0]);
    app.open(&paths[1]);
    // A new window's one tab is what it shows.
    run(&mut app, "split_vertical");
    let (left, right) = halves(&mut app);
    assert!(left.contains("a.txt") && left.contains("b.txt"), "{left}");
    assert!(right.contains("▎ b.txt") && !right.contains("a.txt"), "{right}");

    // Opened in a window: a tab there only, after the shown one.
    app.open(&paths[2]);
    let (left, right) = halves(&mut app);
    assert!(!left.contains("c.txt"), "{left}");
    assert!(right.contains("b.txt × │▎ c.txt"), "{right}");
    // The other window's shown tab isn't marked.
    assert!(!left.contains('▎'), "{left}");

    // Ctrl+Tab goes round this window's tabs only.
    run(&mut app, "tab_next");
    assert_eq!(text(&app), "b.txt");
    run(&mut app, "tab_next");
    assert_eq!(text(&app), "c.txt");

    // A tab clicked in the other window: that window, that buffer.
    let (x, y) = tab(&mut app, "a.txt");
    click(&mut app, MouseButton::Left, x, y);
    assert_eq!(text(&app), "a.txt");
    assert_eq!(app.editor().windows().active(), ferra::editor::WindowId(0));
}

#[test]
fn closing_a_tab_open_in_another_window_keeps_the_buffer() {
    let (_dir, paths) = files(&["a.txt", "b.txt"]);
    let mut app = app_as_shipped();
    app.open(&paths[0]);
    app.open(&paths[1]);
    run(&mut app, "split_vertical");
    // b.txt is in both windows: closing it here leaves the left one's.
    keys(&mut app, "x");
    run(&mut app, "tab_close");
    assert!(app.editor().popup().is_none(), "nothing to ask: it stays open");
    // That was the window's only tab: it closes.
    assert_eq!(app.editor().windows().count(), 1);
    assert_eq!(text(&app), "xb.txt");
    assert!(bar(&mut app).contains("b.txt ●"), "{}", bar(&mut app));

    // The buffer's last tab: the buffer closes, the tab beside it shows.
    run(&mut app, "tab_prev");
    assert_eq!(text(&app), "a.txt");
    run(&mut app, "tab_close");
    assert_eq!(text(&app), "xb.txt");
    let (_, y) = tab(&mut app, "b.txt");
    let row = render(&mut app, W, H).rows[y as usize].clone();
    assert!(!row.contains("a.txt"), "{row}");
    assert_eq!(app.editor().buffer_count(), 1);
}

#[test]
fn files_opened_after_splitting_the_starting_buffer_stay_in_their_window() {
    let (_dir, paths) = files(&["a.txt", "b.txt"]);
    let mut app = app_as_shipped();
    run(&mut app, "split_vertical");
    app.open(&paths[0]);
    let (left, right) = halves(&mut app);
    assert!(right.contains("a.txt") && !left.contains("a.txt"), "{left}|{right}");
    // The untouched starting buffer's tab gave way to it.
    assert!(!right.contains("[scratch]") && !right.contains("untitled"), "{right}");

    app.editor_mut().focus_window(ferra::editor::WindowId(0));
    app.open(&paths[1]);
    let (left, right) = halves(&mut app);
    assert!(left.contains("b.txt") && !left.contains("a.txt"), "{left}");
    assert!(right.contains("a.txt") && !right.contains("b.txt"), "{right}");
}

fn drag(app: &mut App, from: (u16, u16), to: (u16, u16)) {
    let event = |kind, (x, y): (u16, u16)| MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(event(MouseEventKind::Down(MouseButton::Left), from));
    app.handle_mouse(event(MouseEventKind::Drag(MouseButton::Left), to));
    app.handle_mouse(event(MouseEventKind::Up(MouseButton::Left), to));
}

#[test]
fn tabs_are_dragged_along_and_to_other_windows() {
    let (_dir, paths) = files(&["a.txt", "b.txt", "c.txt"]);
    let mut app = app_as_shipped();
    for path in &paths {
        app.open(path);
    }
    // a.txt dragged onto c.txt's end: after it.
    let from = tab(&mut app, "a.txt");
    let (cx, y) = tab(&mut app, "c.txt");
    drag(&mut app, from, (cx + 6, y));
    let row = bar(&mut app);
    let at = |name: &str| row.find(name).unwrap();
    assert!(at("b.txt") < at("c.txt") && at("c.txt") < at("a.txt"), "{row}");
    assert_eq!(text(&app), "a.txt");

    // b.txt dragged into a new window's text: it moves there.
    run(&mut app, "split_vertical");
    let (left, _) = halves(&mut app);
    assert!(left.contains("b.txt"), "{left}");
    let from = tab(&mut app, "b.txt");
    drag(&mut app, from, (W - 10, H / 2));
    let (left, right) = halves(&mut app);
    assert!(!left.contains("b.txt"), "left it: {left}");
    assert!(right.contains("b.txt"), "{right}");
    assert_eq!(text(&app), "b.txt");
}
