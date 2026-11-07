//! Lua plugins reading and setting options, and adding status line
//! elements.

mod common;

use std::fs;

use common::{app, app_with_config, keys, render};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn with_plugin(config: &str, source: &str) -> App {
    let mut app = app_with_config(config);
    app.load_plugin("test.lua", source).expect("plugin loads");
    app
}

const SHOW: &str = r#"
editor.command("show", function(ctx)
  local value = ctx:option(ctx.args[1])
  if type(value) == "table" then value = "list of " .. #value end
  ctx:message(tostring(value))
end)
editor.command("put", function(ctx)
  local value = ctx.args[2]
  if value == "true" then value = true
  elseif value == "false" then value = false
  elseif tonumber(value) then value = tonumber(value) end
  ctx:set_option(ctx.args[1], value, ctx.args[3])
end)
"#;

#[test]
fn ctx_option_reads_options_in_force() {
    let mut app = with_plugin("", SHOW);
    for (name, shown) in [
        ("tab-width", "4"),
        ("ts", "4"),
        ("wrap", "true"),
        ("search.case", "smart"),
        ("statusline-layout.left", "list of 3"),
    ] {
        keys(&mut app, &format!(":show {name}<CR>"));
        assert_eq!(message(&app), shown, "{name}");
    }
    keys(&mut app, ":show nope<CR>");
    assert!(
        message(&app).contains("Unknown option: nope"),
        "{}",
        message(&app)
    );
}

#[test]
fn ctx_set_option_sets_them_globally_or_locally() {
    let mut app = with_plugin("", SHOW);
    keys(&mut app, ":put tab-width 2<CR>");
    assert_eq!(app.editor().options.tab_width, 2);
    assert_eq!(app.editor().global_options().tab_width, 2);
    keys(&mut app, ":put wrap false local<CR>");
    assert!(!app.editor().options.wrap, "here");
    assert!(app.editor().global_options().wrap, "not everywhere");
    keys(&mut app, ":put clipboard off local<CR>");
    assert!(
        message(&app).contains("can't be set locally"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":put line-numbers sideways<CR>");
    assert!(
        message(&app).contains("unknown variant"),
        "{}",
        message(&app)
    );
    keys(&mut app, ":put wrap true nowhere<CR>");
    assert!(
        message(&app).contains("\"local\" or \"global\""),
        "{}",
        message(&app)
    );
}

#[test]
fn a_plugin_status_element_shows_what_its_function_returns() {
    let config = "[editor.statusline-layout]\nleft = [\"mode\", \"plugin:lines\"]\nright = []";
    let mut app = with_plugin(
        config,
        r#"editor.status("lines", function(ctx) return ctx:line_count() .. " lines!" end)"#,
    );
    keys(&mut app, "ia<CR>b<Esc>");
    let screen = render(&mut app, 40, 5);
    assert!(
        screen.rows.iter().any(|r| r.contains("NORMAL  2 lines!")),
        "{:?}",
        screen.rows
    );
    keys(&mut app, "o<Esc>");
    let screen = render(&mut app, 40, 5);
    assert!(
        screen.rows.iter().any(|r| r.contains("3 lines!")),
        "updates"
    );
}

#[test]
fn a_failing_status_function_is_reported_once() {
    let config = "[editor.statusline-layout]\nleft = [\"plugin:broken\"]\nright = []";
    let mut app = with_plugin(
        config,
        r#"editor.status("broken", function(ctx) error("oops", 0) end)"#,
    );
    render(&mut app, 40, 5);
    assert_eq!(message(&app), "plugin:broken: oops");
    keys(&mut app, ":show wrap<CR>");
    render(&mut app, 40, 5);
    assert_ne!(
        message(&app),
        "plugin:broken: oops",
        "not again every frame"
    );
}

#[test]
fn a_status_element_no_plugin_provides_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("plugins")).unwrap();
    fs::write(dir.path().join("plugins/x.lua"), "-- nothing").unwrap();
    let mut app =
        app_with_config("[editor.statusline-layout]\nleft = [\"plugin:missing\"]\nright = []");
    let error = app.load_plugins(dir.path()).unwrap_or_default();
    assert!(
        error.contains("no plugin provides plugin:missing"),
        "{error}"
    );
}

#[test]
fn plugin_elements_need_a_name() {
    let bad = "[editor.statusline-layout]\nleft = [\"plugin:\"]\nright = []";
    assert!(ferra::Config::from_toml(bad).is_err());
    assert!(
        ferra::Config::from_toml("[editor.statusline-layout]\nleft = [\"plugin:x\"]\nright = []")
            .is_ok()
    );
    let _ = app();
}
