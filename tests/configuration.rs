//! User configuration is merged over the defaults and changes behaviour.

mod common;

use common::{app_with_config, cursor, keys, render, text};
use terminal_editor::{Config, config::LineNumbers, editor::Mode};

#[test]
fn user_config_overrides_only_what_it_sets() {
    let config = Config::from_toml("[editor]\ntab-width = 2").unwrap();
    assert_eq!(config.editor.tab_width, 2);
    assert_eq!(config.editor.line_numbers, LineNumbers::Relative);
}

#[test]
fn tab_width_option_is_used() {
    let mut app = app_with_config("[editor]\ntab-width = 2");
    keys(&mut app, "i<Tab>x<Esc>");
    assert_eq!(text(&app), "  x");
}

#[test]
fn keys_can_be_added() {
    let mut app = app_with_config(
        r#"
        [keys.insert]
        "jk" = "normal_mode"
        "#,
    );
    keys(&mut app, "ihijk");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(text(&app), "hi");
}

#[test]
fn abandoned_insert_sequence_types_its_keys() {
    let mut app = app_with_config(
        r#"
        [keys.insert]
        "jk" = "normal_mode"
        "#,
    );
    keys(&mut app, "ijam jj<Esc>");
    assert_eq!(text(&app), "jam jj");
}

#[test]
fn keys_can_be_remapped_and_defaults_kept() {
    let mut app = app_with_config(
        r#"
        [keys.normal]
        "<C-q>" = "quit"
        "#,
    );
    keys(&mut app, "ia<CR>b<Esc>k");
    assert_eq!(cursor(&app), (0, 0), "default `k` still works");
    keys(&mut app, "<C-q>");
    assert!(app.should_quit());
}

#[test]
fn keys_can_be_unbound() {
    let mut app = app_with_config(
        r#"
        [keys.normal]
        "x" = ""
        "#,
    );
    keys(&mut app, "iabc<Esc>x");
    assert_eq!(text(&app), "abc");
}

#[test]
fn unknown_command_in_keymap_is_reported() {
    let app = app_with_config(
        r#"
        [keys.normal]
        "Q" = "not_a_command"
        "#,
    );
    let message = app.editor().message().expect("a warning");
    assert!(
        message.text.contains("not_a_command (normal)"),
        "{}",
        message.text
    );
}

#[test]
fn line_numbers_can_be_disabled() {
    let mut app = app_with_config("[editor]\nline-numbers = \"none\"");
    keys(&mut app, "ihello<Esc>");
    let screen = render(&mut app, 20, 4);
    assert!(screen.rows[0].starts_with("hello"), "{:?}", screen.rows[0]);
}

#[test]
fn invalid_config_values_are_errors() {
    for (toml, needle) in [
        ("[editor]\nline-numbers = \"sideways\"", "sideways"),
        ("[theme]\n\"ui.text\" = \"nope\"", "invalid color"),
        ("[keys.normal]\n\"<Nope>\" = \"quit\"", "<Nope>"),
        ("[keys.visual]\n\"x\" = \"quit\"", "visual"),
        ("[unknown]", "unknown"),
    ] {
        let err = Config::from_toml(toml).expect_err(toml).to_string();
        assert!(err.contains(needle), "{toml:?} gave {err:?}");
    }
}

#[test]
fn shipped_default_config_parses_as_user_config() {
    // Users are told to copy the defaults as a starting point, so the
    // defaults must also be valid when supplied as a user config.
    Config::from_toml(Config::default_toml()).unwrap();
}

#[test]
fn readme_example_config_is_valid() {
    let readme = include_str!("../README.md");
    let example = readme
        .split("```toml")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("README contains a ```toml example");
    Config::from_toml(example).unwrap();
}
