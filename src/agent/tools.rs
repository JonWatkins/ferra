//! The tools ferra gives the coding agent (over MCP, `mcp.rs`): what's
//! open and selected, the diagnostics, the language servers' answers about
//! a symbol, and showing the user a file. Kept few and short, as each
//! one's description goes in the agent's prompt.

use serde_json::{Value, json};

use crate::editor::Editor;
use crate::editor::lsp::LspFeature;

/// What the agent's told about ferra's tools when it connects.
pub const INSTRUCTIONS: &str = "Tools for the editor (ferra) the user is working in. Read and \
edit files with read_buffer and edit_buffer rather than your own Read and Edit tools: they see \
the user's unsaved changes, and edits show in the editor as they're made and can be undone. Use \
the language server tools (definitions, references, types, symbols, diagnostics) rather than \
searching to find your way around, and lsp_rename to rename a symbol everywhere.";

/// The tools, as MCP lists them.
pub fn list() -> Value {
    let place = |what: &str| {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file (relative to the project, or absolute)" },
                "line": { "type": "integer", "description": "The line the symbol is on, from 1" },
                "symbol": { "type": "string", "description": format!("The symbol {what}, as written on that line") },
                "column": { "type": "integer", "description": "Instead of symbol: its column, from 1" },
            },
            "required": ["path", "line"],
        })
    };
    let nothing = json!({ "type": "object", "properties": {} });
    json!([
        {
            "name": "editor_state",
            "description": "What the user has open in the editor now: the current file and cursor, the selection (its text), and the open files (unsaved ones marked). Use it when they say 'this', 'here' or 'the selection'.",
            "inputSchema": nothing,
        },
        {
            "name": "get_diagnostics",
            "description": "Errors and warnings from the language servers (compiler, linter) for a file, or every file reported on. Quicker than building to see problems.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "One file (all, if left out)" } },
            },
        },
        {
            "name": "read_buffer",
            "description": "A file's text as the editor has it (unsaved changes included), lines numbered. Use instead of Read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "line": { "type": "integer", "description": "The first line, from 1" },
                    "limit": { "type": "integer", "description": "How many lines (2000 if left out)" },
                },
                "required": ["path"],
            },
        },
        {
            "name": "edit_buffer",
            "description": "Replace old_text with new_text in a file, in the editor: shown as it happens, undoable, saved unless the user has unsaved changes there. old_text must be there exactly once (with its whitespace), unless replace_all. Use instead of Edit, for existing files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" },
                    "replace_all": { "type": "boolean" },
                },
                "required": ["path", "old_text", "new_text"],
            },
        },
        {
            "name": "lsp_definition",
            "description": "Where a symbol is defined (or its type, its implementations, its declaration: kind), from the language server: path:line:column and the line.",
            "inputSchema": with_kind(place("to look up")),
        },
        {
            "name": "lsp_references",
            "description": "Every use of a symbol in the project, from the language server: path:line:column and the line.",
            "inputSchema": place("to find the uses of"),
        },
        {
            "name": "lsp_hover",
            "description": "A symbol's type and documentation, from the language server.",
            "inputSchema": place("to describe"),
        },
        {
            "name": "lsp_symbols",
            "description": "The functions, types and so on in a file (path), or in the whole project matching a name (query), from the language server, with where each is.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "A file: its symbols" },
                    "query": { "type": "string", "description": "Or: a name to find in the project" },
                },
            },
        },
        {
            "name": "lsp_rename",
            "description": "Rename a symbol everywhere it's used, with the language server (more reliable than replacing text). Saves the files changed, unless the user has unsaved changes there.",
            "inputSchema": with_name(place("to rename")),
        },
        {
            "name": "lsp_code_actions",
            "description": "The language server's fixes and refactors for a place (quick fixes for a diagnostic, imports...), numbered; lsp_apply_code_action makes one.",
            "inputSchema": place("they're for"),
        },
        {
            "name": "lsp_apply_code_action",
            "description": "Make one of the code actions lsp_code_actions listed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file they were for" },
                    "list": { "type": "integer", "description": "The list's number, as lsp_code_actions said" },
                    "number": { "type": "integer", "description": "The action's number in it, from 1" },
                },
                "required": ["path", "list", "number"],
            },
        },
        {
            "name": "lsp_format",
            "description": "Format a file with its language server's formatter.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        },
        {
            "name": "open_file",
            "description": "Show the user a file in the editor, at a line. Only when it helps them to see it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "line": { "type": "integer", "description": "From 1" },
                },
                "required": ["path"],
            },
        },
    ])
}

