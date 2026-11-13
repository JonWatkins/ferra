//! Plugin packages end to end: `:plugin_install` from a git repository
//! (a local one here), asking first, loading with `require`, recording in
//! `plugins.toml` / `plugins.lock`, loading again at startup,
//! `:plugin_list` and `:plugin_remove`.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{app, keys, text};
use ferra::App;
use ferra::plugins::packages::PluginPaths;
use tempfile::TempDir;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A git repository holding `files`, committed.
fn repo(root: &Path, name: &str, files: &[(&str, &str)]) -> String {
    let dir = root.join(name);
    for (path, contents) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    git(&dir, &["init", "--quiet"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "--quiet", "-m", "first"]);
    dir.display().to_string()
}

const MANIFEST: &str = r#"
name = "hello"
version = "0.1.0"
description = "Says hello"
ferra = ">=0.1"
"#;

/// A plugin whose main file `require`s a module from its `lua/` folder.
fn hello_repo(root: &Path) -> String {
    repo(
        root,
        "ferra-hello",
        &[
            ("ferra-plugin.toml", MANIFEST),
            (
                "init.lua",
                r#"local util = require("hello.util")
                   editor.command("hello", "Say hello", function(ctx) ctx:insert(util.greeting()) end)"#,
            ),
            (
                "lua/hello/util.lua",
                r#"return { greeting = function() return "hello from a package" end }"#,
            ),
        ],
    )
}

struct Setup {
    temp: TempDir,
    paths: PluginPaths,
}

fn setup() -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let paths = PluginPaths {
        config: temp.path().join("config"),
        data: temp.path().join("data"),
    };
    Setup { temp, paths }
}

fn app_with(paths: &PluginPaths) -> App {
    let mut app = app();
    app.set_plugin_paths(paths.clone());
    app
}

fn install(app: &mut App, source: &str) {
    keys(app, &format!(":plugin_install {source}<CR>"));
    assert_eq!(
        app.editor().popup().map(|p| p.name.as_str()),
        Some("plugin-install"),
        "asks first: {}",
        message(app)
    );
    keys(app, "y");
    assert!(app.editor().popup().is_none());
    assert!(message(app).starts_with("Installing"), "{}", message(app));
    app.wait_for_background();
}

#[test]
fn install_asks_then_clones_records_and_loads() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);

    keys(&mut app, &format!(":plugin_install {source}<CR>"));
    let popup = app.editor().popup().expect("asks first");
    let shown: String = popup
        .lines
        .iter()
        .flatten()
        .map(|s| s.text.as_str())
        .collect();
    assert!(shown.contains(&source), "{shown}");
    assert!(shown.contains("permissions"), "{shown}");
    keys(&mut app, "y");
    app.wait_for_background();

    let commit = git(Path::new(&source), &["rev-parse", "HEAD"]);
    assert_eq!(
        message(&app),
        format!("Installed hello 0.1.0 ({})", &commit[..7])
    );
    keys(&mut app, ":hello<CR>");
    assert_eq!(text(&app), "hello from a package");

    assert!(s.paths.data.join("hello/ferra-plugin.toml").is_file());
    let list = fs::read_to_string(s.paths.config.join("plugins.toml")).unwrap();
    assert!(
        list.contains(&format!("[plugins.hello]\nsource = \"{source}\"")),
        "{list}"
    );
    let lock = fs::read_to_string(s.paths.config.join("plugins.lock")).unwrap();
    assert!(lock.contains(&commit), "{lock}");

    // Next time ferra starts, it's loaded from plugins.toml.
    let mut again = app_with(&s.paths);
    assert_eq!(again.load_plugins(&s.paths.config), None);
    keys(&mut again, ":hello<CR>");
    assert_eq!(text(&again), "hello from a package");
}

#[test]
fn n_cancels() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    keys(&mut app, &format!(":plugin_install {source}<CR>n"));
    assert!(app.editor().popup().is_none());
    app.wait_for_background();
    assert!(!s.paths.data.join("hello").exists());
    keys(&mut app, ":plugin_confirmed<CR>");
    assert_eq!(message(&app), "Nothing to confirm");
}

