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
    keys(&mut app, ":plugin_install_confirmed<CR>");
    assert_eq!(message(&app), "Nothing to install");
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
    assert_eq!(
        message(&app),
        "Removed hello; its commands stay until ferra restarts"
    );
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
