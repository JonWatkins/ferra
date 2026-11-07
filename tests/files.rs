//! Opening, editing and saving real files through the key interface.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{app, keys, render, text};
use ferra::App;
use ferra::editor::UNSAVED_CHANGES;

struct Workspace {
    dir: tempfile::TempDir,
}

impl Workspace {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, contents).unwrap();
        path
    }
}

fn open(path: &Path) -> App {
    let mut app = app();
    app.open(path);
    app
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// Types a `:` command, e.g. `cmd(&mut app, "w")`.
fn cmd(app: &mut App, command: &str) {
    keys(app, &format!(":{}<CR>", command.replace('<', "<lt>")));
}

#[test]
fn open_edit_save() {
    let ws = Workspace::new();
    let path = ws.file("notes.txt", "first\nsecond\n");
    let mut app = open(&path);
    assert_eq!(text(&app), "first\nsecond");
    assert!(message(&app).ends_with("2L, 13B"), "{}", message(&app));

    keys(&mut app, "jA!<Esc>");
    cmd(&mut app, "w");
    assert_eq!(read(&path), "first\nsecond!\n");
    assert!(
        message(&app).ends_with("2L, 14B written"),
        "{}",
        message(&app)
    );
}

#[test]
fn new_file_is_created_on_write() {
    let ws = Workspace::new();
    let path = ws.path("new.txt");
    let mut app = open(&path);
    assert!(message(&app).ends_with("[New]"));
    assert!(!path.exists(), "opening must not create the file");

    keys(&mut app, "ihello<Esc>");
    cmd(&mut app, "wq");
    assert_eq!(read(&path), "hello\n");
    assert!(app.should_quit());
}

#[test]
fn quit_protects_unsaved_changes() {
    let ws = Workspace::new();
    let path = ws.file("a.txt", "a\n");
    let mut app = open(&path);
    keys(&mut app, "x");

    cmd(&mut app, "q");
    assert!(!app.should_quit());
    assert_eq!(message(&app), UNSAVED_CHANGES);

    cmd(&mut app, "q!");
    assert!(app.should_quit());
    assert_eq!(read(&path), "a\n", "q! discards");
}

#[test]
fn undo_back_to_saved_state_allows_quit() {
    let ws = Workspace::new();
    let path = ws.file("a.txt", "abc\n");
    let mut app = open(&path);
    keys(&mut app, "xxuu");
    cmd(&mut app, "q");
    assert!(app.should_quit());
}

#[test]
fn zz_saves_and_quits_zq_discards() {
    let ws = Workspace::new();
    let path = ws.file("a.txt", "a\n");
    let mut app = open(&path);
    keys(&mut app, "ix<Esc>ZZ");
    assert!(app.should_quit());
    assert_eq!(read(&path), "xa\n");

    let mut app = open(&path);
    keys(&mut app, "dd");
    keys(&mut app, "ZQ");
    assert!(app.should_quit());
    assert_eq!(read(&path), "xa\n");
}

#[test]
fn write_scratch_to_new_path() {
    let ws = Workspace::new();
    let path = ws.path("from scratch.txt");
    let mut app = app();
    keys(&mut app, "idraft<Esc>");
    cmd(&mut app, "w");
    assert_eq!(message(&app), "No file name");

    cmd(&mut app, &format!("w {}", path.display()));
    assert_eq!(read(&path), "draft\n");
    cmd(&mut app, "q");
    assert!(
        app.should_quit(),
        "the scratch now belongs to the file and is saved"
    );
}

#[test]
fn write_refuses_to_clobber_unless_forced() {
    let ws = Workspace::new();
    let existing = ws.file("taken.txt", "precious\n");
    let mut app = app();
    keys(&mut app, "iover<Esc>");
    cmd(&mut app, &format!("w {}", existing.display()));
    assert!(
        message(&app).contains("already exists"),
        "{}",
        message(&app)
    );
    assert_eq!(read(&existing), "precious\n");

    cmd(&mut app, &format!("w! {}", existing.display()));
    assert_eq!(read(&existing), "over\n");
}

#[test]
fn edit_opens_buffers_and_keeps_unsaved_changes() {
    let ws = Workspace::new();
    let a = ws.file("a.txt", "aaa\n");
    let b = ws.file("b.txt", "bbb\n");
    let mut app = open(&a);

    // Editing another file keeps the modified buffer open in the background.
    keys(&mut app, "x");
    cmd(&mut app, &format!("e {}", b.display()));
    assert_eq!(text(&app), "bbb");
    assert_eq!(app.editor().buffer_count(), 2);

    // Its changes are still there, and still protected by :q.
    cmd(&mut app, "q");
    assert!(!app.should_quit());
    assert!(message(&app).contains("for buffer 1"), "{}", message(&app));
    cmd(&mut app, "b a.txt");
    assert_eq!(text(&app), "aa");

    // :e reloads the current file, refusing to discard changes without !.
    cmd(&mut app, "e");
    assert_eq!(message(&app), UNSAVED_CHANGES);
    cmd(&mut app, "e!");
    assert_eq!(text(&app), "aaa", "e! reloads from disk");

    // Opening an already open file switches to it rather than reopening.
    cmd(&mut app, &format!("e {}", b.display()));
    assert_eq!(app.editor().buffer_count(), 2);
    assert_eq!(text(&app), "bbb");
}

#[test]
fn line_endings_and_missing_final_newline_are_preserved() {
    let ws = Workspace::new();
    let path = ws.file("win.txt", "one\r\ntwo");
    let mut app = open(&path);
    assert!(
        message(&app).ends_with("[CRLF] [noeol]"),
        "{}",
        message(&app)
    );

    keys(&mut app, "Go2<Esc>");
    cmd(&mut app, "w");
    assert_eq!(read(&path), "one\r\ntwo\r\n2");
}

#[test]
fn non_utf8_files_open_with_the_fallback_and_save_back_exactly() {
    let ws = Workspace::new();
    let path = ws.path("latin.txt");
    let bytes = [b'c', b'a', b'f', 0xe9, b'\n'];
    fs::write(&path, bytes).unwrap();
    let mut app = open(&path);
    assert_eq!(text(&app), "café");
    assert!(
        message(&app).contains("[windows-1252]"),
        "{}",
        message(&app)
    );
    keys(&mut app, "A!<Esc>");
    cmd(&mut app, "w");
    assert_eq!(
        fs::read(&path).unwrap(),
        [b'c', b'a', b'f', 0xe9, b'!', b'\n']
    );
}

#[test]
fn a_utf8_fallback_refuses_other_files() {
    let ws = Workspace::new();
    let path = ws.path("image.bin");
    fs::write(&path, [0x89, b'P', b'N', b'G', 0xff]).unwrap();
    let mut app = common::app_with_config("[editor]\nfallback-encoding = \"utf-8\"");
    app.open(&path);
    assert!(
        message(&app).contains("not valid UTF-8"),
        "{}",
        message(&app)
    );
    assert_eq!(text(&app), "");
    cmd(&mut app, "w");
    assert_eq!(
        message(&app),
        "No file name",
        "must not offer to overwrite it"
    );
}

#[test]
fn byte_order_marks_and_utf16_round_trip() {
    let ws = Workspace::new();
    let bom = ws.path("bom.txt");
    fs::write(&bom, b"\xEF\xBB\xBFhello\n").unwrap();
    let mut app = open(&bom);
    assert_eq!(text(&app), "hello", "the mark isn't in the text");
    keys(&mut app, "A!<Esc>");
    cmd(&mut app, "w");
    assert_eq!(fs::read(&bom).unwrap(), b"\xEF\xBB\xBFhello!\n");

    let wide = ws.path("wide.txt");
    fs::write(&wide, b"\xFF\xFEh\x00i\x00").unwrap();
    let mut app = open(&wide);
    assert_eq!(text(&app), "hi");
    keys(&mut app, "A!<Esc>");
    cmd(&mut app, "w");
    assert_eq!(fs::read(&wide).unwrap(), b"\xFF\xFEh\x00i\x00!\x00");
}

#[test]
fn fileencoding_converts_and_counts_as_a_change() {
    let ws = Workspace::new();
    let path = ws.path("latin.txt");
    fs::write(&path, [0xe9, b'\n']).unwrap();
    let mut app = open(&path);
    cmd(&mut app, "fenc");
    assert_eq!(message(&app), "windows-1252");
    cmd(&mut app, "fenc utf-8");
    assert!(app.editor().document.is_modified());
    cmd(&mut app, "q");
    assert_eq!(message(&app), UNSAVED_CHANGES, "converting isn't saved yet");
    cmd(&mut app, "w");
    assert_eq!(fs::read(&path).unwrap(), "é\n".as_bytes());
    assert!(!app.editor().document.is_modified());
}

#[test]
fn characters_the_encoding_lacks_stop_the_save() {
    let ws = Workspace::new();
    let path = ws.path("latin.txt");
    fs::write(&path, [0xe9, b'\n']).unwrap();
    let mut app = open(&path);
    keys(&mut app, "A 日<Esc>");
    cmd(&mut app, "w");
    assert!(message(&app).contains("has no '日'"), "{}", message(&app));
    assert_eq!(
        fs::read(&path).unwrap(),
        [0xe9, b'\n'],
        "the file is untouched"
    );
    cmd(&mut app, "fenc utf-8");
    cmd(&mut app, "w");
    assert_eq!(fs::read_to_string(&path).unwrap(), "é 日\n");
}

#[test]
fn unknown_encodings_are_reported() {
    let mut app = app();
    cmd(&mut app, "fenc klingon");
    assert_eq!(message(&app), "Unknown encoding: klingon");
    let error = ferra::config::Config::from_toml("[editor]\nfallback-encoding = \"klingon\"")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown encoding: klingon"), "{error}");
}

#[test]
fn statusline_shows_file_name_and_modified_marker() {
    let ws = Workspace::new();
    let path = ws.file("shown.txt", "x\n");
    let mut app = open(&path);
    let status = render(&mut app, 80, 5).rows[3].clone();
    assert!(status.contains("shown.txt"), "{status:?}");
    assert!(!status.contains("[+]"), "{status:?}");

    keys(&mut app, "x");
    let status = render(&mut app, 80, 5).rows[3].clone();
    assert!(status.contains("shown.txt [+]"), "{status:?}");

    cmd(&mut app, "w");
    let status = render(&mut app, 80, 5).rows[3].clone();
    assert!(!status.contains("[+]"), "{status:?}");
}

#[cfg(unix)]
#[test]
fn saving_a_hard_linked_file_updates_every_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    let other = dir.path().join("same-notes.txt");
    fs::write(&path, "first\n").unwrap();
    fs::hard_link(&path, &other).unwrap();

    let mut app = open(&path);
    keys(&mut app, "A!<Esc>");
    cmd(&mut app, "w");
    assert!(message(&app).ends_with("written"), "{}", message(&app));
    assert_eq!(read(&other), "first!\n");

    // A second save still sees no change on disk it didn't make.
    keys(&mut app, "A?<Esc>");
    cmd(&mut app, "w");
    assert_eq!(read(&other), "first!?\n");
}
