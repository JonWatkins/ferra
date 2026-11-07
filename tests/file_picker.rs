//! The file picker lists in the background and streams files in, end to end.

mod common;

use std::fs;

use common::{app, app_with_config, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

fn labels(app: &App) -> Vec<String> {
    let picker = app.editor().picker().expect("picker open");
    picker
        .matches()
        .iter()
        .map(|m| picker.item(m).label.clone())
        .collect()
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn project(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("{name}\n")).unwrap();
    }
    dir
}

fn open_picker(app: &mut App, dir: &tempfile::TempDir) {
    let root = dir.path().display().to_string();
    keys(app, &format!(":files {}<CR>", root.replace('<', "<lt>")));
}

#[test]
fn it_opens_at_once_and_fills_in_from_the_background() {
    let dir = project(&["README.md", "src/main.rs"]);
    let mut app = app();
    open_picker(&mut app, &dir);
    assert_eq!(
        app.editor().mode(),
        Mode::Picker,
        "open before the listing ends"
    );
    assert!(app.editor().picker().unwrap().loading().is_some());
    let screen = render(&mut app, 60, 12).rows.join("\n");
    assert!(screen.contains("Files (listing…)"), "{screen}");

    app.wait_for_background();
    assert_eq!(
        labels(&app),
        ["README.md", "src/main.rs"],
        "relative to the root"
    );
    assert!(app.editor().picker().unwrap().loading().is_none());
    let screen = render(&mut app, 60, 12).rows.join("\n");
    assert!(
        screen.contains(" Files ") && !screen.contains("listing"),
        "{screen}"
    );

    keys(&mut app, "main<CR>");
    assert_eq!(text(&app), "src/main.rs");
    assert!(
        app.editor()
            .document
            .path()
            .unwrap()
            .ends_with("src/main.rs")
    );
}

#[test]
fn typing_while_it_lists_filters_what_arrives() {
    let dir = project(&["alpha.rs", "beta.rs", "gamma.txt"]);
    let mut app = app();
    open_picker(&mut app, &dir);
    keys(&mut app, ".rs");
    app.wait_for_background();
    assert_eq!(labels(&app), ["alpha.rs", "beta.rs"]);
}

#[test]
fn long_listings_are_cut_short_with_a_message() {
    let dir = project(&["a", "b", "c"]);
    let mut app = app_with_config("[editor.file-picker]\nmax-files = 2");
    open_picker(&mut app, &dir);
    app.wait_for_background();
    assert_eq!(labels(&app).len(), 2);
    assert!(message(&app).contains("first 2 files"), "{}", message(&app));
}

#[test]
fn closing_the_picker_stops_the_listing() {
    let names: Vec<String> = (0..2000).map(|i| format!("d{}/f{i}", i % 40)).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let dir = project(&names);
    let mut app = app();
    open_picker(&mut app, &dir);
    keys(&mut app, "<Esc>");
    app.wait_for_background();
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(app.editor().picker().is_none());
}

#[test]
fn a_new_picker_replaces_an_unfinished_one() {
    let first = project(&["first.txt"]);
    let second = project(&["second.txt"]);
    let mut app = app();
    open_picker(&mut app, &first);
    keys(&mut app, "<Esc>");
    open_picker(&mut app, &second);
    app.wait_for_background();
    assert_eq!(
        labels(&app),
        ["second.txt"],
        "nothing from the first listing"
    );
}
