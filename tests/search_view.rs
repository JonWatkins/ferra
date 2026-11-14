//! The Search view: finding in every file of a project, end to end. The
//! project is a temporary folder made the current one, so these tests run
//! one at a time (they share the process's current folder).

mod common;

use std::fs;
use std::sync::Mutex;

use common::{app_with_config, keys, render};
use ferra::App;
use ferra::editor::Mode;

/// The current folder is the process's: one test at a time.
static CWD: Mutex<()> = Mutex::new(());

struct Project {
    _dir: tempfile::TempDir,
    old: std::path::PathBuf,
}

impl Drop for Project {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old).unwrap();
    }
}

fn project(files: &[(&str, &str)]) -> Project {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    Project { _dir: dir, old }
}

const FILES: &[(&str, &str)] = &[
    (
        "src/main.rs",
        "fn main() {\n    alpha();\n}\n\nfn alpha() {}\n",
    ),
    ("notes.txt", "Alpha is first\nbeta second\n"),
    ("target/skip.rs", "alpha\n"),
    (".gitignore", "target\n"),
];

fn searching(query: &str) -> App {
    // Plain icons, so the toggles are `Aa W .*` on screen.
    let mut app = app_with_config("[editor]\nicons = \"plain\"\n");
    keys(&mut app, "<Space>F");
    keys(&mut app, query);
    app.wait_for_background();
    app
}

#[test]
fn finds_in_every_file_as_you_type() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    // `.gitignore` needs a repository to count.
    std::process::Command::new("git")
        .args(["init", "-q"])
        .status()
        .unwrap();
    let mut app = searching("alpha");
    assert_eq!(app.editor().mode(), Mode::Find, "typing in the query");
    assert_eq!(app.editor().search_status(), "3 results in 2 files");
    let rows = render(&mut app, 70, 20).rows;
    let shown = rows.join("\n");
    assert!(shown.contains("SEARCH"), "{shown}");
    assert!(shown.contains("main.rs src"), "{shown}");
    assert!(shown.contains("alpha();"), "{shown}");
    assert!(
        shown.contains("Alpha is first"),
        "case-insensitive: {shown}"
    );
    assert!(!shown.contains("skip.rs"), "ignored: {shown}");
    assert!(
        shown.contains("Aa") && shown.contains(".*"),
        "toggles: {shown}"
    );

    // Match case: just the two in main.rs.
    keys(&mut app, "<C-a>");
    app.wait_for_background();
    assert_eq!(app.editor().search_status(), "2 results in 1 file");
}

#[test]
fn enter_goes_to_the_results_and_opens_a_match() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("alpha");
    keys(&mut app, "<CR>");
    assert_eq!(app.editor().mode(), Mode::Sidebar, "in the results");
    // The first match is selected: main.rs's line 2 (files in name order:
    // notes.txt, then src/main.rs).
    keys(&mut app, "<CR>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    let path = app.editor().document.path().unwrap().to_path_buf();
    assert!(path.ends_with("notes.txt"), "{path:?}");
    assert_eq!(app.editor().document.cursor().row, 0);
    // `n` carries on with the search in the file.
    keys(&mut app, "<Space>F<CR>jj<CR>");
    let path = app.editor().document.path().unwrap().to_path_buf();
    assert!(path.ends_with("main.rs"), "{path:?}");
    assert_eq!(
        app.editor().document.cursor(),
        ferra::core::Position::new(1, 4)
    );
}

#[test]
fn results_fold_and_slash_goes_back_to_the_query() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("alpha");
    keys(&mut app, "<CR>k<CR>");
    // The first file's matches folded away.
    let rows = render(&mut app, 70, 20).rows.join("\n");
    assert!(rows.contains("▸ notes.txt"), "{rows}");
    keys(&mut app, "/");
    assert_eq!(app.editor().mode(), Mode::Find);
    keys(&mut app, "<C-u>main");
    app.wait_for_background();
    assert_eq!(app.editor().search_status(), "1 result in 1 file");
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal, "back to the editor");
    assert!(app.editor().search_view_shown(), "the view stays");
}

#[test]
fn bad_regexes_and_nothing_found_say_so() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("zzz");
    assert_eq!(app.editor().search_status(), "No results");
    keys(&mut app, "<C-u>(<C-r>");
    app.wait_for_background();
    assert!(
        app.editor().search_status().contains("unclosed group"),
        "{}",
        app.editor().search_status()
    );
}

fn click(app: &mut App, x: u16, y: u16) {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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

/// Where `text` is on a 70x20 screen.
fn find(app: &mut App, text: &str) -> (u16, u16) {
    let rows = render(app, 70, 20).rows;
    for (y, row) in rows.iter().enumerate() {
        if let Some(byte) = row.find(text) {
            return (row[..byte].chars().count() as u16, y as u16);
        }
    }
    panic!("{text:?} isn't on screen:\n{}", rows.join("\n"));
}

#[test]
fn clicks_work_while_typing_in_the_field() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("alpha");
    assert_eq!(app.editor().mode(), Mode::Find);
    // A click on a result, straight from typing: it opens.
    let (x, y) = find(&mut app, "alpha();");
    click(&mut app, x, y);
    let path = app.editor().document.path().unwrap().to_path_buf();
    assert!(path.ends_with("main.rs"), "{path:?}");
    assert_eq!(app.editor().mode(), Mode::Normal);

    // Back into the field by clicking it, then a toggle by clicking.
    let (x, y) = find(&mut app, "alpha ");
    click(&mut app, x, y);
    assert_eq!(app.editor().mode(), Mode::Find, "typing again");
    let (x, y) = find(&mut app, "Aa");
    click(&mut app, x, y);
    app.wait_for_background();
    assert_eq!(app.editor().search_status(), "2 results in 1 file");
}

