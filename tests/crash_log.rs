//! The log (`:log`) and what happens on a panic: it's written down with a
//! backtrace and the input before it, caught in the event loop, and a
//! panic on another thread is reported rather than taking the editor down.
//!
//! The log, its recent keys and the panic hook are global, so these
//! tests take turns.

mod common;

use common::{app, keys};
use ferra::app::crash;
use ferra::editor::Severity;

static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn turn() -> std::sync::MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn panics_and_errors_go_to_the_log() {
    let _turn = turn();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/ferra.log");
    ferra::log::init_at(&path);
    crash::install();
    let log = || std::fs::read_to_string(&path).unwrap_or_default();

    // Errors shown on the message line.
    let mut app = app();
    keys(&mut app, ":no_such_command<CR>");
    assert!(log().contains("WARN"), "{}", log());
    assert!(log().contains("no_such_command"), "{}", log());

    // A caught panic: its message, where, the input before it, and a
    // backtrace; the editor carries on.
    keys(&mut app, "ihello<Esc>");
    let caught: Result<(), String> = crash::catching(|| panic!("index out of bounds"));
    assert_eq!(caught, Err("index out of bounds".into()));
    let text = log();
    assert!(text.contains("ERROR panic in thread"), "{text}");
    assert!(text.contains("tests/crash_log.rs"), "where: {text}");
    assert!(text.contains("index out of bounds"), "{text}");
    assert!(text.contains("recent input:"), "{text}");
    assert!(
        text.contains("h e l l o <Esc>"),
        "the keys before it: {text}"
    );
    assert!(text.contains("backtrace:"), "{text}");
    // Continuation lines are indented, so entries stand out.
    assert!(
        text.lines()
            .filter(|l| !l.is_empty())
            .all(|l| l.starts_with("    ") || l.starts_with("20")),
        "{text}"
    );
    assert_eq!(crash::catching(|| 2 + 2), Ok(4));

    // Another thread's panic is logged and handed to the editor.
    let _ = std::thread::Builder::new()
        .name("lsp reader".into())
        .spawn(|| panic!("bad message"))
        .unwrap()
        .join();
    assert_eq!(
        crash::take_thread_panic().as_deref(),
        Some("lsp reader crashed: bad message")
    );
    assert!(log().contains("panic in thread lsp reader"));
    assert_eq!(crash::take_thread_panic(), None, "reported once");

    // What's typed into the terminal panel is never noted (a password).
    app.editor_mut().set_mode(ferra::editor::Mode::Terminal);
    keys(&mut app, "hunter2");
    app.editor_mut().set_mode(ferra::editor::Mode::Normal);
    let _ = crash::catching(|| panic!("after the terminal"));
    let entry = log();
    let entry = &entry[entry.rfind("after the terminal").unwrap()..];
    assert!(!entry.contains("h u n t e r"), "{entry}");

    // `:log` opens it, at the end.
    keys(&mut app, ":log<CR>");
    let editor = app.editor();
    assert_eq!(editor.document.path(), Some(path.as_path()));
    let last = editor.document.buffer().line_count() - 1;
    assert_eq!(editor.document.cursor().row, last);
    assert!(
        editor
            .message()
            .is_none_or(|m| m.severity != Severity::Error),
        "{:?}",
        editor.message()
    );
}

#[test]
fn the_state_dump_names_windows_and_buffers() {
    let _turn = turn();
    let mut app = app();
    keys(&mut app, "ihi<Esc>:vsplit<CR>:split<CR>");
    let state = app.editor().debug_state();
    assert!(state.contains("windows: 3"), "{state}");
    assert!(state.contains("layout"), "{state}");
    assert!(state.contains("[scratch] [+]"), "{state}");
    assert!(state.contains("(active)"), "{state}");
}

#[test]
fn recovery_saves_unsaved_buffers_only() {
    let _turn = turn();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "saved\n").unwrap();
    let mut app = app();
    app.open(&file);
    let recovery = dir.path().join("recovery");
    assert!(
        app.editor().save_recovery(&recovery).is_empty(),
        "nothing unsaved"
    );

    keys(&mut app, "ichanged <Esc>");
    let saved = app.editor().save_recovery(&recovery);
    assert_eq!(saved.len(), 1);
    assert!(
        saved[0].to_string_lossy().ends_with("-notes.txt"),
        "{saved:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&saved[0]).unwrap(),
        "changed saved\n"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "saved\n",
        "the file is untouched"
    );
}
