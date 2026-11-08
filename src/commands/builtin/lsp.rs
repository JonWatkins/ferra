//! Language server commands: hover, going to definitions and references,
//! stepping through diagnostics, and `:lsp`.

use crate::commands::CommandRegistry;
use crate::editor::Message;
use crate::editor::lsp::LspFeature;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "hover",
        "Show what the language server says about the symbol under the cursor; again to scroll (K)",
        |cx| {
            if cx.editor.scroll_hover() {
                return Ok(());
            }
            cx.editor.request_lsp(LspFeature::Hover)
        },
    );
    let gotos = [
        (
            "goto_definition",
            "Go to the definition of the symbol under the cursor (gd)",
            LspFeature::Definition,
        ),
        (
            "goto_declaration",
            "Go to the declaration of the symbol under the cursor (gD)",
            LspFeature::Declaration,
        ),
        (
            "goto_type_definition",
            "Go to the definition of the type of the symbol under the cursor (gy)",
            LspFeature::TypeDefinition,
        ),
        (
            "goto_implementation",
            "List the implementations of the trait or interface under the cursor (gI)",
            LspFeature::Implementation,
        ),
        (
            "goto_references",
            "List the references to the symbol under the cursor (gr)",
            LspFeature::References,
        ),
    ];
    for (name, description, feature) in gotos {
        r.register(name, description, move |cx| cx.editor.request_lsp(feature));
    }
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
