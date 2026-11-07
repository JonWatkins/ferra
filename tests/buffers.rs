//! Multiple buffers and the picker popup, end to end.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{app, cursor, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

struct Workspace {
    dir: tempfile::TempDir,
}

impl Workspace {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    /// An app with `one.txt`, `two.txt` and `three.txt` open, `one.txt` shown.
    fn three_files(&self) -> App {
        let paths = [
            self.file("one.txt", "one\n"),
            self.file("two.txt", "two\n"),
            self.file("three.txt", "three\n"),
        ];
        let mut app = app();
        app.open_all(&paths);
        app
    }
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

#[test]
fn opening_several_files_shows_the_first() {
    let ws = Workspace::new();
    let app = ws.three_files();
    assert_eq!(app.editor().buffer_count(), 3);
    assert_eq!(text(&app), "one");
}

#[test]
fn bracket_b_cycles_buffers() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "]b");
    assert_eq!(text(&app), "two");
    keys(&mut app, "]b]b");
    assert_eq!(text(&app), "one", "wraps around");
    keys(&mut app, "[b");
    assert_eq!(text(&app), "three");
    keys(&mut app, "2]b");
    assert_eq!(text(&app), "two");
}

#[test]
fn colon_buffer_commands() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, ":b 3<CR>");
    assert_eq!(text(&app), "three");
    keys(&mut app, ":b two<CR>");
    assert_eq!(text(&app), "two");
    keys(&mut app, ":b #<CR>");
    assert_eq!(text(&app), "three", "alternate buffer");
    keys(&mut app, ":bn<CR>");
    assert_eq!(text(&app), "one");
}

#[test]
fn each_buffer_keeps_its_cursor_undo_and_scroll() {
    let ws = Workspace::new();
    let long = (1..=100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = app();
    app.open_all(&[ws.file("long.txt", &long), ws.file("short.txt", "short\n")]);

    // 8 text rows with scroll-off 3: line 60 scrolls the view to start at
    // line 56, and moving up one line (to 59) doesn't scroll it back.
    // (The view scrolls when drawn, so draw at line 60 before moving up.)
    keys(&mut app, "60Gx");
    render(&mut app, 20, 10);
    keys(&mut app, "k");
    let before = render(&mut app, 20, 10);
    assert!(
        before.rows[0].ends_with("56             "),
        "{:?}",
        before.rows[0]
    );

    keys(&mut app, "]b");
    assert_eq!(cursor(&app), (0, 0));
    render(&mut app, 20, 10);
    keys(&mut app, "]b");
    assert_eq!(cursor(&app), (58, 0), "cursor kept");

    // A fresh view would scroll differently for line 59 (starting at 55),
    // so seeing 56 at the top means the buffer's scroll position was kept.
    let after = render(&mut app, 20, 10);
    assert_eq!(after.rows[0], before.rows[0]);

    keys(&mut app, "u");
    assert_eq!(
        app.editor().document.buffer().line(59),
        "60",
        "undo history kept"
    );
}

#[test]
fn space_b_opens_the_buffer_picker() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "]b");
    keys(&mut app, "<Space>b");
    assert_eq!(app.editor().mode(), Mode::Picker);

    let screen = render(&mut app, 60, 20);
    let popup = screen.rows.join("\n");
    assert!(popup.contains("Buffers"), "{popup}");
    assert!(
        popup.contains("one.txt") && popup.contains("three.txt"),
        "{popup}"
    );
    assert!(popup.contains("3/3"), "{popup}");

    // Starts on the alternate buffer, so Enter toggles back.
    keys(&mut app, "<CR>");
    assert_eq!(text(&app), "one");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn typing_filters_the_buffer_picker() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "<Space>bthree<CR>");
    assert_eq!(text(&app), "three");
}

#[test]
fn picker_navigation_and_escape() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "<Space>b<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(text(&app), "one", "nothing changed");

    keys(&mut app, ":ls<CR>");
    assert_eq!(app.editor().mode(), Mode::Picker, ":ls opens the picker");
    // Starts on the alternate buffer, `three` (row 3): down wraps to one,
    // two, three, then up to two.
    keys(&mut app, "<C-n><C-n><Down><Up><CR>");
    assert_eq!(text(&app), "two");
}

