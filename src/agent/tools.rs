//! The tools ferra gives the coding agent (over MCP, `mcp.rs`): what's
//! open and selected, the diagnostics, the language servers' answers about
//! a symbol, and showing the user a file. Kept few and short, as each
//! one's description goes in the agent's prompt.

use serde_json::{Value, json};

use crate::editor::Editor;
use crate::editor::lsp::LspFeature;

/// What the agent's told about ferra's tools when it connects.
pub const INSTRUCTIONS: &str = "Tools for the editor (ferra) the user is working in: what they \
have open and selected, the language servers' diagnostics, definitions, references, types and \
symbols (prefer these to searching or reading files to find a symbol), and showing them a file.";

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
            "name": "lsp_definition",
            "description": "Where a symbol is defined, from the language server: path:line:column and the line.",
            "inputSchema": place("to look up"),
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
        "lsp_definition" | "lsp_references" | "lsp_hover" => {
            let feature = match name {
                "lsp_definition" => LspFeature::Definition,
                "lsp_references" => LspFeature::References,
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
