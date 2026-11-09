//! Git end to end: a real repository, the gutter's marks, the status line,
//! `]c`/`[c`, and changes staged elsewhere showing up.

mod common;

use std::path::Path;
use std::process::{Command, Stdio};

use common::{app, cursor, keys, render};

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A repository with `code.txt` (five lines) committed on `main`.
fn repo() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().ok()?;
    if !git(dir.path(), &["init", "-q", "-b", "main"]) {
        return None;
    }
    std::fs::write(dir.path().join("code.txt"), "one\ntwo\nthree\nfour\nfive\n").ok()?;
    let ok = git(dir.path(), &["add", "."])
        && git(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "x",
            ],
        );
    ok.then_some(dir)
}

#[test]
fn edits_are_marked_in_the_gutter_and_counted_in_the_status_line() {
    let Some(dir) = repo() else {
        eprintln!("git isn't available: skipped");
        return;
    };
    let mut app = app();
    keys(
        &mut app,
        &format!(":e {}<CR>", dir.path().join("code.txt").display()),
    );
    let screen = render(&mut app, 60, 10);
    assert!(
        screen.rows.join("\n").contains("⎇ main"),
        "{}",
        screen.rows.join("\n")
    );
    assert!(!screen.rows.join("").contains('▎'), "nothing changed yet");

    // Change the first line, remove the third, add one at the end.
    keys(&mut app, "ccONE<Esc>jjddGosix<Esc>");
    let screen = render(&mut app, 60, 10);
    let rows = screen.rows.join("\n");
    assert!(screen.rows[0].contains("▎ONE"), "{rows}");
    assert!(screen.rows[1].contains("▁two"), "removed below it: {rows}");
    assert!(screen.rows[4].contains("▎six"), "{rows}");
    assert!(rows.contains("+1 ~1 -1"), "{rows}");

    keys(&mut app, "gg]c");
    assert_eq!(cursor(&app).0, 1);
    keys(&mut app, "]c");
    assert_eq!(cursor(&app).0, 4);
    keys(&mut app, "[c");
    assert_eq!(cursor(&app).0, 1);
}

#[test]
fn staging_elsewhere_shows_when_the_editor_comes_back() {
    let Some(dir) = repo() else {
        return;
    };
    let path = dir.path().join("code.txt");
    let mut app = app();
    keys(&mut app, &format!(":e {}<CR>", path.display()));
    keys(&mut app, "ccONE<Esc>:w<CR>");
    assert!(render(&mut app, 60, 10).rows[0].contains("▎ONE"));
    // `git add` in another terminal, then back to the editor.
    assert!(git(dir.path(), &["add", "code.txt"]));
    app.handle_focus_gained();
    assert!(
        !render(&mut app, 60, 10).rows[0].contains('▎'),
        "staged: no change"
    );
}

#[test]
fn outside_a_repository_there_is_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
    let mut app = common::app_with_config("[editor]\ngit = true");
    keys(
        &mut app,
        &format!(":e {}<CR>", dir.path().join("a.txt").display()),
    );
    keys(&mut app, "ccchanged<Esc>]c");
    let screen = render(&mut app, 60, 10);
    assert!(!screen.rows.join("").contains('▎'));
    assert_eq!(
        app.editor().message().map(|m| m.text.as_str()),
        Some("No changes")
    );
}

fn sidebar_text(app: &mut ferra::App) -> String {
    render(app, 70, 16)
        .rows
        .iter()
        .map(|r| r.chars().take(31).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn source_control_lists_stages_commits_and_discards() {
    let Some(dir) = repo() else {
        return;
    };
    let path = dir.path().join("code.txt");
    std::fs::write(dir.path().join("notes.md"), "notes\n").unwrap();
    let mut app = app();
    keys(&mut app, &format!(":e {}<CR>", path.display()));
    keys(&mut app, "ccONE<Esc>:w<CR>:git_view<CR>");
    let shown = sidebar_text(&mut app);
    assert!(shown.contains("Files   Git"), "{shown}");
    assert!(shown.contains("SOURCE CONTROL"), "{shown}");
    assert!(shown.contains("CHANGES (2)"), "{shown}");
    assert!(shown.contains("M code.txt"), "{shown}");
    assert!(shown.contains("U notes.md"), "{shown}");

    // The first file's selected: stage it, then commit.
    keys(&mut app, "s");
    let shown = sidebar_text(&mut app);
    assert!(
        shown.contains("STAGED (1)") && shown.contains("CHANGES (1)"),
        "{shown}"
    );
    keys(&mut app, "c");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Input);
    keys(&mut app, "Shout the first line<CR>");
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(
        message.starts_with("Committed ") && message.ends_with(": Shout the first line"),
        "{message}"
    );
    let shown = sidebar_text(&mut app);
    assert!(!shown.contains("STAGED"), "{shown}");
    assert!(
        !render(&mut app, 70, 16).rows[0].contains('▎'),
        "committed: no changes in the gutter"
    );

    // Discard asks first.
    keys(&mut app, "x");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Popup);
    keys(&mut app, "n");
    assert!(dir.path().join("notes.md").exists());
    keys(&mut app, "xy");
    assert!(!dir.path().join("notes.md").exists(), "untracked: deleted");
    assert!(sidebar_text(&mut app).contains("No changes"));
}

#[test]
fn committing_with_nothing_staged_says_so() {
    let Some(dir) = repo() else {
        return;
    };
    let mut app = app();
    keys(
        &mut app,
        &format!(":e {}<CR>", dir.path().join("code.txt").display()),
    );
    keys(&mut app, ":git_view<CR>c");
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(message.starts_with("Nothing staged"), "{message}");
}
