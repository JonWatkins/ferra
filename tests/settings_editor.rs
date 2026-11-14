//! The settings editor: every `[editor]` option by category, searchable,
//! changed with keys or clicks, applied at once and saved to config.toml
//! (keeping its comments; a default taken out).

mod common;

use std::path::PathBuf;

use common::{app, app_as_shipped, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::editor::Mode;
use ferra::editor::settings::SettingsFocus;

const W: u16 = 100;
const H: u16 = 30;

/// An app saving its settings to a config file of its own, with `text`.
fn app_saving_to(text: &str) -> (App, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    if !text.is_empty() {
        std::fs::write(&path, text).unwrap();
    }
    let mut app = app();
    app.editor_mut().set_config_file(Some(path.clone()));
    (app, dir, path)
}

fn config(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn screen(app: &mut App) -> String {
    render(app, W, H).rows.join("\n")
}

fn selected(app: &App) -> String {
    app.editor()
        .settings()
        .and_then(|s| s.current())
        .map(|s| s.path.clone())
        .unwrap_or_default()
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
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

fn find(app: &mut App, text: &str) -> (u16, u16) {
    let rows = render(app, W, H).rows;
    for (y, row) in rows.iter().enumerate() {
        if let Some(byte) = row.find(text) {
            return (row[..byte].chars().count() as u16, y as u16);
        }
    }
    panic!("{text:?} isn't on screen:\n{}", rows.join("\n"));
}

#[test]
fn opens_in_the_windows_place_and_closes() {
    let mut app = app();
    keys(&mut app, "ihello<Esc>:settings<CR>");
    assert_eq!(app.editor().mode(), Mode::Settings);
    let shown = screen(&mut app);
    assert!(shown.contains("Settings"), "{shown}");
    assert!(shown.contains("Editing"), "the categories: {shown}");
    assert!(shown.contains("Tab Width"), "{shown}");
    assert!(
        shown.contains("Width of a tab stop"),
        "its description: {shown}"
    );
    assert!(!shown.contains("hello"), "the windows are covered: {shown}");
    assert_eq!(selected(&app), "tab-width");

    keys(&mut app, "q");
    assert!(app.editor().settings().is_none());
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(screen(&mut app).contains("hello"));

    // <Esc> closes it too, and the `:` line comes back to it.
    keys(&mut app, ":settings<CR>:<Esc>");
    assert_eq!(app.editor().mode(), Mode::Settings);
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn the_gear_and_the_menus_open_it() {
    let mut app = app_as_shipped();
    keys(&mut app, "<C-g>activity settings<CR>");
    assert_eq!(app.editor().mode(), Mode::Settings);
    keys(&mut app, ":activity settings<CR>");
    assert!(
        app.editor().settings().is_none(),
        "the gear again closes it"
    );

    // File → Settings.
    keys(&mut app, "<F10>");
    let (x, y) = find(&mut app, "Settings ");
    click(&mut app, x, y);
    assert_eq!(app.editor().mode(), Mode::Settings);
}

#[test]
fn a_switch_flips_applies_and_is_saved() {
    let (mut app, _dir, path) = app_saving_to("");
    keys(&mut app, ":settings expand tab<CR>");
    assert_eq!(selected(&app), "expand-tab");
    assert!(app.editor().options.expand_tab);
    keys(&mut app, "<Space>");
    assert!(!app.editor().options.expand_tab, "applied at once");
    assert_eq!(config(&path), "[editor]\nexpand-tab = false\n");
    assert!(screen(&mut app).contains("[ ] off"));
    // Back to the default: out of the file.
    keys(&mut app, "<CR>");
    assert!(app.editor().options.expand_tab);
    assert_eq!(config(&path), "");
    assert!(message(&app).contains("the default"), "{}", message(&app));
}

#[test]
fn choices_go_round_and_reset_puts_back_the_default() {
    let comments = "# Mine.\n[editor]\n# Wide.\ntab-width = 8\n";
    let (mut app, _dir, path) = app_saving_to(comments);
    keys(&mut app, ":settings line numbers<CR>");
    assert_eq!(selected(&app), "line-numbers");
    // relative → none → absolute (round the end) → none.
    keys(&mut app, "l");
    assert_eq!(
        app.editor().options.line_numbers,
        ferra::config::LineNumbers::None
    );
    keys(&mut app, "l");
    assert_eq!(
        app.editor().options.line_numbers,
        ferra::config::LineNumbers::Absolute
    );
    assert_eq!(
        config(&path),
        "# Mine.\n[editor]\n# Wide.\ntab-width = 8\nline-numbers = \"absolute\"\n",
        "the file's comments stay"
    );
    keys(&mut app, "h");
    assert_eq!(
        app.editor().options.line_numbers,
        ferra::config::LineNumbers::None
    );
    keys(&mut app, "h");
    assert_eq!(
        app.editor().options.line_numbers,
        ferra::config::LineNumbers::Relative
    );
    assert_eq!(config(&path), comments);

    // u: the default.
    keys(&mut app, "lu");
    assert_eq!(config(&path), comments);
}

#[test]
fn numbers_and_text_are_typed() {
    let (mut app, _dir, path) = app_saving_to("");
    keys(&mut app, ":settings tab width<CR>");
    keys(&mut app, "<CR>");
    let view = app.editor().settings().unwrap();
    assert_eq!(view.focus, SettingsFocus::Edit);
    assert_eq!(view.edit.text(), "4");
    // The old value is selected: typing replaces it.
    keys(&mut app, "2<CR>");
    assert_eq!(app.editor().options.tab_width, 2);
    assert_eq!(config(&path), "[editor]\ntab-width = 2\n");
    assert_eq!(app.editor().settings().unwrap().focus, SettingsFocus::List);

    // Not a number: nothing changes, and it says so.
    keys(&mut app, "<CR>x<CR>");
    assert_eq!(app.editor().options.tab_width, 2);
    assert!(
        message(&app).contains("isn't a number"),
        "{}",
        message(&app)
    );
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().settings().unwrap().focus, SettingsFocus::List);

    // + and - step a number.
    keys(&mut app, "+++-");
    assert_eq!(app.editor().options.tab_width, 4);

    // Text.
    keys(&mut app, "/wrap marker<CR><CR><C-u>> <CR>");
    assert_eq!(selected(&app), "wrap-marker");
    assert_eq!(app.editor().options.wrap_marker, "> ");
    assert!(
        config(&path).contains("wrap-marker = \"> \""),
        "{}",
        config(&path)
    );
}

#[test]
fn searching_and_the_categories() {
    let mut app = app();
    keys(&mut app, ":settings<CR>/");
    assert_eq!(
        app.editor().settings().unwrap().focus,
        SettingsFocus::Search
    );
    // Typed keys are the search's, not commands (q doesn't close it).
    keys(&mut app, "qzxq");
    assert_eq!(app.editor().mode(), Mode::Settings);
    assert!(screen(&mut app).contains("No settings match"));
    keys(&mut app, "<C-u>smart");
    assert_eq!(selected(&app), "search.case");
    let shown = screen(&mut app);
    assert!(shown.contains("Search › Case"), "{shown}");
    keys(&mut app, "<CR>");
    assert_eq!(app.editor().settings().unwrap().focus, SettingsFocus::List);
    // <Esc> in the box clears it.
    keys(&mut app, "/<Esc>");
    assert!(!app.editor().settings().unwrap().searching());
    assert_eq!(selected(&app), "tab-width");

    // Tab to the categories; j shows the next one's settings.
    keys(&mut app, "<Tab>j");
    let view = app.editor().settings().unwrap();
    assert_eq!(view.focus, SettingsFocus::Categories);
    assert_eq!(selected(&app), "line-numbers", "Display's first");
    keys(&mut app, "<CR>j");
    assert_eq!(selected(&app), "wrap");
}

#[test]
fn clicks_pick_categories_and_change_values() {
    let (mut app, _dir, path) = app_saving_to("");
    keys(&mut app, ":settings<CR>");
    let (x, y) = find(&mut app, "Git ");
    click(&mut app, x, y);
    assert_eq!(selected(&app), "git");
    // A click on a value changes it.
    let (x, y) = find(&mut app, "[✓] on");
    click(&mut app, x + 1, y);
    assert!(!app.editor().options.git);
    assert_eq!(config(&path), "[editor]\ngit = false\n");
    // On a name, selects it.
    let (x, y) = find(&mut app, "Git Blame");
    click(&mut app, x, y);
    assert_eq!(selected(&app), "git-blame");
    assert!(app.editor().options.git_blame, "only selected");
}

#[test]
fn lists_are_edited_in_the_file() {
    let (mut app, _dir, path) = app_saving_to("");
    keys(&mut app, ":settings layout left<CR><CR>");
    assert_eq!(selected(&app), "statusline-layout.left");
    assert!(message(&app).contains("config.toml"), "{}", message(&app));
    // e: the file itself.
    keys(&mut app, "e");
    assert!(app.editor().settings().is_none());
    assert_eq!(app.editor().document.path(), Some(path.as_path()));
}

#[test]
fn opening_a_file_takes_the_windows_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "notes\n").unwrap();
    let mut app = app();
    keys(&mut app, ":settings<CR>");
    keys(&mut app, &format!(":e {}<CR>", file.display()));
    assert!(app.editor().settings().is_none());
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(screen(&mut app).contains("notes"));
}

#[test]
fn without_a_config_file_changes_still_apply() {
    let mut app = app();
    keys(&mut app, ":settings cursor line<CR><Space>");
    assert!(!app.editor().options.cursorline);
    assert!(message(&app).contains("not saved"), "{}", message(&app));
}

#[test]
fn narrow_it_drops_the_categories() {
    let mut app = app();
    keys(&mut app, ":settings<CR>");
    let rows = render(&mut app, 50, 20).rows.join("\n");
    assert!(!rows.contains("Workbench"), "{rows}");
    assert!(rows.contains("Tab Width"), "{rows}");
}