#[test]
fn the_wheel_scrolls_the_results_not_the_selection() {
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    // Enough matches to scroll.
    let many: String = (0..60).map(|i| format!("alpha {i}\n")).collect();
    let _project = project(&[("long.txt", many.as_str())]);
    let mut app = searching("alpha");
    keys(&mut app, "<CR>");
    let selected = app.editor().project_search().selected();
    let (x, y) = find(&mut app, "alpha 1");
    for _ in 0..3 {
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    }
    let rows = render(&mut app, 70, 20).rows.join("\n");
    assert!(
        !rows.contains("alpha 1\n") && !rows.contains("alpha 1 "),
        "scrolled: {rows}"
    );
    assert_eq!(
        app.editor().project_search().selected(),
        selected,
        "not moved"
    );
    // A key brings the selection back into view.
    keys(&mut app, "j");
    let rows = render(&mut app, 70, 20).rows.join("\n");
    assert!(rows.contains("alpha 1"), "{rows}");
}

fn popup_text(app: &App) -> String {
    app.editor()
        .popup()
        .map(|p| {
            p.lines
                .iter()
                .map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// The text of the buffer for `name`, opening it.
fn buffer_text(app: &mut App, name: &str) -> String {
    keys(app, &format!(":e {name}<CR>"));
    common::text(app)
}

#[test]
fn replace_all_edits_the_buffers_and_asks_first() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("alpha");
    // To the replace field; the preview shows in the results.
    keys(&mut app, "<Tab>omega");
    let rows = render(&mut app, 70, 20).rows.join("\n");
    assert!(rows.contains("alphaomega();"), "old then new: {rows}");
    keys(&mut app, "<CR>");
    let shown = popup_text(&app);
    assert!(
        shown.contains("Replace 3 matches in 2 files with “omega”?"),
        "{shown}"
    );
    keys(&mut app, "y");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Replaced 3 matches in 2 files"),
        "{}",
        message(&app)
    );
    // The search sees the buffers' unsaved text: nothing left.
    assert_eq!(app.editor().search_status(), "No results");
    // Not on disk until saved.
    assert!(
        fs::read_to_string("notes.txt")
            .unwrap()
            .starts_with("Alpha")
    );
    assert_eq!(
        buffer_text(&mut app, "src/main.rs"),
        "fn main() {\n    omega();\n}\n\nfn omega() {}"
    );
    assert!(buffer_text(&mut app, "notes.txt").starts_with("omega is first"));
    // One undo step per file.
    keys(&mut app, "u");
    assert!(common::text(&app).starts_with("Alpha is first"));
    keys(&mut app, ":wa<CR>");
    assert!(
        fs::read_to_string("src/main.rs")
            .unwrap()
            .contains("omega();")
    );
}

#[test]
fn r_replaces_the_selected_match_or_file() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("alpha");
    keys(&mut app, "<Tab>beta<Tab>");
    assert_eq!(app.editor().mode(), Mode::Sidebar, "Tab, Tab: the results");
    // The first match (notes.txt's line): just that one.
    keys(&mut app, "r");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Replaced 1 match in 1 file"),
        "{}",
        message(&app)
    );
    assert_eq!(app.editor().search_status(), "2 results in 1 file");
    // main.rs's row: all of the file's.
    keys(&mut app, "gg");
    keys(&mut app, "r");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Replaced 2 matches in 1 file"),
        "{}",
        message(&app)
    );
    assert_eq!(app.editor().search_status(), "No results");
}

#[test]
fn regex_replacements_expand_captures() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(&[("a.txt", "key=value\nname=ferra\n")]);
    let mut app = searching(r"(\w+)=(\w+)");
    keys(&mut app, "<C-r>");
    app.wait_for_background();
    assert_eq!(app.editor().search_status(), "2 results in 1 file");
    keys(&mut app, "<Tab>$2: $1");
    let rows = render(&mut app, 70, 20).rows.join("\n");
    assert!(rows.contains("key=valuevalue: key"), "{rows}");
    keys(&mut app, "<CR>y");
    app.wait_for_background();
    assert_eq!(buffer_text(&mut app, "a.txt"), "value: key\nferra: name");
}

#[test]
fn a_line_changed_since_the_search_isnt_replaced() {
    let _cwd = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let _project = project(FILES);
    let mut app = searching("alpha");
    keys(&mut app, "<Tab>x<Tab>");
    // Change notes.txt's line behind the search's back.
    fs::write("notes.txt", "Alpha was first\n").unwrap();
    keys(&mut app, "r");
    assert!(
        message(&app).contains("changed since the search"),
        "{}",
        message(&app)
    );
}
