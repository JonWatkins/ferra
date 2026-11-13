//! Plugin packages end to end: `:plugin_install` from a git repository
//! (a local one here), asking first, loading with `require`, recording in
//! `plugins.toml` / `plugins.lock`, loading again at startup,
//! `:plugin_list` and `:plugin_remove`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{keys, text};
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
    // No offers of language packs (from GitHub) unless a test asks.
    let mut app = common::app_with_config("[editor]\nplugin-suggestions = false\n");
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
    // A bare name is looked up in the plugin directory.
    keys(&mut app, ":plugin_install justaname<CR>");
    assert!(
        message(&app).starts_with("No plugin named justaname in the plugin directory"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":plugin_install has space/x<CR>");
    assert!(
        message(&app).starts_with("Not a plugin to install"),
        "{}",
        message(&app)
    );
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

/// An app whose plugin directory is `index` (a local repository).
fn app_browsing(paths: &PluginPaths, index: &str) -> App {
    let mut app = common::app_with_config(&format!(
        "[editor]\nplugin-index = \"{index}\"\nplugin-suggestions = false\n"
    ));
    app.set_plugin_paths(paths.clone());
    app
}

fn index_toml(entries: &[(&str, &str, &str)]) -> String {
    let mut text = "version = 1\n".to_string();
    for (name, repo, description) in entries {
        text.push_str(&format!(
            "\n[[plugin]]\nname = \"{name}\"\nrepo = \"{repo}\"\ndescription = \"{description}\"\n"
        ));
    }
    text
}

fn picker_items(app: &App) -> Vec<(String, String)> {
    let picker = app.editor().picker().expect("the picker is open");
    picker
        .matches()
        .iter()
        .map(|m| picker.item(m))
        .map(|i| (i.label.clone(), i.detail.clone()))
        .collect()
}

#[test]
fn browse_lists_the_directory_and_installs_the_choice() {
    let s = setup();
    let hello = hello_repo(s.temp.path());
    let other = repo(
        s.temp.path(),
        "other",
        &[
            ("ferra-plugin.toml", "name = \"other\"\nversion = \"0.2.0\""),
            ("init.lua", ""),
        ],
    );
    let index = repo(
        s.temp.path(),
        "index",
        &[(
            "index.toml",
            &index_toml(&[
                ("hello", &hello, "Says hello"),
                ("other", &other, "Another"),
            ]),
        )],
    );
    let mut app = app_browsing(&s.paths, &index);
    install(&mut app, &other);

    keys(&mut app, ":plugin_browse<CR>");
    assert_eq!(message(&app), "Getting the plugin directory…");
    app.wait_for_background();
    assert_eq!(
        picker_items(&app),
        [
            ("hello".to_string(), "Says hello".to_string()),
            ("other".to_string(), "(installed) Another".to_string()),
        ]
    );
    assert_eq!(message(&app), "");

    keys(&mut app, "hello<CR>");
    let shown = popup_text(&app);
    assert!(shown.contains(&hello), "asks first: {shown}");
    keys(&mut app, "y");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Installed hello 0.1.0"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":hello<CR>");
    assert_eq!(text(&app), "hello from a package");
}

#[test]
fn browse_follows_the_directory_and_falls_back_to_the_last_copy() {
    let s = setup();
    let index = repo(s.temp.path(), "index", &[("index.toml", "version = 1\n")]);
    let mut app = app_browsing(&s.paths, &index);
    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert_eq!(message(&app), "The plugin directory has no plugins yet");

    commit(
        &index,
        &[("index.toml", &index_toml(&[("new", "o/new", "Just added")]))],
        "Add new",
    );
    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert_eq!(picker_items(&app)[0].0, "new");
    keys(&mut app, "<Esc>");

    // The directory can't be reached: the last copy is shown.
    fs::rename(&index, s.temp.path().join("moved")).unwrap();
    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert_eq!(picker_items(&app)[0].0, "new");
    assert!(
        message(&app).starts_with("Couldn't update the plugin directory"),
        "{}",
        message(&app)
    );
}

#[test]
fn browse_reports_what_is_not_a_directory() {
    let s = setup();
    let plain = repo(s.temp.path(), "plain", &[("README.md", "hi")]);
    let mut app = app_browsing(&s.paths, &plain);
    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert_eq!(
        message(&app),
        format!("{plain} has no index.toml: not a plugin directory")
    );

    let missing = s.temp.path().join("nowhere").display().to_string();
    let mut app = app_browsing(&s.paths, &missing);
    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert!(
        message(&app).starts_with(&format!("Couldn't get the plugin directory {missing}")),
        "{}",
        message(&app)
    );
    assert!(app.editor().picker().is_none());
}

const ZIG_PACK: &str = r#"
name = "zig"
version = "0.1.0"
description = "Zig files: comments, tabs and zls"

[languages.zig]
extensions = ["zig", "zon"]
comment = "//"
options = { tab-width = 8 }
language-server = { command = "zls" }
"#;

#[test]
fn language_packs_add_languages() {
    let s = setup();
    let pack = repo(
        s.temp.path(),
        "ferra-zig",
        &[("ferra-plugin.toml", ZIG_PACK)],
    );
    let file = s.temp.path().join("main.zig");
    fs::write(&file, "const x = 1;\n").unwrap();

    let mut app = app_with(&s.paths);
    app.open(&file);
    assert_eq!(app.editor().language_of(&app.editor().document), None);

    // No Lua at all: a pure language pack.
    install(&mut app, &pack);
    assert!(
        message(&app).starts_with("Installed zig 0.1.0"),
        "{}",
        message(&app)
    );
    assert_eq!(
        app.editor().language_of(&app.editor().document),
        Some("zig"),
        "the open file is Zig now"
    );
    assert_eq!(app.editor().options.tab_width, 8);
    assert_eq!(
        app.editor()
            .languages
            .server("zig")
            .map(|s| s.command.as_str()),
        Some("zls")
    );
    keys(&mut app, "gcc");
    assert_eq!(text(&app), "// const x = 1;");

    // At startup too.
    let mut again = app_with(&s.paths);
    assert_eq!(again.load_plugins(&s.paths.config), None);
    again.open(&file);
    assert_eq!(
        again.editor().language_of(&again.editor().document),
        Some("zig")
    );

    // Removing it takes the language away.
    keys(&mut again, ":plugin_remove zig<CR>");
    assert_eq!(again.editor().language_of(&again.editor().document), None);
    assert_ne!(again.editor().options.tab_width, 8);
}

#[test]
fn language_packs_cannot_replace_known_languages() {
    let s = setup();
    let pack = repo(
        s.temp.path(),
        "my-rust",
        &[(
            "ferra-plugin.toml",
            "name = \"my-rust\"\nversion = \"0.1.0\"\n[languages.rust]\nextensions = [\"rs\"]\n",
        )],
    );
    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    assert!(
        message(&app)
            .contains("rust is already a language (change it in config.toml's [languages.rust])"),
        "{}",
        message(&app)
    );

    let bad = repo(
        s.temp.path(),
        "bad-options",
        &[(
            "ferra-plugin.toml",
            "name = \"bad\"\nversion = \"0.1.0\"\n[languages.badlang]\nextensions = [\"bad\"]\noptions = { mouse = false }\n",
        )],
    );
    install(&mut app, &bad);
    assert!(
        message(&app).contains("can't be set per language"),
        "{}",
        message(&app)
    );

    let empty = repo(
        s.temp.path(),
        "empty",
        &[(
            "ferra-plugin.toml",
            "name = \"empty\"\nversion = \"0.1.0\"\n",
        )],
    );
    install(&mut app, &empty);
    assert!(
        message(&app).contains("no init.lua (or main file) and adds no languages"),
        "{}",
        message(&app)
    );
}

/// Whether there's a C compiler to build grammars with.
fn have_cc() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A language pack carrying the test grammar (tree-sitter-json's, as a
/// language called "data") with its highlight query.
fn grammar_pack(root: &Path) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grammar-json");
    let read = |p: &str| fs::read_to_string(fixture.join(p)).unwrap();
    let parser = read("src/parser.c");
    let header = read("src/tree_sitter/parser.h");
    let alloc = read("src/tree_sitter/alloc.h");
    let array = read("src/tree_sitter/array.h");
    let highlights = read("queries/highlights.scm");
    repo(
        root,
        "ferra-data",
        &[
            (
                "ferra-plugin.toml",
                r#"
name = "data"
version = "0.1.0"
description = "A test language with a compiled grammar"

[languages.data]
extensions = ["data"]

[grammars.data]
path = "grammar"
symbol = "tree_sitter_json"
"#,
            ),
            ("grammar/src/parser.c", &parser),
            ("grammar/src/tree_sitter/parser.h", &header),
            ("grammar/src/tree_sitter/alloc.h", &alloc),
            ("grammar/src/tree_sitter/array.h", &array),
            ("queries/data/highlights.scm", &highlights),
            (
                "queries/data/indents.scm",
                "[(object) (array)] @indent\n[\"}\" \"]\"] @outdent\n",
            ),
        ],
    )
}

/// Screen columns on row `y` drawn in the default theme's string colour
/// (the gutter is 5 wide, so text column 0 is screen column 5).
fn string_cells(app: &mut App, y: u16) -> Vec<u16> {
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    let string = Color::Rgb(0x9e, 0xce, 0x6a);
    let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..40).filter(|&x| buffer[(x, y)].fg == string).collect()
}

