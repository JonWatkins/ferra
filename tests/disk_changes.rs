//! Files changed by other programs while open, end to end: reloading
//! unmodified buffers, warning about modified ones, and `:w`/`:w!`/`:e!`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{app_with_config, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// Writes `contents` as another program would, with a modified time
/// clearly after anything the editor has seen.
fn change_externally(path: &Path, contents: &str, seconds_later: u64) {
    fs::write(path, contents).unwrap();
    let later = SystemTime::now() + Duration::from_secs(seconds_later);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

fn setup(contents: &str) -> (tempfile::TempDir, PathBuf, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, contents).unwrap();
    let mut app = app_with_config("");
    app.open(&path);
    (dir, path, app)
}

#[test]
fn an_unmodified_buffer_reloads_when_focus_returns() {
    let (_dir, path, mut app) = setup("one\ntwo\nthree\n");
    keys(&mut app, "jj");
    change_externally(&path, "one\ntwo\nthree\nfour\n", 5);
    app.handle_focus_gained();
    assert_eq!(text(&app), "one\ntwo\nthree\nfour");
    assert_eq!(cursor(&app), (2, 0), "the cursor stays on its line");
    assert!(
        message(&app).ends_with("reloaded: changed on disk"),
        "{}",
        message(&app)
    );
    assert!(!app.editor().document.is_modified());

    keys(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo\nthree", "u undoes the reload");
}

#[test]
fn nothing_happens_when_nothing_changed() {
    let (_dir, _path, mut app) = setup("same\n");
    app.handle_focus_gained();
    assert_eq!(text(&app), "same");
    assert!(!message(&app).contains("reloaded"));
}

#[test]
fn a_modified_buffer_gets_a_warning_once() {
    let (_dir, path, mut app) = setup("mine\n");
    keys(&mut app, "A!<Esc>");
    change_externally(&path, "theirs\n", 5);
    app.handle_focus_gained();
    assert_eq!(text(&app), "mine!", "your changes are kept");
    assert_eq!(
        message(&app),
        format!(
            "\"{}\" changed on disk: :e! loads it, :w! keeps your version",
            path.display()
        )
    );

    keys(&mut app, ":noh<CR>");
    app.handle_focus_gained();
    assert!(!message(&app).contains("changed on disk"), "not repeated");

    keys(&mut app, ":w<CR>");
    assert!(
        message(&app).contains("has changed on disk"),
        "{}",
        message(&app)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "theirs\n");
}

#[test]
fn force_write_keeps_your_version() {
    let (_dir, path, mut app) = setup("mine\n");
    keys(&mut app, "A!<Esc>");
    change_externally(&path, "theirs\n", 5);
    app.handle_focus_gained();
    keys(&mut app, ":w!<CR>");
    assert_eq!(fs::read_to_string(&path).unwrap(), "mine!\n");
    app.handle_focus_gained();
    assert!(
        message(&app).ends_with("written"),
        "no warning after saving"
    );
}

#[test]
fn edit_bang_loads_theirs_and_u_brings_yours_back() {
    let (_dir, path, mut app) = setup("mine\n");
    keys(&mut app, "A!<Esc>");
    change_externally(&path, "theirs\n", 5);
    keys(&mut app, ":e!<CR>");
    assert_eq!(text(&app), "theirs");
    assert!(!app.editor().document.is_modified());
    keys(&mut app, "u");
    assert_eq!(text(&app), "mine!");
}

#[test]
fn a_deleted_file_is_reported_and_w_writes_it_again() {
    let (_dir, path, mut app) = setup("keep me\n");
    fs::remove_file(&path).unwrap();
    app.handle_focus_gained();
    assert_eq!(text(&app), "keep me", "the buffer is left alone");
    assert!(
        message(&app).contains("was deleted on disk"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":w<CR>");
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me\n");
}

#[test]
fn switching_to_a_buffer_checks_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "a1\n").unwrap();
    fs::write(&b, "b1\n").unwrap();
    let mut app = app_with_config("");
    app.open(&a);
    keys(&mut app, &format!(":e {}<CR>", b.display()));
    change_externally(&a, "a2\n", 5);
    keys(&mut app, ":b #<CR>");
    assert_eq!(text(&app), "a2");
    assert!(message(&app).contains("reloaded"), "{}", message(&app));
}

#[test]
fn checktime_reloads_every_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "a1\n").unwrap();
    fs::write(&b, "b1\n").unwrap();
    let mut app = app_with_config("");
    app.open(&a);
    keys(&mut app, &format!(":e {}<CR>", b.display()));
    change_externally(&a, "a2\n", 5);
    change_externally(&b, "b2\n", 5);
    keys(&mut app, ":checktime<CR>");
    assert_eq!(message(&app), "2 buffers reloaded: changed on disk");
    assert_eq!(text(&app), "b2");
    keys(&mut app, ":b #<CR>");
    assert_eq!(text(&app), "a2");
}

#[test]
fn a_file_created_for_a_new_buffer_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("later.txt");
    let mut app = app_with_config("");
    app.open(&path);
    assert_eq!(text(&app), "");
    change_externally(&path, "made elsewhere\n", 5);
    app.handle_focus_gained();
    assert_eq!(text(&app), "made elsewhere");
}

#[test]
fn marks_outside_the_changed_part_stay_put() {
    let (_dir, path, mut app) = setup("a\nb\nc\nd\n");
    keys(&mut app, "Gma");
    change_externally(&path, "a\nB\nc\nd\n", 5);
    app.handle_focus_gained();
    keys(&mut app, "gg'a");
    assert_eq!(cursor(&app), (3, 0));
}