#[test]
fn tags_are_installed_and_recorded() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let first = git(Path::new(&source), &["rev-parse", "HEAD"]);
    git(Path::new(&source), &["tag", "v0.1.0"]);
    fs::write(Path::new(&source).join("later.txt"), "x").unwrap();
    git(Path::new(&source), &["add", "."]);
    git(Path::new(&source), &["commit", "--quiet", "-m", "later"]);

    let mut app = app_with(&s.paths);
    install(&mut app, &format!("{source}@v0.1.0"));
    assert!(
        message(&app).starts_with("Installed hello"),
        "{}",
        message(&app)
    );
    assert!(!s.paths.data.join("hello/later.txt").exists());
    let list = fs::read_to_string(s.paths.config.join("plugins.toml")).unwrap();
    assert!(list.contains("tag = \"v0.1.0\""), "{list}");
    let lock = fs::read_to_string(s.paths.config.join("plugins.lock")).unwrap();
    assert!(lock.contains(&first), "{lock}");
}

#[test]
fn repositories_that_are_not_plugins_are_refused_and_cleaned_up() {
    let s = setup();
    let mut app = app_with(&s.paths);

    let plain = repo(s.temp.path(), "plain", &[("README.md", "hi")]);
    install(&mut app, &plain);
    assert_eq!(
        message(&app),
        format!("Plugin not installed: {plain}: no ferra-plugin.toml: not a ferra plugin")
    );

    let future = repo(
        s.temp.path(),
        "future",
        &[
            (
                "ferra-plugin.toml",
                "name = \"future\"\nversion = \"1.0.0\"\nferra = \">=99\"",
            ),
            ("init.lua", ""),
        ],
    );
    install(&mut app, &future);
    assert!(
        message(&app).contains("future needs ferra >=99"),
        "{}",
        message(&app)
    );

    let missing = s.temp.path().join("nothing-here").display().to_string();
    install(&mut app, &missing);
    assert!(
        message(&app).starts_with("Plugin not installed: Couldn't clone"),
        "{}",
        message(&app)
    );

    // Nothing left behind, nothing recorded.
    let left: Vec<_> = fs::read_dir(&s.paths.data).unwrap().flatten().collect();
    assert!(left.is_empty(), "{left:?}");
    assert!(!s.paths.config.join("plugins.toml").exists());
}

#[test]
fn installing_twice_is_refused() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    install(&mut app, &source);

    keys(&mut app, &format!(":plugin_install {source}<CR>"));
    assert_eq!(
        message(&app),
        format!("{source} is already installed as hello")
    );
    assert!(app.editor().popup().is_none());

    // Another repository with the same name.
    let copy = repo(
        s.temp.path(),
        "copy",
        &[("ferra-plugin.toml", MANIFEST), ("init.lua", "")],
    );
    install(&mut app, &copy);
    assert_eq!(
        message(&app),
        "Plugin not installed: A plugin named hello is already installed (:plugin_remove hello first)"
    );
}

#[test]
fn list_and_remove() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);

    keys(&mut app, ":plugin_list<CR>");
    assert_eq!(
        message(&app),
        "No plugins installed (:plugin_install owner/repo)"
    );

    install(&mut app, &source);
    keys(&mut app, ":plugin_list<CR>");
    let popup = app.editor().popup().expect("a list");
    assert_eq!(popup.title, "Plugins (1)");
    let line: String = popup.lines[0].iter().map(|s| s.text.as_str()).collect();
    assert!(line.starts_with("hello  0.1.0"), "{line}");
    assert!(line.contains("Says hello"), "{line}");
    keys(&mut app, "q");

    keys(&mut app, ":plugin_remove hello<CR>");
    assert_eq!(message(&app), "Removed hello");
    keys(&mut app, ":hello<CR>");
    assert!(
        message(&app).contains("hello"),
        "its command is gone: {}",
        message(&app)
    );
    assert_eq!(text(&app), "");
    assert!(!s.paths.data.join("hello").exists());
    let list = fs::read_to_string(s.paths.config.join("plugins.toml")).unwrap();
    assert!(!list.contains("hello"), "{list}");

    keys(&mut app, ":plugin_remove hello<CR>");
    assert_eq!(message(&app), "No plugin named hello is installed");
}

#[test]
fn startup_reports_broken_packages_and_still_loads_the_rest() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    install(&mut app, &source);
    fs::write(
        s.paths.config.join("plugins.toml"),
        format!("[plugins.gone]\nsource = \"o/gone\"\n\n[plugins.hello]\nsource = \"{source}\"\n"),
    )
    .unwrap();

    let mut again = app_with(&s.paths);
    let error = again
        .load_plugins(&s.paths.config)
        .expect("gone is missing");
    assert!(
        error.contains("gone is in plugins.toml but not installed"),
        "{error}"
    );
    keys(&mut again, ":hello<CR>");
    assert_eq!(text(&again), "hello from a package");
}

