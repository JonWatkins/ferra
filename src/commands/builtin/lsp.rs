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
    r.register(
        "format",
        "Have the language server format the buffer",
        |cx| cx.editor.format(false),
    );
    r.alias("fmt", "format");
    r.register(
        "rename",
        "Rename the symbol under the cursor to [name], everywhere the language server finds it",
        |cx| {
            let name = cx.args.join(" ");
            cx.editor.request_rename(&name)
        },
    );
    r.register(
        "signature_help",
        "Show the signature of the call the cursor is in (<C-s> in insert mode)",
        |cx| cx.editor.request_signature_help(),
    );
    r.register(
        "symbol_picker",
        "Pick one of this file's symbols (functions, types...) to go to (<Space>s)",
        |cx| cx.editor.request_lsp(LspFeature::DocumentSymbols),
    );
    r.register(
        "project_symbol_picker",
        "Search the project's symbols to go to one, as the language server finds them (<Space>S)",
        |cx| cx.editor.request_workspace_symbols(),
    );
    r.register(
        "code_actions",
        "List the language server's fixes and changes for the cursor or selection, to make one (<Space>a)",
        |cx| cx.editor.request_code_actions(),
    );
    r.register(
        "rename_prompt",
        "Rename the symbol under the cursor: :rename with its name to edit (<F2>, <Space>n)",
        |cx| {
            let word = cx.editor.word_under_cursor(false).unwrap_or_default();
            cx.editor.open_command_line();
            for c in format!("rename {word}").chars() {
                cx.editor.command_line.push(c);
            }
            Ok(())
        },
    );
    // `format-on-save`: before the hooks from config, so theirs see the
    // formatted text. Not formatting doesn't stop the save.
    r.on(
        crate::events::EventKind::BeforeSave,
        "format-on-save",
        |cx, _| {
            if cx.editor.options.format_on_save
                && let Err(error) = cx.editor.format(true)
            {
                cx.editor.note_for_save(format!("not formatted: {error}"));
            }
            Ok(())
        },
    );
}
