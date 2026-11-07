//! `:set`: reading and changing options while running (see
//! [`config::set`](crate::config::set)).

use crate::clipboard;
use crate::commands::{ArgCompletion, CommandRegistry, CommandResult, Context};
use crate::config::set;
use crate::editor::Message;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "set_option",
        "Show or change options: name=value, name, noname, name!, name?, name& (no arguments: those changed)",
        set_option,
    )
    .complete_args(ArgCompletion::Option);
    r.alias("set", "set_option");
    r.alias("se", "set_option");
}

fn set_option(cx: &mut Context) -> CommandResult {
    let result = set::set(&mut cx.editor.options, cx.args)?;
    let options = &cx.editor.options;
    let changed = |name: &str| result.changed.iter().any(|c| c == name);
    // Most options are read when they're used; these were put in place at
    // startup.
    if changed("clipboard-provider") {
        let provider = clipboard::for_provider(options.clipboard_provider);
        cx.editor
            .registers
            .set_clipboard(provider, options.clipboard);
    } else if changed("clipboard") {
        cx.editor.registers.set_clipboard_mode(options.clipboard);
    }
    let mut text = result.shown.join("  ");
    if changed("kitty-keyboard") {
        text.push_str(" (from the next start)");
    }
    cx.editor.set_message(Message::info(text));
    Ok(())
}