#[test]
fn bad_sources_are_refused_before_asking() {
    let s = setup();
    let mut app = app_with(&s.paths);
    keys(&mut app, ":plugin_install<CR>");
    assert!(message(&app).starts_with("Usage: :plugin_install"));
    keys(&mut app, ":plugin_install justaname<CR>");
    assert!(message(&app).starts_with("Not a plugin to install"));
    assert!(app.editor().popup().is_none());
}

/// Commits `files` to the repository at `source`.
fn commit(source: &str, files: &[(&str, &str)], subject: &str) -> String {
    let dir = Path::new(source);
    for (path, contents) in files {
        fs::write(dir.join(path), contents).unwrap();
    }
    git(dir, &["add", "."]);
    git(dir, &["commit", "--quiet", "-m", subject]);
    git(dir, &["rev-parse", "HEAD"])
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

#[test]
fn reload_starts_afresh() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    install(&mut app, &source);

    // Edit the installed copy, as when working on a plugin.
    let dir = s.paths.data.join("hello");
    fs::write(
        dir.join("init.lua"),
        r#"local util = require("hello.util")
           editor.command("hi", function(ctx) ctx:insert(util.greeting()) end)
           editor.map("normal", "<Space>H", "hi")"#,
    )
    .unwrap();
    fs::write(
        dir.join("lua/hello/util.lua"),
        r#"return { greeting = function() return "changed" end }"#,
    )
    .unwrap();
    keys(&mut app, ":plugin_reload<CR>");
    assert_eq!(message(&app), "Plugins reloaded");

    keys(&mut app, ":hello<CR>");
    assert!(message(&app).contains("hello"), "gone: {}", message(&app));
    keys(&mut app, "<Space>H");
    assert_eq!(text(&app), "changed", "require isn't cached across reloads");
    keys(&mut app, "dd");
    assert_eq!(text(&app), "", "built-in keys are still there");
}

#[test]
fn update_shows_the_new_commits_then_updates() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    install(&mut app, &source);

    keys(&mut app, ":plugin_update<CR>");
    assert_eq!(message(&app), "Checking plugins for updates…");
    app.wait_for_background();
    assert_eq!(message(&app), "hello is up to date");

    let new = commit(
        &source,
        &[(
            "lua/hello/util.lua",
            r#"return { greeting = function() return "updated" end }"#,
        )],
        "Greet differently",
    );
    keys(&mut app, ":plugin_update hello<CR>");
    app.wait_for_background();
    let shown = popup_text(&app);
    assert!(shown.contains("1 new commit"), "{shown}");
    assert!(
        shown.contains(&format!("{} Greet differently", &new[..7])),
        "{shown}"
    );
    keys(&mut app, "y");
    app.wait_for_background();
    assert_eq!(
        message(&app),
        format!("Updated hello 0.1.0 ({})", &new[..7])
    );
    let lock = fs::read_to_string(s.paths.config.join("plugins.lock")).unwrap();
    assert!(lock.contains(&new), "{lock}");
    keys(&mut app, ":hello<CR>");
    assert_eq!(text(&app), "updated", "reloaded");

    keys(&mut app, ":plugin_update nope<CR>");
    assert_eq!(message(&app), "No plugin named nope is installed");
}

#[test]
fn an_update_that_breaks_the_plugin_keeps_the_old_version() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let first = git(Path::new(&source), &["rev-parse", "HEAD"]);
    let mut app = app_with(&s.paths);
    install(&mut app, &source);
    commit(
        &source,
        &[(
            "ferra-plugin.toml",
            "name = \"hello\"\nversion = \"2.0.0\"\nferra = \">=99\"",
        )],
        "Need a newer ferra",
    );

    keys(&mut app, ":plugin_update<CR>");
    app.wait_for_background();
    keys(&mut app, "y");
    app.wait_for_background();
    assert_eq!(
        message(&app),
        "hello: kept the old version: hello needs ferra >=99, this is 0.1.0"
    );
    let dir = s.paths.data.join("hello");
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), first);
    let lock = fs::read_to_string(s.paths.config.join("plugins.lock")).unwrap();
    assert!(lock.contains(&first), "{lock}");
    keys(&mut app, ":hello<CR>");
    assert_eq!(text(&app), "hello from a package");
}

