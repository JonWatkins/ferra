//! Colour themes: ferra's own and theme packs', chosen with `color-theme`
//! or `:theme` (which shows each as the selection moves), saved to
//! config.toml, with the user's `[theme]` still over them.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::keys;
use ferra::App;
use ferra::editor::Mode;
use ferra::plugins::packages::PluginPaths;
use ratatui::{Terminal, backend::TestBackend, style::Color};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap();
    assert!(status.success());
}

/// A theme pack: "paper", whose background is #fafafa.
fn theme_pack(root: &Path) -> String {
    let dir = root.join("ferra-paper");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("ferra-plugin.toml"),
        "name = \"paper\"\nversion = \"0.1.0\"\ndescription = \"A light theme\"\n\n\
         [themes.paper]\nname = \"Paper\"\nkind = \"light\"\nfile = \"paper.toml\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("paper.toml"),
        "\"ui.background\" = { bg = \"#fafafa\" }\n\"ui.text\" = \"#202020\"\n",
    )
    .unwrap();
    git(&dir, &["init", "--quiet"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "--quiet", "-m", "first"]);
    dir.display().to_string()
}

struct Setup {
    temp: tempfile::TempDir,
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

fn app_with(s: &Setup, config: &str) -> App {
    let mut app =
        common::app_with_config(&format!("[editor]\nplugin-suggestions = false\n{config}"));
    app.set_plugin_paths(s.paths.clone());
    app.editor_mut()
        .set_config_file(Some(s.temp.path().join("config.toml")));
    app
}

fn install_paper(s: &Setup, app: &mut App) {
    let source = theme_pack(s.temp.path());
    keys(app, &format!(":plugin_install {source}<CR>y"));
    app.wait_for_background();
}

/// The background colour drawn in the text's area (low down at the
/// right, clear of a picker).
fn background(app: &mut App) -> Color {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer()[(98, 25)].bg
}

const OWN: Color = Color::Rgb(0x16, 0x18, 0x1d);
const PAPER: Color = Color::Rgb(0xfa, 0xfa, 0xfa);

#[test]
fn a_pack_s_theme_is_picked_previewed_and_saved() {
    let s = setup();
    let mut app = app_with(&s, "");
    assert_eq!(background(&mut app), OWN, "ferra's own");
    install_paper(&s, &mut app);

    keys(&mut app, ":theme<CR>");
    assert_eq!(app.editor().mode(), Mode::Picker);
    let picker = app.editor().picker().unwrap();
    assert_eq!(picker.title(), "Color Theme");
    let labels: Vec<String> = picker
        .matches()
        .iter()
        .map(|m| picker.item(m).label.clone())
        .collect();
    assert_eq!(labels, ["Ferra Dark", "Paper", "Browse more themes…"]);
    assert_eq!(
        picker.selected_item().unwrap().label,
        "Ferra Dark",
        "the one in use"
    );

    // Moving to it shows it; Esc puts back the one in use.
    keys(&mut app, "<Down>");
    assert_eq!(background(&mut app), PAPER, "previewed");
    keys(&mut app, "<Esc>");
    assert_eq!(background(&mut app), OWN);

    // Choosing it: in use, and saved.
    keys(&mut app, ":theme<CR><Down><CR>");
    assert_eq!(background(&mut app), PAPER);
    assert_eq!(app.editor().global_options().color_theme, "paper");
    let config = fs::read_to_string(s.temp.path().join("config.toml")).unwrap();
    assert!(config.contains("color-theme = \"paper\""), "{config}");
}

#[test]
fn the_users_own_colours_stay_over_a_theme() {
    let s = setup();
    let mut app = app_with(&s, "[theme]\n\"ui.background\" = { bg = \"#000000\" }\n");
    install_paper(&s, &mut app);
    keys(&mut app, ":color_theme paper<CR>");
    assert_eq!(app.editor().global_options().color_theme, "paper");
    assert_eq!(background(&mut app), Color::Rgb(0, 0, 0));
}

#[test]
fn a_theme_that_isnt_installed_says_so() {
    let s = setup();
    let mut app = app_with(&s, "");
    keys(&mut app, ":color_theme nowhere<CR>");
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(message.contains("No theme nowhere installed"), "{message}");
    assert_eq!(background(&mut app), OWN, "ferra's own instead");
}