#[test]
fn language_packs_compile_and_load_grammars() {
    if !have_cc() {
        eprintln!("no C compiler: skipped");
        return;
    }
    let s = setup();
    let pack = grammar_pack(s.temp.path());
    let file = s.temp.path().join("x.data");
    fs::write(&file, "{\"key\": 12}\n").unwrap();

    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    assert!(
        message(&app).starts_with("Installed data 0.1.0"),
        "{}",
        message(&app)
    );
    let builds: Vec<_> = fs::read_dir(s.paths.data.join("data/.ferra-build"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(builds.len(), 1, "{builds:?}");
    assert!(builds[0].starts_with("data-"), "{builds:?}");
    let dir = s.paths.data.join("data");
    assert_eq!(
        git(&dir, &["status", "--porcelain"]),
        "",
        "the build isn't in git's way"
    );

    app.open(&file);
    // `{"key": 12}`: the key is a string; the number isn't.
    let cells = string_cells(&mut app, 0);
    assert_eq!(cells, vec![6, 7, 8, 9, 10], "{cells:?}");

    // At startup the build is reused, not made again.
    let mut again = app_with(&s.paths);
    assert_eq!(again.load_plugins(&s.paths.config), None);
    again.open(&file);
    assert_eq!(string_cells(&mut again, 0), vec![6, 7, 8, 9, 10]);
    keys(&mut again, ":plugin_reload<CR>");
    assert_eq!(message(&again), "Plugins reloaded");
    assert_eq!(string_cells(&mut again, 0), vec![6, 7, 8, 9, 10]);
}

#[test]
fn grammars_that_do_not_build_still_install_the_language() {
    let s = setup();
    let pack = repo(
        s.temp.path(),
        "broken",
        &[
            (
                "ferra-plugin.toml",
                "name = \"broken\"\nversion = \"0.1.0\"\n[languages.broken]\nextensions = [\"brk\"]\ncomment = \"#\"\n[grammars.broken]\n",
            ),
            ("src/parser.c", "this is not C"),
        ],
    );
    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    let shown = message(&app);
    assert!(shown.starts_with("Installed broken 0.1.0"), "{shown}");
    let why = if have_cc() {
        "didn't compile"
    } else {
        "no C compiler"
    };
    assert!(shown.contains(why), "{shown}");
    let file = s.temp.path().join("a.brk");
    fs::write(&file, "x").unwrap();
    app.open(&file);
    assert_eq!(
        app.editor().language_of(&app.editor().document),
        Some("broken")
    );
}

#[test]
fn grammars_need_their_language_and_cannot_replace_built_ins() {
    let s = setup();
    let pack = repo(
        s.temp.path(),
        "rusty",
        &[(
            "ferra-plugin.toml",
            "name = \"rusty\"\nversion = \"0.1.0\"\n[grammars.rust]\n[grammars.nolang]\n",
        )],
    );
    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    let shown = message(&app);
    assert!(
        shown.contains("ferra has a rust grammar built in"),
        "{shown}"
    );
    assert!(shown.contains("there's no language nolang"), "{shown}");
}

#[test]
fn language_packs_bring_indentation_rules() {
    if !have_cc() {
        eprintln!("no C compiler: skipped");
        return;
    }
    let s = setup();
    let pack = grammar_pack(s.temp.path());
    let file = s.temp.path().join("y.data");
    // A list opened mid-line: the line after it is one level in from
    // where the list starts, which plain indentation (copying the line
    // above) gets wrong.
    fs::write(&file, "{\n    \"a\": [1,\n}\n").unwrap();
    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    app.open(&file);
    keys(&mut app, "jo2]<Esc>");
    assert_eq!(text(&app), "{\n    \"a\": [1,\n        2]\n}");
}

/// A directory like `ferra-plugins`: an index, and plugins in folders
/// (`plugins/hello`, `plugins/other`), listed without a `repo`.
fn directory_repo(root: &Path) -> String {
    repo(
        root,
        "directory",
        &[
            (
                "index.toml",
                "version = 1\n\n[[plugin]]\nname = \"hello\"\npath = \"plugins/hello\"\ndescription = \"Says hello\"\n\n[[plugin]]\nname = \"other\"\npath = \"plugins/other\"\n",
            ),
            ("plugins/hello/ferra-plugin.toml", MANIFEST),
            (
                "plugins/hello/init.lua",
                r#"local util = require("hello.util")
                   editor.command("hello", function(ctx) ctx:insert(util.greeting()) end)"#,
            ),
            (
                "plugins/hello/lua/hello/util.lua",
                r#"return { greeting = function() return "hello from a folder" end }"#,
            ),
            (
                "plugins/other/ferra-plugin.toml",
                "name = \"other\"\nversion = \"0.1.0\"\n",
            ),
            ("plugins/other/init.lua", ""),
        ],
    )
}

#[test]
fn plugins_install_from_a_folder_of_a_repository() {
    let s = setup();
    let directory = directory_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    install(&mut app, &format!("{directory}//plugins/hello"));
    assert!(
        message(&app).starts_with("Installed hello 0.1.0"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":hello<CR>");
    assert_eq!(text(&app), "hello from a folder");

    // Only that folder is checked out.
    let clone = s.paths.data.join("hello");
    assert!(clone.join("plugins/hello/init.lua").is_file());
    assert!(!clone.join("plugins/other").exists());

    let list = fs::read_to_string(s.paths.config.join("plugins.toml")).unwrap();
    assert!(list.contains("path = \"plugins/hello\""), "{list}");
    keys(
        &mut app,
        &format!(":plugin_install {directory}//plugins/hello<CR>"),
    );
    assert_eq!(
        message(&app),
        format!("{directory}//plugins/hello is already installed as hello")
    );

    // At startup, and restored on a new machine.
    let mut again = app_with(&s.paths);
    assert_eq!(again.load_plugins(&s.paths.config), None);
    keys(&mut again, ":hello<CR>");
    assert_eq!(text(&again), "hello from a folder");
    fs::remove_dir_all(&s.paths.data).unwrap();
    let mut fresh = app_with(&s.paths);
    fresh.load_plugins(&s.paths.config);
    keys(&mut fresh, ":plugin_install<CR>y");
    fresh.wait_for_background();
    assert!(
        message(&fresh).starts_with("Installed hello 0.1.0"),
        "{}",
        message(&fresh)
    );
    keys(&mut fresh, ":hello<CR>");
    assert_eq!(text(&fresh), "hello from a folder");

    keys(
        &mut fresh,
        &format!(":plugin_install {directory}//plugins/nope<CR>y"),
    );
    fresh.wait_for_background();
    assert!(
        message(&fresh).contains("has no folder plugins/nope"),
        "{}",
        message(&fresh)
    );
}

#[test]
fn a_plugin_in_a_folder_only_updates_for_its_own_commits() {
    let s = setup();
    let directory = directory_repo(s.temp.path());
    let mut app = app_with(&s.paths);
    install(&mut app, &format!("{directory}//plugins/hello"));

    commit(
        &directory,
        &[("plugins/other/init.lua", "-- changed")],
        "Change other",
    );
    keys(&mut app, ":plugin_update<CR>");
    app.wait_for_background();
    assert_eq!(message(&app), "hello is up to date");

    commit(
        &directory,
        &[(
            "plugins/hello/lua/hello/util.lua",
            r#"return { greeting = function() return "newer" end }"#,
        )],
        "Greet newer",
    );
    commit(
        &directory,
        &[("plugins/other/init.lua", "-- again")],
        "Change other again",
    );
    keys(&mut app, ":plugin_update<CR>");
    app.wait_for_background();
    let shown = popup_text(&app);
    assert!(shown.contains("Greet newer"), "{shown}");
    assert!(!shown.contains("Change other"), "{shown}");
    keys(&mut app, "y");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Updated hello"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":hello<CR>");
    assert_eq!(text(&app), "newer");
}

#[test]
fn the_directory_lists_its_own_plugins_by_folder() {
    let s = setup();
    let directory = directory_repo(s.temp.path());
    let mut app = app_browsing(&s.paths, &directory);
    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert_eq!(
        picker_items(&app)[0],
        ("hello".to_string(), "Says hello".to_string())
    );
    keys(&mut app, "hello<CR>");
    assert!(
        popup_text(&app).contains("folder plugins/hello"),
        "{}",
        popup_text(&app)
    );
    keys(&mut app, "y");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Installed hello"),
        "{}",
        message(&app)
    );

    keys(&mut app, ":plugin_browse<CR>");
    app.wait_for_background();
    assert_eq!(picker_items(&app)[0].1, "(installed) Says hello");
}

#[test]
fn grammars_build_for_plugins_in_folders() {
    if !have_cc() {
        eprintln!("no C compiler: skipped");
        return;
    }
    let s = setup();
    // The grammar pack, moved into plugins/data of another repository.
    let pack = PathBuf::from(grammar_pack(s.temp.path()));
    let directory = s.temp.path().join("directory");
    fs::create_dir_all(directory.join("plugins")).unwrap();
    fs::rename(&pack, directory.join("plugins/data")).unwrap();
    fs::remove_dir_all(directory.join("plugins/data/.git")).unwrap();
    git(&directory, &["init", "--quiet"]);
    git(&directory, &["add", "."]);
    git(&directory, &["commit", "--quiet", "-m", "first"]);

    let mut app = app_with(&s.paths);
    install(&mut app, &format!("{}//plugins/data", directory.display()));
    assert!(
        message(&app).starts_with("Installed data"),
        "{}",
        message(&app)
    );
    let clone = s.paths.data.join("data");
    assert!(clone.join("plugins/data/.ferra-build").is_dir());
    assert_eq!(git(&clone, &["status", "--porcelain"]), "");

    let file = s.temp.path().join("z.data");
    fs::write(&file, "{\"key\": 12}\n").unwrap();
    app.open(&file);
    assert_eq!(string_cells(&mut app, 0), vec![6, 7, 8, 9, 10]);
}

/// The test grammar as a grammar repository of its own, with its queries.
fn grammar_repo(root: &Path) -> (String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grammar-json");
    let read = |p: &str| fs::read_to_string(fixture.join(p)).unwrap();
    let dir = repo(
        root,
        "tree-sitter-data",
        &[
            ("src/parser.c", &read("src/parser.c")),
            (
                "src/tree_sitter/parser.h",
                &read("src/tree_sitter/parser.h"),
            ),
            ("src/tree_sitter/alloc.h", &read("src/tree_sitter/alloc.h")),
            ("src/tree_sitter/array.h", &read("src/tree_sitter/array.h")),
            ("queries/highlights.scm", &read("queries/highlights.scm")),
        ],
    );
    let rev = git(Path::new(&dir), &["rev-parse", "HEAD"]);
    (dir, rev)
}

#[test]
fn grammars_are_fetched_from_their_repository_at_a_commit() {
    if !have_cc() {
        eprintln!("no C compiler: skipped");
        return;
    }
    let s = setup();
    let (grammar, rev) = grammar_repo(s.temp.path());
    // Later commits to the grammar don't matter: the pack names its rev.
    commit(&grammar, &[("src/parser.c", "this is not C")], "Break it");
    let pack = repo(
        s.temp.path(),
        "pack",
        &[(
            "ferra-plugin.toml",
            &format!(
                "name = \"data\"\nversion = \"0.1.0\"\n[languages.data]\nextensions = [\"data\"]\n\
                 [grammars.data]\ngit = \"{grammar}\"\nrev = \"{rev}\"\nsymbol = \"tree_sitter_json\"\n"
            ),
        )],
    );
    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    assert!(
        message(&app).starts_with("Installed data"),
        "{}",
        message(&app)
    );
    let sources = s.paths.data.join("data/.ferra-build/src");
    assert!(
        sources
            .join(format!("data-{}", &rev[..12]))
            .join("src/parser.c")
            .is_file()
    );

    // The pack has no queries: the grammar's own are used.
    let file = s.temp.path().join("q.data");
    fs::write(&file, "{\"key\": 12}\n").unwrap();
    app.open(&file);
    assert_eq!(string_cells(&mut app, 0), vec![6, 7, 8, 9, 10]);
    assert_eq!(
        git(&s.paths.data.join("data"), &["status", "--porcelain"]),
        ""
    );
}

#[test]
fn a_grammar_from_git_needs_its_commit() {
    let s = setup();
    let pack = repo(
        s.temp.path(),
        "norev",
        &[(
            "ferra-plugin.toml",
            "name = \"norev\"\nversion = \"0.1.0\"\n[languages.norev]\nextensions = [\"nr\"]\n[grammars.norev]\ngit = \"o/r\"\n",
        )],
    );
    let mut app = app_with(&s.paths);
    install(&mut app, &pack);
    assert!(
        message(&app).contains("git needs a rev"),
        "{}",
        message(&app)
    );
}

/// A plugin directory with a language pack for `.zz` files (no grammar).
fn directory_with_pack(root: &Path) -> String {
    repo(
        root,
        "packs",
        &[
            (
                "index.toml",
                "version = 1\n\n[[plugin]]\nname = \"zz\"\npath = \"languages/zz\"\ndescription = \"The zz language\"\nlanguage = true\n",
            ),
            (
                "languages/zz/ferra-plugin.toml",
                "name = \"zz\"\nversion = \"0.1.0\"\n[languages.zz]\nextensions = [\"zz\"]\ncomment = \"#\"\n",
            ),
        ],
    )
}

/// An app that offers language packs from the directory at `index`.
fn app_offering(paths: &PluginPaths, index: &str) -> App {
    let mut app = common::app_with_config(&format!("[editor]\nplugin-index = \"{index}\"\n"));
    app.set_plugin_paths(paths.clone());
    app
}

#[test]
fn opening_an_unknown_file_offers_its_language_pack() {
    let s = setup();
    let directory = directory_with_pack(s.temp.path());
    let file = s.temp.path().join("a.zz");
    fs::write(&file, "x\n").unwrap();

    // No copy of the directory yet: it's fetched, then the pack offered.
    let mut app = app_offering(&s.paths, &directory);
    app.open(&file);
    app.wait_for_background();
    let shown = popup_text(&app);
    assert!(shown.contains("a.zz is zz"), "{shown}");
    assert!(shown.contains("The zz language"), "{shown}");
    assert!(shown.contains("permissions"), "{shown}");
    keys(&mut app, "y");
    app.wait_for_background();
    assert!(
        message(&app).starts_with("Installed zz 0.1.0"),
        "{}",
        message(&app)
    );
    assert_eq!(app.editor().language_of(&app.editor().document), Some("zz"));
    keys(&mut app, "gcc");
    assert_eq!(text(&app), "# x");
}

#[test]
fn offers_can_be_put_off_or_declined() {
    let s = setup();
    let directory = directory_with_pack(s.temp.path());
    let file = s.temp.path().join("b.zz");
    fs::write(&file, "x\n").unwrap();
    let other = s.temp.path().join("c.zz");
    fs::write(&other, "y\n").unwrap();

    let mut app = app_offering(&s.paths, &directory);
    app.open(&file);
    app.wait_for_background();
    assert!(app.editor().popup().is_some());
    keys(&mut app, "n");
    // Not again this session.
    app.open(&other);
    app.wait_for_background();
    assert!(app.editor().popup().is_none());

    // Next session, with the copy of the directory fresh: offered at once.
    let mut again = app_offering(&s.paths, &directory);
    again.open(&file);
    assert!(again.editor().popup().is_some(), "no fetch needed");
    keys(&mut again, "d");
    assert_eq!(
        message(&again),
        "Won't offer zz again (:plugin_install zz installs it)"
    );

    let mut third = app_offering(&s.paths, &directory);
    third.open(&file);
    third.wait_for_background();
    assert!(third.editor().popup().is_none(), "declined");

    // By name, from the directory.
    keys(&mut third, ":plugin_install zz<CR>");
    assert!(
        popup_text(&third).contains("folder languages/zz"),
        "{}",
        popup_text(&third)
    );
    keys(&mut third, "y");
    third.wait_for_background();
    assert!(
        message(&third).starts_with("Installed zz"),
        "{}",
        message(&third)
    );
    keys(&mut third, ":plugin_install nothing<CR>");
    assert_eq!(
        message(&third),
        "No plugin named nothing in the plugin directory (:plugin_browse lists them)"
    );
}

#[test]
fn known_files_never_ask() {
    let s = setup();
    let directory = directory_with_pack(s.temp.path());
    let file = s.temp.path().join("main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    let mut app = app_offering(&s.paths, &directory);
    app.open(&file);
    app.wait_for_background();
    assert!(app.editor().popup().is_none());
    assert!(!s.paths.data.join(".index").exists(), "nothing fetched");
}
