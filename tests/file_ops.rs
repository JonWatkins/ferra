//! New files, saving one with no name, the file tree's new file and folder,
//! rename and bin, and the right-click menus. The project is a temporary
//! folder made the current one (and HOME, for the bin, a temporary one),
//! so these tests run one at a time.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use common::{app, app_with_config, keys, text};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::clipboard::MemoryClipboard;
use ferra::editor::Mode;

static CWD: Mutex<()> = Mutex::new(());

struct Project {
    dir: tempfile::TempDir,
    home: tempfile::TempDir,
    old: PathBuf,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl Project {
    fn path(&self, name: &str) -> PathBuf {
        fs::canonicalize(self.dir.path()).unwrap().join(name)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old).unwrap();
    }
}

fn project(files: &[(&str, &str)]) -> Project {
    let turn = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let home = tempfile::tempdir().unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    // SAFETY: the tests here take turns (CWD), and nothing else in this
    // process reads HOME at the same time.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
    }
    Project {
        dir,
        home,
        old,
        _turn: turn,
    }
}

/// Selects the file tree's row named `name`.
fn select(app: &mut App, name: &str) {
    keys(app, "gg");
    for _ in 0..30 {
        let tree = app.editor().sidebar().unwrap();
        if tree.selected_row().is_some_and(|r| r.name == name) {
            return;
        }
        keys(app, "j");
    }
    panic!("no {name} in the tree");
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
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

#[test]
fn a_new_file_is_named_when_saved() {
    let project = project(&[]);
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\nauto-indent = false\n");
    keys(&mut app, "<C-n>hello<C-s>");
    assert_eq!(app.editor().mode(), Mode::Input, "asks for a name");
    keys(&mut app, "notes.txt<CR>");
    let path = project.path("notes.txt");
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello\n");
    assert!(app.editor().document.path().is_some());
    assert_eq!(app.editor().mode(), Mode::Insert);
    // Now it has a name, Ctrl+S just saves.
    keys(&mut app, " there<C-s>");
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello there\n");
    assert_eq!(app.editor().mode(), Mode::Insert);
}

#[test]
fn the_tree_makes_files_and_folders() {
    let project = project(&[("src/main.rs", "fn main() {}\n")]);
    let mut app = app();
    keys(&mut app, "<Space>e");
    select(&mut app, "src");
    keys(&mut app, "a");
    assert_eq!(app.editor().mode(), Mode::Input);
    assert!(
        app.editor().input_box().unwrap().title.contains("src/"),
        "in the folder"
    );
    keys(&mut app, "lib.rs<CR>");
    let path = project.path("src/lib.rs");
    assert!(path.is_file());
    assert_eq!(app.editor().document.path(), Some(path.as_path()), "opened");
    assert_eq!(app.editor().mode(), Mode::Normal);

    // A folder (and folders on the way, with a /).
    keys(&mut app, "<Space>e");
    select(&mut app, "src");
    keys(&mut app, "Atests/unit<CR>");
    assert!(project.path("src/tests/unit").is_dir());
    assert_eq!(app.editor().mode(), Mode::Sidebar, "stays in the tree");
    // A name that's taken.
    select(&mut app, "lib.rs");
    keys(&mut app, "amain.rs<CR>");
    assert!(
        message(&app).contains("already exists"),
        "{}",
        message(&app)
    );
}

#[test]
fn renaming_takes_open_buffers_along() {
    let project = project(&[("a.txt", "a\n")]);
    let mut app = app();
    app.open(&project.path("a.txt"));
    keys(&mut app, "<Space>e");
    select(&mut app, "a.txt");
    keys(&mut app, "m");
    let input = app.editor().input_box().unwrap();
    assert_eq!(input.field.text(), "a.txt");
    assert!(input.field.is_selected(), "typing replaces it");
    keys(&mut app, "b.txt<CR>");
    assert!(!project.path("a.txt").exists());
    assert!(project.path("b.txt").is_file());
    assert_eq!(
        app.editor().document.path(),
        Some(project.path("b.txt").as_path())
    );
    assert_eq!(
        app.editor().sidebar().unwrap().selected_row().unwrap().name,
        "b.txt"
    );
}

#[test]
fn the_bin_asks_first() {
    let project = project(&[("a.txt", "a\n"), ("keep.txt", "")]);
    let mut app = app();
    keys(&mut app, "<Space>e");
    select(&mut app, "a.txt");
    keys(&mut app, "d");
    assert_eq!(app.editor().mode(), Mode::Popup);
    keys(&mut app, "n");
    assert!(project.path("a.txt").exists(), "n: kept");
    keys(&mut app, "dy");
    assert!(!project.path("a.txt").exists());
    let bin: PathBuf = if cfg!(target_os = "macos") {
        project.home.path().join(".Trash")
    } else {
        project.home.path().join("data/Trash/files")
    };
    assert_eq!(
        fs::read_to_string(bin.join("a.txt")).unwrap(),
        "a\n",
        "in the bin"
    );
    assert!(
        message(&app).contains("Moved a.txt to the bin"),
        "{}",
        message(&app)
    );
    assert!(project.path("keep.txt").exists());
}

#[test]
fn paths_are_copied() {
    let project = project(&[("src/a.txt", "")]);
    let clipboard = MemoryClipboard::default();
    let mut app = app();
    app.use_clipboard(Box::new(clipboard.clone()));
    keys(&mut app, "<Space>e");
    select(&mut app, "src");
    keys(&mut app, "l");
    select(&mut app, "a.txt");
    keys(&mut app, "y");
    assert_eq!(
        clipboard.contents(),
        Path::new("src").join("a.txt").to_string_lossy()
    );
    keys(&mut app, "Y");
    assert_eq!(
        clipboard.contents(),
        project.path("src/a.txt").to_string_lossy()
    );
}

#[test]
fn the_trees_keys_are_for_the_tree_only() {
    let _project = project(&[("a.txt", "")]);
    let mut app = app();
    keys(&mut app, "<Space>F");
    assert!(app.editor().search_view_shown());
    app.editor_mut().set_mode(Mode::Sidebar);
    keys(&mut app, "<Esc>:tree_new_file<CR>");
    assert!(message(&app).contains("file tree"), "{}", message(&app));
}

#[test]
fn right_click_menus() {
    let project = project(&[("a.txt", "hello\n")]);
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\n");
    app.open(&project.path("a.txt"));
    common::render(&mut app, 80, 20);
    // In the text: Go to Definition and the rest, where the click was.
    click(&mut app, MouseButton::Right, 4, 0);
    let popup = app.editor().popup().unwrap();
    assert_eq!(popup.name, "menu");
    assert!(format!("{:?}", popup.lines).contains("Go to Definition"));
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Insert);

    // In the file tree: on the entry clicked, then New File runs from it.
    keys(&mut app, "<C-b>");
    let shown = common::render(&mut app, 80, 20);
    let y = shown
        .rows
        .iter()
        .position(|r| r.contains("a.txt") && !r.contains('1'))
        .unwrap() as u16;
    click(&mut app, MouseButton::Right, 3, y);
    let popup = app.editor().popup().unwrap();
    assert!(format!("{:?}", popup.lines).contains("New File"));
    keys(&mut app, "<CR>");
    assert_eq!(app.editor().mode(), Mode::Input, "New File asks for a name");
    keys(&mut app, "b.txt<CR>");
    assert!(project.path("b.txt").is_file());
    assert_eq!(text(&app), "");
}

