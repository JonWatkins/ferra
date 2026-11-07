//! Event hooks end-to-end: `[hooks]` in config, and hooks registered in
//! Rust the way plugins will.

mod common;

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use common::{app, app_with_config, keys, text};
use ferra::App;
use ferra::editor::Mode;
use ferra::events::{Event, EventKind};

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// A shared log hooks can push to, and a test can read back.
fn log() -> Rc<RefCell<Vec<String>>> {
    Rc::new(RefCell::new(Vec::new()))
}

#[test]
fn config_before_save_hook_trims_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "keep  \nme\t\n").unwrap();

    let mut app = app_with_config(
        r#"
        [hooks]
        before-save = ["trim_trailing_whitespace"]
        "#,
    );
    app.open(&path);
    keys(&mut app, ":w<CR>");
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep\nme\n");
    assert!(!app.editor().document.is_modified());
}

#[test]
fn unknown_commands_in_hooks_are_reported() {
    let app = app_with_config(
        r#"
        [hooks]
        after-save = ["not_a_command"]
        "#,
    );
    assert_eq!(
        message(&app),
        "Hooks reference unknown commands: not_a_command (after-save)"
    );
}

#[test]
fn failing_before_save_hook_cancels_the_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "original\n").unwrap();

    let mut app = app();
    app.commands_mut()
        .on(EventKind::BeforeSave, "linter", |cx, _| {
            if cx.editor.document.buffer().text().contains("TODO") {
                Err("refusing to save a TODO".into())
            } else {
                Ok(())
            }
        });
    app.open(&path);
    keys(&mut app, "OTODO<Esc>:w<CR>");
    assert_eq!(
        message(&app),
        "before-save hook (linter): refusing to save a TODO"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    assert!(app.editor().document.is_modified());
}

#[test]
fn opened_and_after_save_hooks_get_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let seen = log();

    let mut app = app();
    for kind in [EventKind::Opened, EventKind::AfterSave] {
        let seen = seen.clone();
        app.commands_mut().on(kind, "test", move |_, event| {
            seen.borrow_mut().push(format!("{event:?}"));
            Ok(())
        });
    }
    app.open(&path);
    keys(&mut app, "ix<Esc>:w<CR>");

    let path_debug = format!("{path:?}");
    assert_eq!(
        *seen.borrow(),
        [
            format!("Opened {{ path: {path_debug} }}"),
            format!("AfterSave {{ path: {path_debug} }}"),
        ]
    );
}

#[test]
fn mode_changed_hooks_see_each_transition() {
    let seen = log();
    let mut app = app();
    let hook_log = seen.clone();
    app.commands_mut()
        .on(EventKind::ModeChanged, "test", move |_, event| {
            if let Event::ModeChanged { from, to } = event {
                hook_log.borrow_mut().push(format!("{from}->{to}"));
            }
            Ok(())
        });
    keys(&mut app, "ia<Esc>v<Esc>");
    assert_eq!(
        *seen.borrow(),
        [
            "NORMAL->INSERT",
            "INSERT->NORMAL",
            "NORMAL->VISUAL",
            "VISUAL->NORMAL"
        ]
    );
}

#[test]
fn buffer_changed_fires_once_per_editing_key() {
    let count = Rc::new(RefCell::new(0));
    let mut app = app();
    let hook_count = count.clone();
    app.commands_mut()
        .on(EventKind::BufferChanged, "test", move |_, _| {
            *hook_count.borrow_mut() += 1;
            Ok(())
        });
    keys(&mut app, "iab<Esc>");
    assert_eq!(*count.borrow(), 2, "one per typed character");
    keys(&mut app, "hlj0");
    assert_eq!(*count.borrow(), 2, "motions don't change the buffer");
    keys(&mut app, "x");
    assert_eq!(*count.borrow(), 3);
}

#[test]
fn hook_edits_are_their_own_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let mut app = app();
    app.commands_mut()
        .on(EventKind::AfterSave, "stamp", |cx, _| {
            cx.editor.document.insert(Default::default(), "saved ");
            Ok(())
        });
    app.open(&path);
    keys(&mut app, "itext<Esc>:w<CR>");
    assert_eq!(text(&app), "saved text");
    keys(&mut app, "u");
    assert_eq!(text(&app), "text", "undo removes only the hook's edit");
}

#[test]
fn hooks_that_trigger_each_other_are_stopped() {
    let mut app = app();
    app.commands_mut()
        .on(EventKind::ModeChanged, "ping-pong", |cx, _| {
            let next = if cx.editor.mode() == Mode::Normal {
                Mode::Insert
            } else {
                Mode::Normal
            };
            cx.editor.set_mode(next);
            Ok(())
        });
    keys(&mut app, "i");
    assert!(
        message(&app).contains("Hooks kept triggering each other"),
        "{}",
        message(&app)
    );
}
