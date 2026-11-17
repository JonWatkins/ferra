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
searching to find your way around, and lsp_rename to rename a symbol everywhere. Files you \
change from your shell (a formatter, sed, codegen) reload in the editor, except where the user \
has unsaved changes: those keep the user's text, so change them with edit_buffer instead.";

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
                "occurrence": { "type": "integer", "description": "Which of the symbol's places on the line, from 1 (1 if left out)" },
            },
            "required": ["path", "line"],
        })
    };
    let nothing = json!({ "type": "object", "properties": {} });
    // References: every use, or the callers or callees of a function.
    let mut references = place("to find the uses of");
    references["properties"]["kind"] = json!({
        "type": "string",
        "enum": ["references", "callers", "callees"],
        "description": "references if left out",
    });
    // Code actions: a place, or just a file (its file-wide ones).
    let mut actions = place("they're for");
    actions["required"] = json!(["path"]);
    json!([
        {
            "name": "editor_state",
            "description": "What the user has open in the editor now: the current file and cursor, the selection (its text), and the open files (unsaved ones marked). Use it when they say 'this', 'here' or 'the selection'.",
            "inputSchema": nothing,
        },
        {
            "name": "buffer_changes",
            "description": "The user's changes as a unified diff: a file as the editor has it (unsaved edits included) against the saved file (against: saved, the default) or against git's staged text (against: git, what the gutter shows). With no path, every open file with such changes. Use it for 'review what I wrote' or 'finish what I started'; git diff from the shell can't see unsaved edits.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "One file (every open one with changes, if left out)" },
                    "against": { "type": "string", "enum": ["saved", "git"], "description": "saved if left out" },
                },
            },
        },
        {
            "name": "language_servers",
            "description": "The language servers: each one's state (starting, running, busy indexing, stopped and why), how many files it has open, those not installed, and what they've logged lately. Use it when the lsp_ tools answer nothing or fail, to tell a server that's missing, crashed or still indexing from one with no answer.",
            "inputSchema": nothing,
        },
        {
            "name": "read_terminal",
            "description": "The text in the editor's terminal panel (the user's shell, scrollback included) and whether a command is still running there. Use it when the user mentions output they just saw (a failing build or test, an error, a server's log) instead of running the command again.",
            "inputSchema": {
                "type": "object",
                "properties": { "lines": { "type": "integer", "description": "How many of the last lines (200 if left out)" } },
            },
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
            "description": "Replace old_text with new_text in a file, in the editor: shown as it happens, undoable, saved unless the user has unsaved changes there; the result says what's wrong in the file after. old_text must be there exactly once (with its whitespace), unless replace_all. For several changes to a file at once, give edits (a list of old_text/new_text) instead: one undo step, all or none. Use instead of Edit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" },
                    "replace_all": { "type": "boolean" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_text": { "type": "string" },
                                "new_text": { "type": "string" },
                                "replace_all": { "type": "boolean" },
                            },
                            "required": ["old_text", "new_text"],
                        },
                    },
                },
                "required": ["path"],
            },
        },
        {
            "name": "create_file",
            "description": "Make a new file with this text (not one that exists), through the editor; the result says what's wrong in it. Use instead of Write for new files.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" }, "text": { "type": "string" } },
                "required": ["path", "text"],
            },
        },
        {
            "name": "lsp_definition",
            "description": "Where a symbol is defined (or its type, its implementations, its declaration: kind), from the language server: path:line:column and the line.",
            "inputSchema": with_kind(place("to look up")),
        },
        {
            "name": "lsp_references",
            "description": "Every use of a symbol in the project, from the language server: path:line:column and the line. With kind callers: the functions that call it (and on which lines); callees: the functions it calls. Those answer 'who calls this?' and 'what does this do?' better than every mention.",
            "inputSchema": references,
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
            "description": "The language server's fixes and refactors for a place (quick fixes for a diagnostic, imports...), numbered; lsp_apply_code_action makes one. With no line: the file-wide ones (organize imports, fix all, remove unused imports).",
            "inputSchema": actions,
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
            "name": "move_file",
            "description": "Move or rename a file: the language server updates what refers to it (imports, mod paths) first, as edits that can be undone, then the file moves and its open buffer follows. Use instead of mv. Its answer says what changed and the problems after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "The file" },
                    "to": { "type": "string", "description": "Its new path (folders are made)" },
                },
                "required": ["from", "to"],
            },
        },
        {
            "name": "show_locations",
            "description": "Give the user a list of places to step through, in the editor's panel (a Locations tab, by file; they press ↵ to go to one): the answer to 'where does this need changing?', the callers to update, the places a change touched. Better than paths in your reply when there are several. Each with a short note of what's there.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "What they are (\"Callers of parse to update\")" },
                    "locations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "line": { "type": "integer", "description": "From 1" },
                                "column": { "type": "integer", "description": "From 1 (1 if left out)" },
                                "note": { "type": "string", "description": "What's there (the line's text if left out)" },
                            },
                            "required": ["path", "line"],
                        },
                    },
                },
                "required": ["title", "locations"],
            },
        },
        {
            "name": "close_file",
            "description": "Close a file's buffer in the editor (one you opened for the user and they're done with). Files you only read or edited close by themselves when your turn ends.",
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
    /// A file changed: the answer, once its language server has had a
    /// moment to say what's wrong in it now.
    Changed(String, Vec<crate::editor::DocumentId>),
}

