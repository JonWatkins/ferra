//! Language server commands: stepping through diagnostics, and `:lsp`.

use crate::commands::CommandRegistry;
use crate::editor::Message;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "goto_next_diagnostic",
        "Go to the next diagnostic in this buffer, showing its message (]d)",
        |cx| cx.editor.goto_diagnostic(true, cx.count()),
    );
    r.register(
        "goto_prev_diagnostic",
        "Go to the previous diagnostic in this buffer, showing its message ([d)",
        |cx| cx.editor.goto_diagnostic(false, cx.count()),
    );
    r.register(
        "lsp_info",
        "Show the language servers: which are running, and which aren't installed",
        |cx| {
            let lines = cx.editor.lsp_status();
            let text = if !cx.editor.options.lsp {
                "Language servers are off (lsp = false)".to_string()
            } else if lines.is_empty() {
                "No language servers: none is set for the open files' languages".to_string()
            } else {
                lines.join("  ·  ")
            };
            cx.editor.set_message(Message::info(text));
            Ok(())
        },
    );
    r.alias("lsp", "lsp_info");
}