#[test]
fn tagged_plugins_stay_pinned() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    git(Path::new(&source), &["tag", "v0.1.0"]);
    commit(&source, &[("later.txt", "x")], "Later");
    let mut app = app_with(&s.paths);
    install(&mut app, &format!("{source}@v0.1.0"));

    keys(&mut app, ":plugin_update<CR>");
    app.wait_for_background();
    assert_eq!(message(&app), "Nothing to update: hello: pinned to v0.1.0");
    assert!(app.editor().popup().is_none());
}

#[test]
fn install_with_nothing_restores_what_plugins_toml_lists() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    let locked = git(Path::new(&source), &["rev-parse", "HEAD"]);
    let mut app = app_with(&s.paths);
    install(&mut app, &source);

    keys(&mut app, ":plugin_install<CR>");
    assert!(
        message(&app).starts_with("Usage: :plugin_install"),
        "nothing missing: {}",
        message(&app)
    );

    // A new machine: the config came along, the clones didn't. The
    // source has moved on since, but the lock says where to be.
    fs::remove_dir_all(&s.paths.data).unwrap();
    commit(&source, &[("later.txt", "x")], "Later");
    let mut again = app_with(&s.paths);
    let error = again.load_plugins(&s.paths.config).unwrap();
    assert!(error.contains(":plugin_install installs it"), "{error}");

    keys(&mut again, ":plugin_install<CR>");
    let shown = popup_text(&again);
    assert!(shown.contains("hello"), "{shown}");
    assert!(shown.contains(&source), "{shown}");
    keys(&mut again, "y");
    again.wait_for_background();
    assert_eq!(
        message(&again),
        format!("Installed hello 0.1.0 ({})", &locked[..7])
    );
    let dir = s.paths.data.join("hello");
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), locked);
    keys(&mut again, ":hello<CR>");
    assert_eq!(text(&again), "hello from a package");

    // It's on its branch, so it can update later.
    keys(&mut again, ":plugin_update<CR>");
    again.wait_for_background();
    assert!(
        popup_text(&again).contains("Later"),
        "{}",
        popup_text(&again)
    );
}

#[test]
fn reload_reads_init_lua_again_and_keeps_the_shown_view() {
    let s = setup();
    fs::create_dir_all(&s.paths.config).unwrap();
    let init = s.paths.config.join("init.lua");
    let view = |word: &str| {
        format!(
            r#"editor.command("say", function(ctx) ctx:insert("{word}") end)
               editor.view("notes", {{ title = "Notes", lines = function() return {{ "{word}" }} end }})"#
        )
    };
    fs::write(&init, view("one")).unwrap();
    let mut app = app_with(&s.paths);
    assert_eq!(app.load_plugins(&s.paths.config), None);
    keys(&mut app, ":sidebar_view notes<CR>");
    assert_eq!(
        app.editor().plugin_view().map(|v| v.spec.name.as_str()),
        Some("notes")
    );

    fs::write(&init, view("two")).unwrap();
    keys(&mut app, "<Esc>:plugin_reload<CR>");
    assert_eq!(message(&app), "Plugins reloaded");
    assert_eq!(
        app.editor().plugin_view().map(|v| v.spec.name.as_str()),
        Some("notes"),
        "still shown"
    );
    keys(&mut app, ":say<CR>");
    assert_eq!(text(&app), "two");

    fs::write(&init, "this does not parse").unwrap();
    keys(&mut app, ":plugin_reload<CR>");
    assert!(
        message(&app).starts_with("Plugin errors: init.lua"),
        "{}",
        message(&app)
    );
    assert!(
        app.editor().plugin_view().is_none(),
        "its view went with it"
    );
}

#[test]
fn a_popup_opened_by_background_results_takes_its_keys_at_once() {
    let s = setup();
    let source = hello_repo(s.temp.path());
    install(&mut app_with(&s.paths), &source);
    let new = commit(&source, &[("later.txt", "x")], "Later");

    // A fresh app: no popup has been open before the update's.
    let mut app = app_with(&s.paths);
    assert_eq!(app.load_plugins(&s.paths.config), None);
    keys(&mut app, ":plugin_update<CR>");
    app.wait_for_background();
    assert!(popup_text(&app).contains("Later"));
    keys(&mut app, "y");
    app.wait_for_background();
    assert_eq!(
        message(&app),
        format!("Updated hello 0.1.0 ({})", &new[..7])
    );
}
