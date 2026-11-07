//! The command registry is the extension point plugins will use. These tests
//! pin down that contract: new commands can be registered at runtime, bound
//! via config, invoked from `:`, and receive counts and arguments.

mod common;

use common::{app_with_config, keys, text};
use ferra::commands::Invocation;

#[test]
fn custom_command_can_be_bound_and_run() {
    let mut app = app_with_config(
        r#"
        [keys.normal]
        "<Space>s" = "shout"
        "#,
    );
    app.commands_mut().register("shout", "Insert SHOUT", |cx| {
        let n = cx.count();
        cx.editor.document.insert_str(&"SHOUT".repeat(n));
        Ok(())
    });

    keys(&mut app, "2<Space>s");
    assert_eq!(text(&app), "SHOUTSHOUT");
}

#[test]
fn custom_command_is_available_on_command_line_with_args() {
    let mut app = common::app();
    app.commands_mut()
        .register("say", "Insert arguments", |cx| {
            let words = cx.args.join(" ");
            cx.editor.document.insert_str(&words);
            Ok(())
        });
    app.commands_mut().alias("s", "say");

    keys(&mut app, ":s hello there<CR>");
    assert_eq!(text(&app), "hello there");
}

#[test]
fn command_errors_surface_as_messages() {
    let mut app = common::app();
    app.commands_mut()
        .register("fail", "Always fails", |_| Err("it broke".into()));
    keys(&mut app, ":fail<CR>");
    assert_eq!(app.editor().message().unwrap().text, "it broke");
}

#[test]
fn commands_can_call_other_commands() {
    let mut app = common::app();
    app.commands_mut()
        .register("top_and_insert", "gg then i", |cx| {
            cx.registry
                .execute("goto_file_start", cx.editor, Invocation::default())?;
            cx.registry
                .execute("insert_mode", cx.editor, Invocation::default())
        });
    keys(&mut app, "ia<CR>b<Esc>:top_and_insert<CR>X");
    assert_eq!(text(&app), "Xa\nb");
}

#[test]
fn custom_commands_receive_selected_register() {
    let mut app = app_with_config(
        r#"
        [keys.normal]
        "Q" = "which_register"
        "#,
    );
    app.commands_mut()
        .register("which_register", "Insert register name", |cx| {
            let name = cx.register.unwrap_or('-');
            cx.editor.document.insert_str(&name.to_string());
            Ok(())
        });
    keys(&mut app, "\"zQ");
    assert_eq!(text(&app), "z");
}
