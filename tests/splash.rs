//! The splash screen shown when ferra starts without a file, end to end.

mod common;

use std::fs;

use common::{app, app_with_config, keys, render};
use ferra::App;

/// What's on screen, as one string.
fn screen(app: &mut App) -> String {
    render(app, 80, 24).rows.join("\n")
}

fn started_without_files(config: &str) -> App {
    let mut app = app_with_config(config);
    app.open_all(&[]);
    app
}

#[test]
fn starting_without_a_file_shows_the_splash() {
    let mut app = started_without_files("");
    assert!(app.editor().splash_visible());
    let screen = screen(&mut app);
    assert!(
        screen.contains("|  _|  __/ |  | | | (_| |"),
        "the logo:\n{screen}"
    );
    assert!(screen.contains(&format!("version {}", env!("CARGO_PKG_VERSION"))));
    for (keys, what) in [
        (":e <file>", "open a file"),
        ("<Space>f", "find a file"),
        ("<Space>p", "browse commands"),
        ("i", "start writing"),
        (":q", "quit"),
    ] {
        let line = screen
            .lines()
            .find(|l| l.contains(what))
            .unwrap_or_else(|| panic!("no {what:?}:\n{screen}"));
        assert!(line.contains(keys), "{line:?}");
    }
}

#[test]
fn starting_with_a_file_opens_it_instead() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, "hello\n").unwrap();
    let mut app = app();
    app.open_all(&[path]);
    assert!(!app.editor().splash_visible());
    let screen = screen(&mut app);
    assert!(screen.contains("hello") && !screen.contains("open a file"));
}

#[test]
fn typing_dismisses_it_for_good() {
    let mut app = started_without_files("");
    keys(&mut app, "i");
    assert!(
        app.editor().splash_visible(),
        "still there until something's typed"
    );
    keys(&mut app, "x<Esc>");
    assert!(!app.editor().splash_visible());
    keys(&mut app, "u");
    assert!(app.editor().document.is_pristine_scratch(), "empty again");
    assert!(
        !app.editor().splash_visible(),
        "but the splash doesn't come back"
    );
}

#[test]
fn opening_a_file_or_splitting_dismisses_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "text\n").unwrap();
    let mut app = started_without_files("");
    keys(&mut app, ":");
    assert!(
        app.editor().splash_visible(),
        "shown behind the command line"
    );
    keys(&mut app, &format!("e {}<CR>", path.display()));
    assert!(!app.editor().splash_visible());

    let mut app = started_without_files("");
    keys(&mut app, "<C-w>v");
    assert!(!app.editor().splash_visible());
}

#[test]
fn the_menu_shows_your_own_key_bindings() {
    let mut app = started_without_files(
        "[keys.normal]\n\"<Space>f\" = \"\"\n\"<C-p>\" = \"file_picker\"\n\"<Space>p\" = \"\"",
    );
    let screen = screen(&mut app);
    let find = screen.lines().find(|l| l.contains("find a file")).unwrap();
    assert!(find.contains("<C-p>"), "{find:?}");
    assert!(
        !screen.contains("browse commands"),
        "an unbound command is left out:\n{screen}"
    );
}