#[test]
fn command_palette_runs_commands() {
    let mut app = app();
    keys(&mut app, "iab  <Esc>");
    keys(&mut app, "<Space>ptrim_trail<CR>");
    assert_eq!(text(&app), "ab");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn command_palette_shows_descriptions_and_highlights_matches() {
    let mut app = app();
    keys(&mut app, "<Space>pbufnext");
    let screen = render(&mut app, 100, 20);
    let popup = screen.rows.join("\n");
    assert!(popup.contains("Commands"), "{popup}");
    assert!(popup.contains("buffer_next"), "{popup}");
    assert!(popup.contains("Switch to the next buffer"), "{popup}");
    assert!(popup.contains("> bufnext"), "{popup}");
}

#[test]
fn picker_choices_are_not_repeated_by_dot() {
    let mut app = app();
    keys(&mut app, "iab  <Esc>x");
    keys(&mut app, "<Space>ptrim_trail<CR>");
    keys(&mut app, "0.");
    assert_eq!(text(&app), "b", "repeats x, not the palette command");
}

#[test]
fn buffer_delete_and_write_all() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "A!<Esc>]bA?<Esc>");
    keys(&mut app, ":bd<CR>");
    assert!(message(&app).contains("No write since last change"));
    keys(&mut app, ":wa<CR>");
    assert_eq!(message(&app), "All buffers written");
    assert_eq!(
        fs::read_to_string(ws.dir.path().join("one.txt")).unwrap(),
        "one!\n"
    );
    assert_eq!(
        fs::read_to_string(ws.dir.path().join("two.txt")).unwrap(),
        "two?\n"
    );

    keys(&mut app, ":bd<CR>");
    assert_eq!(app.editor().buffer_count(), 2);
    assert_eq!(text(&app), "one!", "most recently used buffer shown");
}

#[test]
fn quit_protects_changes_in_hidden_buffers() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "x]b:q<CR>");
    assert!(!app.should_quit());
    assert!(message(&app).contains("buffer 1"), "{}", message(&app));
    keys(&mut app, ":wqa<CR>");
    assert!(app.should_quit());
    assert_eq!(
        fs::read_to_string(ws.dir.path().join("one.txt")).unwrap(),
        "ne\n"
    );
}

#[test]
fn statusline_shows_the_active_buffer() {
    let ws = Workspace::new();
    let mut app = ws.three_files();
    keys(&mut app, "]b");
    let status = render(&mut app, 120, 5).rows[3].clone();
    assert!(status.contains("two.txt"), "{status:?}");
}

#[test]
fn space_f_lists_project_files_respecting_gitignore() {
    // Tests run from the crate root, so this lists this repository.
    let mut app = app();
    keys(&mut app, "<Space>f");
    app.wait_for_background();
    assert_eq!(app.editor().mode(), Mode::Picker);
    let picker = app.editor().picker().unwrap();
    let labels: Vec<&str> = picker
        .matches()
        .iter()
        .map(|m| picker.item(m).label.as_str())
        .collect();
    assert!(labels.contains(&"Cargo.toml"), "{labels:?}");
    assert!(labels.contains(&"src/main.rs"), "{labels:?}");
    assert!(
        !labels.iter().any(|l| l.starts_with("target/")),
        "gitignored"
    );
    assert!(!labels.iter().any(|l| l.starts_with(".git/")), "never .git");
}

#[test]
fn file_picker_opens_the_chosen_file_in_a_new_buffer() {
    let ws = Workspace::new();
    ws.file("notes.md", "notes\n");
    fs::create_dir_all(ws.dir.path().join("src")).unwrap();
    fs::write(ws.dir.path().join("src/lib.rs"), "lib\n").unwrap();

    let mut app = app();
    keys(&mut app, "iscratch<Esc>");
    let root = ws.dir.path().display().to_string();
    keys(
        &mut app,
        &format!(":file_picker {}<CR>", root.replace('<', "<lt>")),
    );
    app.wait_for_background();
    let screen = render(&mut app, 80, 20);
    assert!(screen.rows.join("\n").contains("Files"));

    keys(&mut app, "lib<CR>");
    assert_eq!(text(&app), "lib");
    assert_eq!(app.editor().buffer_count(), 2, "the scratch buffer is kept");
}