/// Runs tool `name` with `args`, as call `call`.
pub fn call(editor: &mut Editor, call: u64, name: &str, args: &Value) -> Called {
    // Files changed outside the editor (from the agent's shell, a
    // formatter) are read again first, as the user's idle check would: a
    // tool never works from text the disk has moved past. Buffers with the
    // user's unsaved changes are kept, and warned about.
    if editor.options.watch_files {
        editor.check_disk_changes();
    }
    let text = |key: &str| args[key].as_str().filter(|s| !s.is_empty());
    let number = |key: &str| args[key].as_u64().map(|n| n as usize);
    let ask = |editor: &mut Editor, feature, path, query| {
        let place = (
            number("line"),
            text("symbol"),
            number("column"),
            number("occurrence"),
        );
        match editor.agent_ask_lsp(call, feature, path, place, query) {
            Ok(()) => Called::Waiting,
            Err(error) => Called::Done(Err(error)),
        }
    };
    match name {
        "editor_state" => Called::Done(Ok(editor.agent_editor_state())),
        "get_diagnostics" => Called::Done(Ok(editor.agent_diagnostics(text("path")))),
        "read_terminal" => Called::Done(editor.agent_read_terminal(number("lines"))),
        "language_servers" => Called::Done(Ok(editor.agent_language_servers())),
        "buffer_changes" => {
            Called::Done(editor.agent_buffer_changes(text("path"), text("against")))
        }
        "open_file" => match text("path") {
            Some(path) => Called::Done(editor.agent_open_file(path, number("line"))),
            None => Called::Done(Err("Which file? (path)".into())),
        },
        "read_buffer" => match text("path") {
            Some(path) => {
                Called::Done(editor.agent_read_buffer(path, number("line"), number("limit")))
            }
            None => Called::Done(Err("Which file? (path)".into())),
        },
        "edit_buffer" => {
            let Some(path) = text("path") else {
                return Called::Done(Err("Which file? (path)".into()));
            };
            // One replacement, or several (`edits`).
            let one = |e: &Value| -> Option<(String, String, bool)> {
                Some((
                    e["old_text"].as_str()?.to_string(),
                    e["new_text"].as_str()?.to_string(),
                    e["replace_all"].as_bool().unwrap_or(false),
                ))
            };
            let replacements: Option<Vec<_>> = match args["edits"].as_array() {
                Some(edits) => edits.iter().map(one).collect(),
                None => one(args).map(|r| vec![r]),
            };
            match replacements {
                Some(replacements) => match editor.agent_edit_buffer(path, &replacements) {
                    Ok((text, id)) => Called::Changed(text, id),
                    Err(error) => Called::Done(Err(error)),
                },
                None => Called::Done(Err(
                    "Give old_text and new_text (or edits: a list of them)".into()
                )),
            }
        }
        "create_file" => match (text("path"), args["text"].as_str()) {
            (Some(path), Some(content)) => match editor.agent_create_file(path, content) {
                Ok((text, id)) => Called::Changed(text, id),
                Err(error) => Called::Done(Err(error)),
            },
            _ => Called::Done(Err("Give path and text".into())),
        },
        "move_file" => match (text("from"), text("to")) {
            (Some(from), Some(to)) => match editor.agent_move_file(call, from, to) {
                Ok(Some((text, changed))) => Called::Changed(text, changed),
                Ok(None) => Called::Waiting,
                Err(error) => Called::Done(Err(error)),
            },
            _ => Called::Done(Err("Give from and to".into())),
        },
        "show_locations" => match args["locations"].as_array() {
            Some(places) => Called::Done(
                editor.agent_show_locations(text("title").unwrap_or("Locations"), places),
            ),
            None => Called::Done(Err("Give locations: a list of {path, line, note}".into())),
        },
        "close_file" => match text("path") {
            Some(path) => Called::Done(editor.agent_close_file(path)),
            None => Called::Done(Err("Which file? (path)".into())),
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
            (Some(path), Some(list), Some(n)) => {
                match editor.agent_apply_code_action(call, path, list, n) {
                    Ok(()) => Called::Waiting,
                    Err(error) => Called::Done(Err(error)),
                }
            }
            _ => Called::Done(Err("Give path, list and number".into())),
        },
        "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_code_actions" => {
            // A kind that isn't one of theirs: said, not taken as the default.
            let kinds: &[&str] = match name {
                "lsp_definition" => &["definition", "type", "implementation", "declaration"],
                "lsp_references" => &["references", "callers", "callees"],
                _ => &[],
            };
            if let Some(kind) = text("kind")
                && !kinds.is_empty()
                && !kinds.contains(&kind)
            {
                return Called::Done(Err(format!(
                    "kind is one of {}, not \"{kind}\"",
                    kinds.join(", ")
                )));
            }
            let feature = match (name, text("kind")) {
                ("lsp_definition", Some("type")) => LspFeature::TypeDefinition,
                ("lsp_definition", Some("implementation")) => LspFeature::Implementation,
                ("lsp_definition", Some("declaration")) => LspFeature::Declaration,
                ("lsp_definition", _) => LspFeature::Definition,
                ("lsp_references", Some("callers")) => LspFeature::Callers,
                ("lsp_references", Some("callees")) => LspFeature::Callees,
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
            (None, None) => {
                Called::Done(Err("Give a file (path) or a name to find (query)".into()))
            }
        },
        other => Called::Done(Err(format!("No tool named {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(called: Called) -> String {
        match called {
            Called::Done(Ok(text)) | Called::Changed(text, _) => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tool_sees_what_the_shell_wrote_since() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let file = path.to_str().unwrap();
        let mut editor = Editor::default();
        // Opened (hidden) by an edit of the agent's, saved.
        let args = json!({"path": file, "old_text": "two", "new_text": "TWO"});
        done(call(&mut editor, 1, "edit_buffer", &args));
        // The agent's shell rewrites it: the next tool call sees that, with
        // no idle check in between.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "from the shell\n").unwrap();
        let read = done(call(&mut editor, 2, "read_buffer", &json!({"path": file})));
        assert!(read.contains("from the shell"), "{read}");
        let changes = done(call(&mut editor, 3, "buffer_changes", &json!({})));
        assert_eq!(changes, "No open file has unsaved changes");
    }
}