#[test]
fn right_clicking_below_the_files_makes_them_in_the_root() {
    let project = project(&[("src/a.txt", "")]);
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\n");
    keys(&mut app, "<C-b>");
    let shown = common::render(&mut app, 80, 20);
    let last = shown.rows.iter().rposition(|r| r.contains("src")).unwrap() as u16;
    click(&mut app, MouseButton::Right, 3, last + 3);
    let popup = app.editor().popup().unwrap();
    assert!(format!("{:?}", popup.lines).contains("New File"));
    keys(&mut app, "<CR>");
    let input = app.editor().input_box().unwrap();
    assert_eq!(input.title, "New file", "in the project's folder");
    keys(&mut app, "top.txt<CR>");
    assert!(project.path("top.txt").is_file());
}

#[test]
fn save_as_completes_paths_with_tab() {
    let project = project(&[("src/ui/x.rs", ""), ("src/util.rs", ""), ("docs/a.md", "")]);
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\n");
    keys(&mut app, "<C-n>x<C-s>");
    let field = |app: &App| app.editor().input_box().unwrap().field.text().to_string();
    keys(&mut app, "s<Tab>");
    assert_eq!(field(&app), "src/", "the one match");
    // Two matches, agreeing on nothing more: the first, and what there is.
    keys(&mut app, "u<Tab>");
    assert_eq!(field(&app), "src/ui/");
    assert!(message(&app).contains("ui/  util.rs"), "{}", message(&app));
    keys(&mut app, "<Tab>");
    assert_eq!(field(&app), "src/util.rs", "then each in turn");
    keys(&mut app, "<S-Tab>");
    assert_eq!(field(&app), "src/ui/");
    keys(&mut app, "new.rs<CR>");
    assert_eq!(
        fs::read_to_string(project.path("src/ui/new.rs")).unwrap(),
        "x\n"
    );
}

