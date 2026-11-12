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

    // The first file (code.txt): stage it, then commit.
    keys(&mut app, "ggs");
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
    let screen = render(&mut app, 70, 16).rows.join("\n");
    assert!(
        !screen.lines().next().unwrap().contains('▎'),
        "committed: no changes in the gutter\n{screen}"
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

#[test]
fn the_git_view_stages_a_hunk_commits_and_shows_commits() {
    let Some(dir) = repo() else {
        return;
    };
    let path = dir.path().join("code.txt");
    let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    assert!(git(dir.path(), &["add", "."]));
    assert!(git(
        dir.path(),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "twenty lines"
        ]
    ));
    let mut changed = lines.clone();
    changed[1] = "LINE 2".into();
    changed[17] = "LINE 18".into();
    std::fs::write(&path, changed.join("\n") + "\n").unwrap();

    let mut app = app();
    keys(&mut app, &format!(":e {}<CR><Space>g", path.display()));
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Git);
    let shown = render(&mut app, 110, 30).rows.join("\n");
    for part in [
        "[1] Files",
        "[2] Branches (1)",
        "[3] Commits",
        "[4] code.txt (unstaged)",
        "* main",
        "+LINE 2",
        "+LINE 18",
        "twenty lines",
    ] {
        assert!(shown.contains(part), "{part}: {shown}");
    }

    // Into the diff, the second hunk, staged on its own.
    keys(&mut app, "<CR>js");
    let out = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "MM code.txt\n");
    keys(&mut app, "<Esc>");
    let shown = render(&mut app, 110, 30).rows.join("\n");
    assert!(
        shown.contains("STAGED (1)") && shown.contains("CHANGES (1)"),
        "{shown}"
    );

    // Commit it from here, and stay here.
    keys(&mut app, "cjust line 18<CR>");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Git);
    let shown = render(&mut app, 110, 30).rows.join("\n");
    assert!(shown.contains("just line 18"), "in the commits: {shown}");

    // A click on the older commit shows it.
    let screen = render(&mut app, 110, 30);
    let (y, row) = screen
        .rows
        .iter()
        .enumerate()
        .find(|(_, r)| r.contains("twenty lines"))
        .unwrap();
    let x = row[..row.find("twenty").unwrap()].chars().count() as u16;
    app.handle_mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: x,
        row: y as u16,
        modifiers: crossterm::event::KeyModifiers::NONE,
    });
    let gui = app.editor().git_gui().unwrap();
    assert!(gui.diff_title.starts_with("Commit "), "{}", gui.diff_title);
    assert!(
        gui.diff.lines.iter().any(|(_, l)| l == "+line 20"),
        "its diff"
    );

    keys(&mut app, "q");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Normal);
    assert!(app.editor().git_gui().is_none());
}

/// Waits for a background fetch, pull or push to finish.
fn wait_for_job(app: &mut ferra::App) {
    let start = std::time::Instant::now();
    while app.editor().git_job_running() && start.elapsed() < std::time::Duration::from_secs(20) {
        app.editor_mut().pump_git_job();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!app.editor().git_job_running(), "the job finished");
}

fn message(app: &ferra::App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

#[test]
fn branches_and_pushing_from_the_git_view() {
    let Some(dir) = repo() else {
        return;
    };
    let remote = tempfile::tempdir().unwrap();
    let bare = remote.path().join("origin.git");
    assert!(git(
        remote.path(),
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]
    ));
    assert!(git(
        dir.path(),
        &["remote", "add", "origin", bare.to_str().unwrap()]
    ));

    let path = dir.path().join("code.txt");
    let mut app = app();
    keys(&mut app, &format!(":e {}<CR><Space>g", path.display()));

    // The first push sets the upstream.
    keys(&mut app, "P");
    wait_for_job(&mut app);
    assert_eq!(message(&app), "Pushed");
    let local = |args: &[&str]| git(dir.path(), args);
    assert!(local(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "local"
    ]));
    keys(&mut app, "R");
    keys(&mut app, "q");
    let shown = render(&mut app, 80, 8).rows.join("\n");
    assert!(shown.contains("⎇ main ↑1"), "{shown}");
    keys(&mut app, "<Space>gP");
    wait_for_job(&mut app);
    keys(&mut app, "q");
    assert!(!render(&mut app, 80, 8).rows.join("\n").contains('↑'));

    // A new branch, back to main, and deleting it.
    keys(&mut app, "<Space>g2nfeature<CR>");
    assert_eq!(app.editor().git_branch(), Some("feature"));
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Git);
    keys(&mut app, "2");
    let gui = app.editor().git_gui().unwrap();
    assert_eq!(gui.branches[0].name, "feature", "the current one first");
    keys(&mut app, "j<CR>");
    assert_eq!(app.editor().git_branch(), Some("main"));
    keys(&mut app, "2");
    let index = app
        .editor()
        .git_gui()
        .unwrap()
        .branches
        .iter()
        .position(|b| b.name == "feature")
        .unwrap();
    for _ in 0..index {
        keys(&mut app, "j");
    }
    keys(&mut app, "dy");
    assert_eq!(message(&app), "Deleted feature");
    assert_eq!(app.editor().git_gui().unwrap().branches.len(), 1);
    keys(&mut app, "q");

    // Not with unsaved changes.
    assert!(local(&["branch", "other"]));
    keys(&mut app, "ccchanged<Esc>:git_switch other<CR>");
    assert!(
        message(&app).starts_with("Save or undo the unsaved changes first"),
        "{}",
        message(&app)
    );
    keys(&mut app, "u:git_branch<CR>");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Picker);
    keys(&mut app, "other<CR>");
    assert_eq!(app.editor().git_branch(), Some("other"));
}

