//! `:set`, `:setlocal`, `:setglobal`: reading and changing options while
//! running (see [`config::set`](crate::config::set)).
//!
//! Some options can also have a value of the active window's own (`wrap`,
//! `line-numbers`, `cursorline`, `scroll-off`) or buffer's own (`tab-width`,
//! `expand-tab`, `auto-indent`), as in Vim. `:set` changes the global value
//! and drops the active window's or buffer's own; `:setlocal` gives them
//! one; `:setglobal` changes only the global value, for windows and
//! buffers without their own.

use crate::commands::{ArgCompletion, CommandRegistry, CommandResult, Context};
use crate::config::set::{self, SetResult};
use crate::editor::{Editor, Message};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "set_option",
        "Show or change options: name=value, name, noname, name!, name?, name& (no arguments: those changed)",
        set_option,
    )
    .complete_args(ArgCompletion::Option);
    r.alias("set", "set_option");
    r.alias("se", "set_option");

    r.register(
        "set_local_option",
        "Like :set, for the active window or buffer only (no arguments: its own values)",
        set_local_option,
    )
    .complete_args(ArgCompletion::Option);
    r.alias("setlocal", "set_local_option");
    r.alias("setl", "set_local_option");

    r.register(
        "set_global_option",
        "Like :set, leaving windows and buffers with their own values alone",
        set_global_option,
    )
    .complete_args(ArgCompletion::Option);
    r.alias("setglobal", "set_global_option");
    r.alias("setg", "set_global_option");
}

/// `:set`: works on the options in force (so `:set ts?` shows the value
/// used here), then makes the changes global and drops the active window's
/// and buffer's own values for them.
fn set_option(cx: &mut Context) -> CommandResult {
    let mut options = cx.editor.options.clone();
    let result = set::set(&mut options, cx.args)?;
    let mut global = cx.editor.global_options().clone();
    set::copy_values(&options, &mut global, &result.changed);
    cx.editor.set_global_options(global);
    cx.editor.follow_global_options(&result.changed);
    finish(cx.editor, &result);
    Ok(())
}

fn set_global_option(cx: &mut Context) -> CommandResult {
    let mut global = cx.editor.global_options().clone();
    let result = set::set(&mut global, cx.args)?;
    cx.editor.set_global_options(global);
    finish(cx.editor, &result);
    Ok(())
}

fn set_local_option(cx: &mut Context) -> CommandResult {
    if cx.args.is_empty() {
        let shown = cx.editor.local_options();
        let text = if shown.is_empty() {
            "No options set for this window or buffer".to_string()
        } else {
            shown.join("  ")
        };
        cx.editor.set_message(Message::info(text));
        return Ok(());
    }
    let mut options = cx.editor.options.clone();
    let result = set::set(&mut options, cx.args)?;
    if let Some(name) = result.changed.iter().find(|n| set::scope(n).is_none()) {
        return Err(format!(
            "{name} is the same everywhere: use :set (a window or buffer can have its own wrap, wrap-indent, wrap-marker, line-numbers, cursorline, scroll-off, tab-width, expand-tab, auto-indent, text-width)"
        ));
    }
    for name in &result.changed {
        if let Some(value) = set::value_of(&options, name) {
            cx.editor.set_local_option(name, value);
        }
    }
    finish(cx.editor, &result);
    Ok(())
}

/// Re-applies what was put in place at startup, and reports the values.
fn finish(editor: &mut Editor, result: &SetResult) {
    editor.apply_option_effects(&result.changed);
    let changed = |name: &str| result.changed.iter().any(|c| c == name);
    let mut text = result.shown.join("  ");
    if changed("kitty-keyboard") {
        text.push_str(" (from the next start)");
    }
    editor.set_message(Message::info(text));
}