#[test]
fn ctrl_tab_goes_through_the_buffers() {
    let project = project(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\n");
    app.open(&project.path("a.txt"));
    app.open(&project.path("b.txt"));
    keys(&mut app, "<C-Tab>");
    assert_eq!(text(&app), "a");
    keys(&mut app, "<C-S-Tab>");
    assert_eq!(text(&app), "b");
}

/// Where `text` is on screen.
fn find(app: &mut App, text: &str) -> (u16, u16) {
    let rows = common::render(app, 80, 20).rows;
    for (y, row) in rows.iter().enumerate() {
        if let Some(byte) = row.find(text) {
            return (row[..byte].chars().count() as u16, y as u16);
        }
    }
    panic!("{text:?} isn't on screen:\n{}", rows.join("\n"));
}

#[test]
fn confirmations_take_clicks() {
    let project = project(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\n");
    app.open(&project.path("a.txt"));
    keys(&mut app, "x<C-q>");
    // The question itself does nothing.
    let (x, y) = find(&mut app, "Save the changes");
    click(&mut app, MouseButton::Left, x + 2, y);
    assert_eq!(app.editor().mode(), Mode::Popup);
    // "Esc cancel": as Esc.
    let (x, y) = find(&mut app, "cancel");
    click(&mut app, MouseButton::Left, x, y);
    assert_eq!(app.editor().mode(), Mode::Insert);
    assert!(!app.editor().should_quit());
    // "d don't save": the key's words count too.
    keys(&mut app, "<C-q>");
    let (x, y) = find(&mut app, "d don't save");
    click(&mut app, MouseButton::Left, x + 5, y);
    assert!(app.editor().should_quit());
    assert_eq!(fs::read_to_string(project.path("a.txt")).unwrap(), "a\n");

    // The bin's question: "y move it".
    let mut app = common::app();
    keys(&mut app, "<Space>e");
    select(&mut app, "b.txt");
    keys(&mut app, "d");
    let (x, y) = find(&mut app, "move it");
    click(&mut app, MouseButton::Left, x, y);
    assert!(!project.path("b.txt").exists());
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
fn dragging_in_the_tree_moves_after_asking() {
    let project = project(&[("notes.txt", "n\n"), ("docs/readme.md", "r\n")]);
    let mut app = app();
    app.open(&project.path("notes.txt"));
    keys(&mut app, "<Space>e");
    let from = find(&mut app, "notes.txt");
    let onto = find(&mut app, "docs");
    drag(&mut app, from, onto);
    let shown = common::render(&mut app, 80, 20).rows.join("\n");
    assert!(shown.contains("Move notes.txt into docs?"), "{shown}");
    keys(&mut app, "y");
    assert!(project.path("docs/notes.txt").exists());
    assert!(!project.path("notes.txt").exists());
    // The open buffer went with it.
    assert!(
        app.editor()
            .document
            .path()
            .unwrap()
            .ends_with("docs/notes.txt")
    );
    assert_eq!(message(&app), "Moved notes.txt to docs/notes.txt");

    // Below the files: back to the project's folder.
    let from = find(&mut app, "notes.txt");
    drag(&mut app, from, (from.0, 18));
    let shown = common::render(&mut app, 80, 20).rows.join("\n");
    assert!(shown.contains("into the project's folder?"), "{shown}");
    keys(&mut app, "y");
    assert!(project.path("notes.txt").exists());

    // Onto where it already is: nothing to ask.
    let from = find(&mut app, "readme.md");
    let onto = find(&mut app, "docs");
    drag(&mut app, from, onto);
    assert!(app.editor().popup().is_none());
}

#[test]
fn a_file_dragged_from_the_tree_opens_in_that_window() {
    let project = project(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let mut app = app();
    app.open(&project.path("a.txt"));
    keys(&mut app, ":vs<CR><C-w>h<Space>e");
    let from = find(&mut app, "b.txt");
    // The right-hand window, well clear of the tree.
    drag(&mut app, from, (70, 10));
    assert_eq!(text(&app), "b");
    assert_ne!(app.editor().windows().active(), ferra::editor::WindowId(0));
}

#[test]
fn the_tree_shows_files_made_and_deleted_outside_ferra() {
    let project = project(&[("a.txt", "a\n")]);
    let mut app = app();
    keys(&mut app, "<Space>e");
    let names = |app: &App| -> Vec<String> {
        app.editor()
            .sidebar()
            .unwrap()
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect()
    };
    assert!(!names(&app).contains(&"new.md".to_string()));
    // As the agent (or a command) would.
    fs::write(project.path("new.md"), "# new\n").unwrap();
    app.check_files_while_idle();
    assert!(names(&app).contains(&"new.md".to_string()), "{:?}", names(&app));
    fs::remove_file(project.path("a.txt")).unwrap();
    app.check_files_while_idle();
    assert!(!names(&app).contains(&"a.txt".to_string()), "{:?}", names(&app));
}