/// Runs the blame's ticks (as the event loop does) until `done`, or 5s.
fn blame_until(app: &mut ferra::App, done: impl Fn(&ferra::App) -> bool) {
    let start = std::time::Instant::now();
    while !done(app) && start.elapsed() < std::time::Duration::from_secs(5) {
        app.editor_mut().git_blame_tick();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn blame_shown(app: &ferra::App) -> Option<(usize, String)> {
    app.editor()
        .document
        .virtual_text()
        .iter()
        .find(|v| v.source == "blame")
        .map(|v| (v.at.row, v.text.clone()))
}

#[test]
fn the_cursors_line_is_blamed_after_it() {
    let Some(dir) = repo() else {
        return;
    };
    let mut app = app();
    keys(
        &mut app,
        &format!(":e {}<CR>j", dir.path().join("code.txt").display()),
    );
    blame_until(&mut app, |a| blame_shown(a).is_some());
    let (row, text) = blame_shown(&app).expect("blamed");
    assert_eq!(row, 1);
    assert!(text.starts_with("t, ") && text.ends_with(" • x"), "{text}");
    let screen = render(&mut app, 80, 6);
    assert!(screen.rows[1].contains("two  t, "), "{:?}", screen.rows);

    // A line added and not saved isn't committed.
    keys(&mut app, "onew line<Esc>");
    blame_until(&mut app, |a| blame_shown(a).is_some_and(|(r, _)| r == 2));
    assert_eq!(blame_shown(&app).unwrap().1, "You • not committed yet");

    // Insert mode hides it; so does turning it off.
    keys(&mut app, "A");
    app.editor_mut().git_blame_tick();
    assert_eq!(blame_shown(&app), None);
    keys(&mut app, "<Esc>:git_blame<CR>");
    // Past the time it'd have been asked for.
    for _ in 0..30 {
        app.editor_mut().git_blame_tick();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(blame_shown(&app), None);
}

#[test]
fn amending_and_stashing_from_the_git_view() {
    let Some(dir) = repo() else {
        return;
    };
    let path = dir.path().join("code.txt");
    let mut app = app();
    keys(&mut app, &format!(":e {}<CR>", path.display()));

    // Amend: the last message to edit, and the staged change goes in.
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();
    assert!(git(dir.path(), &["add", "code.txt"]));
    keys(&mut app, ":e!<CR><Space>gC");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Input);
    let input = app.editor().input_box().unwrap();
    assert_eq!(input.field.text(), "x", "the last message");
    assert!(!input.field.is_selected(), "to edit, not type over");
    keys(&mut app, ", now with six<CR>");
    assert!(
        message(&app).ends_with(": x, now with six"),
        "{}",
        message(&app)
    );
    assert_eq!(
        app.editor().mode(),
        ferra::editor::Mode::Git,
        "back in the view"
    );
    let gui = app.editor().git_gui().unwrap();
    assert_eq!(gui.commits.len(), 1, "still one commit");
    assert_eq!(gui.commits[0].subject, "x, now with six");

    // `:` in the view comes back to it too.
    keys(&mut app, ":git_refresh<CR>");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Git);

    // Stash: changes (an untracked file too) put away, and back.
    std::fs::write(&path, "changed\n").unwrap();
    std::fs::write(dir.path().join("notes.md"), "notes\n").unwrap();
    keys(&mut app, "R");
    keys(&mut app, "zwip<CR>");
    assert_eq!(message(&app), "Stashed your changes (Z puts them back)");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "one\ntwo\nthree\nfour\nfive\nsix\n"
    );
    assert!(!dir.path().join("notes.md").exists());
    assert_eq!(
        common::text(&app),
        "one\ntwo\nthree\nfour\nfive\nsix",
        "the open file follows"
    );
    keys(&mut app, "Z");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Picker);
    keys(&mut app, "<CR>");
    assert_eq!(message(&app), "Put back stash@{0}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed\n");
    assert!(dir.path().join("notes.md").exists());
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Git);
    keys(&mut app, "Z");
    assert_eq!(message(&app), "No stashes");

    // Not with unsaved changes.
    keys(&mut app, "q:e!<CR>ccedited<Esc><Space>gz");
    assert!(
        message(&app).starts_with("Save or undo the unsaved changes first"),
        "{}",
        message(&app)
    );
}
