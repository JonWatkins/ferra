//! The keybinding editor: every command's keys, searchable by name or by
//! pressing keys; changed by pressing the new keys; saved to config.toml's
//! [keys] and in force at once.

mod common;

use std::path::PathBuf;

use common::{app, app_as_shipped, keys, render, text};
use ferra::App;
use ferra::editor::Mode;
use ferra::editor::keybindings::{BindingSource, KeysFocus};

/// Vim's keys, saving to a config file of its own, with `config` in it.
fn app_saving_to(config: &str) -> (App, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    if !config.is_empty() {
        std::fs::write(&path, config).unwrap();
    }
    let mut app = app();
    app.editor_mut().set_config_file(Some(path.clone()));
    (app, dir, path)
}

fn config(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// The selected row: command, keys (notation), mode, source.
fn selected(app: &App) -> (String, String, Option<Mode>, BindingSource) {
    let row = app
        .editor()
        .keybindings()
        .unwrap()
        .current()
        .unwrap()
        .clone();
    (
        row.command,
        ferra::editor::keybindings::notation(&row.keys),
        row.mode,
        row.source,
    )
}

/// Selects the row for `command` bound to `keys` in `mode`.
fn select(app: &mut App, command: &str, keys_: &str, mode: Mode) {
    keys(app, "gg");
    for _ in 0..50 {
        let (c, k, m, _) = selected(app);
        if c == command && k == keys_ && m == Some(mode) {
            return;
        }
        keys(app, "j");
    }
    panic!("no row for {command} {keys_} in {mode:?}");
}

#[test]
fn opens_searches_and_closes() {
    let mut app = app();
    keys(&mut app, ":keybindings join lines<CR>");
    assert_eq!(app.editor().mode(), Mode::Keybindings);
    let shown = render(&mut app, 100, 24).rows.join("\n");
    assert!(shown.contains("Keyboard Shortcuts"), "{shown}");
    assert!(shown.contains("join_lines"), "{shown}");
    assert!(shown.contains("normal"), "the mode: {shown}");
    assert!(shown.contains("keymap"), "where it's from: {shown}");
    assert_eq!(selected(&app).0, "join_lines");

    // `/` searches again (what's there is replaced as you type).
    keys(&mut app, "/write all<CR>");
    assert_eq!(selected(&app).0, "write_all");
    // Unbound commands are listed too.
    keys(&mut app, "/trim trailing<CR>");
    let (command, keys_, mode, source) = selected(&app);
    assert_eq!(command, "trim_trailing_whitespace");
    assert_eq!(
        (keys_.as_str(), mode, source),
        ("", None, BindingSource::Unbound)
    );

    keys(&mut app, "q");
    assert!(app.editor().keybindings().is_none());
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn searching_by_pressing_keys() {
    let mut app = app();
    keys(&mut app, ":keybindings<CR>r<C-w>");
    let view = app.editor().keybindings().unwrap();
    assert_eq!(view.focus, KeysFocus::Record);
    // Every binding starting with <C-w>, whatever it would do.
    assert!(
        view.shown()
            .iter()
            .all(|r| r.keys[0].to_string() == "<C-w>")
    );
    assert!(view.shown().iter().any(|r| r.command == "window_left"));
    keys(&mut app, "h");
    let view = app.editor().keybindings().unwrap();
    assert!(
        view.shown().iter().all(|r| r.command == "window_left"),
        "<C-w>h"
    );
    keys(&mut app, "<CR>");
    let view = app.editor().keybindings().unwrap();
    assert_eq!(view.focus, KeysFocus::List);
    assert!(!view.key_filter.is_empty(), "the search stays");
    // Esc while pressing keys clears them.
    keys(&mut app, "r<Esc>");
    assert!(app.editor().keybindings().unwrap().key_filter.is_empty());
}

#[test]
fn changing_a_binding_saves_it_and_it_works() {
    let (mut app, _dir, path) = app_saving_to("# Mine.\n[editor]\nwrap = false\n");
    keys(&mut app, ":keybindings join lines<CR>");
    select(&mut app, "join_lines", "J", Mode::Normal);
    // Press the new keys, then <CR> to keep them.
    keys(&mut app, "<CR><C-j>");
    let shown = render(&mut app, 100, 24).rows.join("\n");
    assert!(
        shown.contains("Keys for join_lines in normal: ⌃J") || shown.contains("Ctrl+J"),
        "{shown}"
    );
    keys(&mut app, "<CR>");
    assert_eq!(
        config(&path),
        "# Mine.\n[editor]\nwrap = false\n\n[keys.normal]\nJ = \"\"\n\"<C-j>\" = \"join_lines\"\n"
    );
    assert_eq!(selected(&app).1, "<C-j>", "still on it");
    assert_eq!(selected(&app).3, BindingSource::User);

    // In force at once: J does nothing, <C-j> joins.
    keys(&mut app, "q");
    keys(&mut app, "ione<CR>two<Esc>ggJ");
    assert_eq!(text(&app), "one\ntwo");
    keys(&mut app, "<C-j>");
    assert_eq!(text(&app), "one two");
}

#[test]
fn adding_removing_and_resetting() {
    let (mut app, _dir, path) = app_saving_to("");
    keys(&mut app, ":keybindings join lines<CR>");
    select(&mut app, "join_lines", "J", Mode::Normal);
    keys(&mut app, "a<C-j><CR>");
    assert_eq!(
        config(&path),
        "[keys.normal]\n\"<C-j>\" = \"join_lines\"\n",
        "J stays"
    );

    // d on the user's own: its line goes.
    select(&mut app, "join_lines", "<C-j>", Mode::Normal);
    keys(&mut app, "d");
    assert_eq!(config(&path), "");
    // d on the keymap's: taken away with "".
    select(&mut app, "join_lines", "J", Mode::Normal);
    keys(&mut app, "d");
    assert_eq!(config(&path), "[keys.normal]\nJ = \"\"\n");
    assert_eq!(selected(&app).3, BindingSource::Removed);
    // u: back to the keymap's.
    keys(&mut app, "u");
    assert_eq!(config(&path), "");
    assert_eq!(selected(&app).3, BindingSource::Keymap);
}

#[test]
fn clashes_are_shown_and_said() {
    let (mut app, _dir, _path) = app_saving_to("");
    keys(&mut app, ":keybindings join lines<CR>");
    select(&mut app, "join_lines", "J", Mode::Normal);
    keys(&mut app, "<CR>x");
    let clashes: Vec<String> = app
        .editor()
        .keybindings()
        .unwrap()
        .conflicts()
        .iter()
        .map(|r| r.command.clone())
        .collect();
    assert_eq!(clashes, ["delete_char"]);
    keys(&mut app, "<CR>");
    assert!(
        message(&app).contains("took them from delete_char"),
        "{}",
        message(&app)
    );
    keys(&mut app, "q");
    keys(&mut app, "ione<CR>two<Esc>ggx");
    assert_eq!(text(&app), "one two", "x joins now");
}

#[test]
fn an_unbound_command_is_bound_in_the_home_mode() {
    let (mut app, _dir, path) = app_saving_to("");
    keys(&mut app, ":keybindings trim trailing<CR><CR><F5><CR>");
    assert_eq!(
        config(&path),
        "[keys.normal]\n\"<F5>\" = \"trim_trailing_whitespace\"\n"
    );
}

#[test]
fn without_a_config_file_it_still_applies() {
    let mut app = app();
    keys(&mut app, ":keybindings join lines<CR>");
    select(&mut app, "join_lines", "J", Mode::Normal);
    keys(&mut app, "<CR><C-j><CR>");
    assert!(message(&app).contains("not saved"), "{}", message(&app));
    keys(&mut app, "q");
    keys(&mut app, "ione<CR>two<Esc>gg<C-j>");
    assert_eq!(text(&app), "one two");
}

#[test]
fn opened_from_the_settings_editor_and_vs_codes_keys() {
    let mut app = app();
    keys(&mut app, ":settings<CR>K");
    assert_eq!(app.editor().mode(), Mode::Keybindings);
    assert!(app.editor().settings().is_none());

    let mut app = app_as_shipped();
    keys(&mut app, "<C-k><C-s>");
    assert_eq!(app.editor().mode(), Mode::Keybindings);
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Insert);
}