/// A place's schema, with `kind` (which of the definitions).
fn with_kind(mut schema: Value) -> Value {
    schema["properties"]["kind"] = json!({
        "type": "string",
        "enum": ["definition", "type", "implementation", "declaration"],
        "description": "definition if left out",
    });
    schema
}

/// A place's schema, with the `new_name` to rename to.
fn with_name(mut schema: Value) -> Value {
    schema["properties"]["new_name"] = json!({ "type": "string" });
    schema["required"] = json!(["path", "line", "new_name"]);
    schema
}

/// What a tool call came to: an answer now, or one on its way (a language
/// server's, which comes back to [`Editor::agent_lsp_answer`]).
#[derive(Debug, PartialEq)]
pub enum Called {
    Done(Result<String, String>),
    Waiting,
}

/// Runs tool `name` with `args`, as call `call`.
pub fn call(editor: &mut Editor, call: u64, name: &str, args: &Value) -> Called {
    let text = |key: &str| args[key].as_str().filter(|s| !s.is_empty());
    let number = |key: &str| args[key].as_u64().map(|n| n as usize);
    let flag = |key: &str| args[key].as_bool().unwrap_or(false);
    let ask = |editor: &mut Editor, feature, path, query| {
        let place = (number("line"), text("symbol"), number("column"));
        match editor.agent_ask_lsp(call, feature, path, place, query) {
            Ok(()) => Called::Waiting,
            Err(error) => Called::Done(Err(error)),
        }
    };
    match name {
        "editor_state" => Called::Done(Ok(editor.agent_editor_state())),
        "get_diagnostics" => Called::Done(Ok(editor.agent_diagnostics(text("path")))),
        "open_file" => match text("path") {
            Some(path) => Called::Done(editor.agent_open_file(path, number("line"))),
            None => Called::Done(Err("Which file? (path)".into())),
        },
        "read_buffer" => match text("path") {
            Some(path) => Called::Done(editor.agent_read_buffer(path, number("line"), number("limit"))),
            None => Called::Done(Err("Which file? (path)".into())),
        },
        "edit_buffer" => match (text("path"), args["old_text"].as_str(), args["new_text"].as_str()) {
            (Some(path), Some(old), Some(new)) => {
                Called::Done(editor.agent_edit_buffer(path, old, new, flag("replace_all")))
            }
            _ => Called::Done(Err("Give path, old_text and new_text".into())),
        },
        "lsp_rename" => match (text("path"), text("new_name")) {
            (Some(path), Some(name)) => ask(editor, LspFeature::Rename, Some(path), Some(name)),
            _ => Called::Done(Err("Give path, line, the symbol and new_name".into())),
        },
        "lsp_format" => match text("path") {
            Some(path) => ask(editor, LspFeature::Format, Some(path), None),
            None => Called::Done(Err("Which file? (path)".into())),
        },
        "lsp_apply_code_action" => match (text("path"), args["list"].as_u64(), number("number")) {
            (Some(path), Some(list), Some(n)) => match editor.agent_apply_code_action(call, path, list, n) {
                Ok(()) => Called::Waiting,
                Err(error) => Called::Done(Err(error)),
            },
            _ => Called::Done(Err("Give path, list and number".into())),
        },
        "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_code_actions" => {
            let feature = match (name, text("kind")) {
                ("lsp_definition", Some("type")) => LspFeature::TypeDefinition,
                ("lsp_definition", Some("implementation")) => LspFeature::Implementation,
                ("lsp_definition", Some("declaration")) => LspFeature::Declaration,
                ("lsp_definition", _) => LspFeature::Definition,
                ("lsp_references", _) => LspFeature::References,
                ("lsp_code_actions", _) => LspFeature::CodeAction,
                _ => LspFeature::Hover,
            };
            match text("path") {
                Some(path) => ask(editor, feature, Some(path), None),
                None => Called::Done(Err("Which file? (path)".into())),
            }
        }
        "lsp_symbols" => match (text("path"), text("query")) {
            (Some(path), _) => ask(editor, LspFeature::DocumentSymbols, Some(path), None),
            (None, Some(query)) => ask(editor, LspFeature::WorkspaceSymbols, None, Some(query)),
            (None, None) => Called::Done(Err("Give a file (path) or a name to find (query)".into())),
        },
        other => Called::Done(Err(format!("No tool named {other}"))),
    }
}
