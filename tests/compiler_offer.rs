//! Offering to install a C compiler when a language pack's grammar can't
//! be built without one. A test binary of its own: it points `$CC` at a
//! program that doesn't exist, which would break other tests' grammars.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use ferra::App;
use ferra::editor::Mode;
use ferra::plugins::grammars;
use ferra::plugins::packages::PluginPaths;

fn git(dir: &Path, args: &[&str]) {
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
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// A language pack whose grammar is in the pack (the test grammar).
fn pack(root: &Path) -> String {
    let dir = root.join("pack");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grammar-json");
    for file in [
        "src/parser.c",
        "src/tree_sitter/parser.h",
        "src/tree_sitter/alloc.h",
        "src/tree_sitter/array.h",
    ] {
        let to = dir.join("grammar").join(file);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(fixture.join(file), to).unwrap();
    }
    fs::write(
        dir.join("ferra-plugin.toml"),
        "name = \"data\"\nversion = \"0.1.0\"\n[languages.data]\nextensions = [\"data\"]\n\
         [grammars.data]\npath = \"grammar\"\nsymbol = \"tree_sitter_json\"\n",
    )
    .unwrap();
    git(&dir, &["init", "--quiet"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "--quiet", "-m", "first"]);
    dir.display().to_string()
}

/// An app whose terminal panel runs `cat`, so an install command is only
/// echoed, never run.
fn app(paths: &PluginPaths) -> App {
    let mut app =
        common::app_with_config("[editor]\nplugin-suggestions = false\nshell = \"/bin/cat\"\n");
    app.set_plugin_paths(paths.clone());
    app
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
fn a_missing_compiler_is_offered_once_and_can_be_declined() {
    // SAFETY: this test binary has only this test, so nothing else reads
    // the environment meanwhile.
    unsafe { std::env::set_var("CC", "ferra-no-such-compiler") };
    let install = grammars::compiler_install(std::env::consts::OS, &grammars::os_release());

    let temp = tempfile::tempdir().unwrap();
    let paths = PluginPaths {
        config: temp.path().join("config"),
        data: temp.path().join("data"),
    };
    let source = pack(temp.path());
    let mut app = app(&paths);
    common::keys(&mut app, &format!(":plugin_install {source}<CR>y"));
    app.wait_for_background();
    assert!(
        message(&app).contains("no C compiler (ferra-no-such-compiler)"),
        "{}",
        message(&app)
    );
    let Some(install) = install else {
        // A system ferra doesn't know how to install one on: the message is
        // all there is.
        assert!(app.editor().popup().is_none());
        return;
    };
    let shown = popup_text(&app);
    assert!(
        shown.contains("The data grammar is built from source"),
        "{shown}"
    );
    assert!(shown.contains(&install.command), "{shown}");

    // `y` types the command into the terminal panel (which runs `cat`).
    common::keys(&mut app, "y");
    assert!(app.editor().terminal_panel().is_some(), "the panel opened");
    assert!(message(&app).starts_with(&format!("Running {}", install.command)));
    common::keys(&mut app, "<C-\\><C-n>");

    // Once a session: reloading doesn't ask again.
    common::keys(&mut app, ":terminal_close<CR>:plugin_reload<CR>");
    assert!(app.editor().popup().is_none(), "{}", popup_text(&app));

    // Next session it's offered again, and `d` stops that.
    let mut again = app_for_next_session(&paths);
    assert!(popup_text(&again).contains(&install.command));
    assert_eq!(
        again.editor().mode(),
        Mode::Popup,
        "opening files after the plugins load keeps the offer's keys"
    );
    common::keys(&mut again, "d");
    assert_eq!(
        message(&again),
        "Won't offer a C compiler again; grammars need one to build"
    );
    let third = app_for_next_session(&paths);
    assert!(third.editor().popup().is_none(), "declined");
}

/// As ferra starts: plugins load, then the files given open.
fn app_for_next_session(paths: &PluginPaths) -> App {
    let mut app = app(paths);
    app.load_plugins(&paths.config);
    let file = paths.config.join("notes.txt");
    fs::write(&file, "x").unwrap();
    app.open_all(&[file]);
    app
}
