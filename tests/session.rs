//! Sessions: what was open when ferra quit in a folder, opened again when
//! it starts there with no files named. The folder is made the current
//! one, so these tests take turns.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use common::{app, app_with_config, cursor, keys, text};
use ferra::App;

static CWD: Mutex<()> = Mutex::new(());

struct Project {
    dir: tempfile::TempDir,
    sessions: tempfile::TempDir,
    old: PathBuf,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Project {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old).unwrap();
    }
}

impl Project {
    fn path(&self, name: &str) -> PathBuf {
        fs::canonicalize(self.dir.path()).unwrap().join(name)
    }

    fn app(&self, config: &str) -> App {
        let mut app = app_with_config(config);
        app.set_session_dir(Some(self.sessions.path().to_path_buf()));
        app
    }
}

fn project(files: &[(&str, &str)]) -> Project {
    let turn = CWD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    Project {
        dir,
        sessions: tempfile::tempdir().unwrap(),
        old,
        _turn: turn,
    }
}

#[test]
fn what_was_open_opens_again() {
    let project = project(&[("a.txt", "one\ntwo\nthree\n"), ("b.txt", "b\n")]);
    let mut app = project.app("");
    app.open(&project.path("a.txt"));
    let a = app.editor().document.id();
    keys(&mut app, "jl");
    app.open(&project.path("b.txt"));
    keys(&mut app, "<Space>e<Esc>");
    // a.txt shown when it quits.
    app.editor_mut().switch_to(a);
    assert_eq!(text(&app), "one\ntwo\nthree");
    app.save_session().unwrap();

    let mut again = project.app("");
    assert!(again.restore_session());
    assert_eq!(text(&again), "one\ntwo\nthree", "the one shown");
    assert_eq!(cursor(&again), (1, 1), "where its cursor was");
    assert_eq!(again.editor().buffer_count(), 2);
    assert!(again.editor().sidebar().is_some(), "the sidebar was open");
    let names: Vec<String> = again
        .editor()
        .buffers()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.iter().any(|n| n.ends_with("b.txt")), "{names:?}");
}

#[test]
fn files_gone_since_are_skipped_and_nothing_open_keeps_nothing() {
    let project = project(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let mut app = project.app("");
    app.open(&project.path("a.txt"));
    app.open(&project.path("b.txt"));
    app.save_session().unwrap();
    fs::remove_file(project.path("b.txt")).unwrap();
    let mut again = project.app("");
    assert!(again.restore_session());
    assert_eq!(again.editor().buffer_count(), 1);
    assert_eq!(text(&again), "a");

    // Nothing with a file open: no session.
    let app = project.app("");
    app.save_session().unwrap();
    assert!(!project.app("").restore_session());
}

#[test]
fn it_can_be_turned_off() {
    let project = project(&[("a.txt", "a\n")]);
    let mut app = project.app("[editor]\nrestore-session = false\n");
    app.open(&project.path("a.txt"));
    app.save_session().unwrap();
    assert_eq!(
        fs::read_dir(project.sessions.path()).unwrap().count(),
        0,
        "not kept"
    );
    assert!(!project.app("").restore_session());
}

#[test]
fn without_a_sessions_folder_nothing_is_kept() {
    let project = project(&[("a.txt", "a\n")]);
    let mut plain = app();
    plain.open(&project.path("a.txt"));
    plain.save_session().unwrap();
    assert_eq!(fs::read_dir(project.sessions.path()).unwrap().count(), 0);
}
