//! The tools ferra gives the coding agent (over MCP, `mcp.rs`): what's
//! open and selected, the diagnostics, the language servers' answers about
//! a symbol, and showing the user a file. Kept few and short, as each
//! one's description goes in the agent's prompt.
//!
//! Each tool is one entry in [`TOOLS`]: its name, description, argument
//! schema and handler together, so what's listed and what runs can't
//! drift apart. Handlers read their arguments through [`Args`], whose
//! checks give the agent the same words for the same mistake.

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

/// One tool: what MCP lists, and what it does.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// Its arguments' JSON schema.
    schema: fn() -> Value,
    /// Runs it, as call `call`. An `Err` is the answer, an error.
    run: fn(&mut Editor, u64, &Args) -> Result<Called, String>,
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

/// A tool call's arguments.
pub struct Args<'a>(&'a Value);

impl Args<'_> {
    /// A string argument, if given and not empty.
    fn text(&self, key: &str) -> Option<&str> {
        self.0[key].as_str().filter(|s| !s.is_empty())
    }

    /// A string argument, empty or not.
    fn string(&self, key: &str) -> Option<&str> {
        self.0[key].as_str()
    }

    fn number(&self, key: &str) -> Option<usize> {
        self.0[key].as_u64().map(|n| n as usize)
    }

    /// The file it's about.
    fn path(&self) -> Result<&str, String> {
        self.text("path").ok_or_else(|| "Which file? (path)".into())
    }

    /// The place in the file: line, symbol, column, occurrence.
    fn place(&self) -> (Option<usize>, Option<&str>, Option<usize>, Option<usize>) {
        (
            self.number("line"),
            self.text("symbol"),
            self.number("column"),
            self.number("occurrence"),
        )
    }

    /// Which of `kinds` was asked for (the first, if none was).
    fn kind(&self, kinds: &[(&str, LspFeature)]) -> Result<LspFeature, String> {
        let Some(kind) = self.text("kind") else {
            return Ok(kinds[0].1);
        };
        kinds
            .iter()
            .find(|(name, _)| *name == kind)
            .map(|&(_, feature)| feature)
            .ok_or_else(|| {
                let names: Vec<_> = kinds.iter().map(|(name, _)| *name).collect();
                format!("kind is one of {}, not \"{kind}\"", names.join(", "))
            })
    }

    fn get(&self, key: &str) -> &Value {
        &self.0[key]
    }
}

/// `lsp_definition`'s kinds: the first is the default.
const DEFINITION_KINDS: &[(&str, LspFeature)] = &[
    ("definition", LspFeature::Definition),
    ("type", LspFeature::TypeDefinition),
    ("implementation", LspFeature::Implementation),
    ("declaration", LspFeature::Declaration),
];

/// `lsp_references`'s kinds: every use, or the callers or callees of a
/// function.
const REFERENCE_KINDS: &[(&str, LspFeature)] = &[
    ("references", LspFeature::References),
    ("callers", LspFeature::Callers),
    ("callees", LspFeature::Callees),
];

/// Asks the language server; its answer comes later.
fn ask(
    editor: &mut Editor,
    call: u64,
    args: &Args,
    feature: LspFeature,
    path: Option<&str>,
    query: Option<&str>,
) -> Result<Called, String> {
    editor.agent_ask_lsp(call, feature, path, args.place(), query)?;
    Ok(Called::Waiting)
}

/// A file's text was changed: the answer waits for its problems.
fn changed((text, documents): (String, Vec<crate::editor::DocumentId>)) -> Called {
    Called::Changed(text, documents)
}

static TOOLS: &[Tool] = &[
    Tool {
        name: "editor_state",
        description: "What the user has open in the editor now: the current file and cursor, the selection (its text), and the open files (unsaved ones marked). Use it when they say 'this', 'here' or 'the selection'.",
        schema: nothing,
        run: |editor, _, _| Ok(Called::Done(Ok(editor.agent_editor_state()))),
    },
    Tool {
        name: "buffer_changes",
        description: "The user's changes as a unified diff: a file as the editor has it (unsaved edits included) against the saved file (against: saved, the default) or against git's staged text (against: git, what the gutter shows). With no path, every open file with such changes. Use it for 'review what I wrote' or 'finish what I started'; git diff from the shell can't see unsaved edits.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "One file (every open one with changes, if left out)" },
                    "against": { "type": "string", "enum": ["saved", "git"], "description": "saved if left out" },
                },
            })
        },
        run: |editor, _, args| {
            Ok(Called::Done(editor.agent_buffer_changes(
                args.text("path"),
                args.text("against"),
            )))
        },
    },
    Tool {
        name: "language_servers",
        description: "The language servers: each one's state (starting, running, busy indexing, stopped and why), how many files it has open, those not installed, and what they've logged lately. Use it when the lsp_ tools answer nothing or fail, to tell a server that's missing, crashed or still indexing from one with no answer.",
        schema: nothing,
        run: |editor, _, _| Ok(Called::Done(Ok(editor.agent_language_servers()))),
    },
    Tool {
        name: "read_terminal",
        description: "The text in the editor's terminal panel (the user's shell, the one shown if they have more than one; scrollback included) and whether a command is still running there. Use it when the user mentions output they just saw (a failing build or test, an error, a server's log) instead of running the command again.",
        schema: || {
            json!({
                "type": "object",
                "properties": { "lines": { "type": "integer", "description": "How many of the last lines (200 if left out)" } },
            })
        },
        run: |editor, _, args| {
            Ok(Called::Done(
                editor.agent_read_terminal(args.number("lines")),
            ))
        },
    },
    Tool {
        name: "get_diagnostics",
        description: "Errors and warnings from the language servers (compiler, linter) for a file, or every file reported on. Quicker than building to see problems.",
        schema: || {
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "One file (all, if left out)" } },
            })
        },
        run: |editor, _, args| {
            Ok(Called::Done(
                Ok(editor.agent_diagnostics(args.text("path"))),
            ))
        },
    },
    Tool {
        name: "read_buffer",
        description: "A file's text as the editor has it (unsaved changes included), lines numbered. Use instead of Read.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "line": { "type": "integer", "description": "The first line, from 1" },
                    "limit": { "type": "integer", "description": "How many lines (2000 if left out)" },
                },
                "required": ["path"],
            })
        },
        run: |editor, _, args| {
            let path = args.path()?;
            Ok(Called::Done(editor.agent_read_buffer(
                path,
                args.number("line"),
                args.number("limit"),
            )))
        },
    },
    Tool {
        name: "edit_buffer",
        description: "Replace old_text with new_text in a file, in the editor: shown as it happens, undoable, saved unless the user has unsaved changes there; the result says what's wrong in the file after. old_text must be there exactly once (with its whitespace), unless replace_all. For several changes to a file at once, give edits (a list of old_text/new_text) instead: one undo step, all or none. Use instead of Edit.",
        schema: || {
            json!({
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
            })
        },
        run: |editor, _, args| {
            let path = args.path()?;
            // One replacement, or several (`edits`).
            let one = |e: &Value| -> Option<(String, String, bool)> {
                Some((
                    e["old_text"].as_str()?.to_string(),
                    e["new_text"].as_str()?.to_string(),
                    e["replace_all"].as_bool().unwrap_or(false),
                ))
            };
            let replacements: Option<Vec<_>> = match args.get("edits").as_array() {
                Some(edits) => edits.iter().map(one).collect(),
                None => one(args.0).map(|r| vec![r]),
            };
            let replacements =
                replacements.ok_or("Give old_text and new_text (or edits: a list of them)")?;
            editor.agent_edit_buffer(path, &replacements).map(changed)
        },
    },
    Tool {
        name: "create_file",
        description: "Make a new file with this text (not one that exists), through the editor; the result says what's wrong in it. Use instead of Write for new files.",
        schema: || {
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" }, "text": { "type": "string" } },
                "required": ["path", "text"],
            })
        },
        run: |editor, _, args| match (args.text("path"), args.string("text")) {
            (Some(path), Some(content)) => editor.agent_create_file(path, content).map(changed),
            _ => Err("Give path and text".into()),
        },
    },
    Tool {
        name: "lsp_definition",
        description: "Where a symbol is defined (or its type, its implementations, its declaration: kind), from the language server: path:line:column and the line.",
        schema: || with_kinds(place("to look up"), DEFINITION_KINDS),
        run: |editor, call, args| {
            let feature = args.kind(DEFINITION_KINDS)?;
            ask(editor, call, args, feature, Some(args.path()?), None)
        },
    },
    Tool {
        name: "lsp_references",
        description: "Every use of a symbol in the project, from the language server: path:line:column and the line. With kind callers: the functions that call it (and on which lines); callees: the functions it calls. Those answer 'who calls this?' and 'what does this do?' better than every mention.",
        schema: || with_kinds(place("to find the uses of"), REFERENCE_KINDS),
        run: |editor, call, args| {
            let feature = args.kind(REFERENCE_KINDS)?;
            ask(editor, call, args, feature, Some(args.path()?), None)
        },
    },
    Tool {
        name: "lsp_hover",
        description: "A symbol's type and documentation, from the language server.",
        schema: || place("to describe"),
        run: |editor, call, args| {
            ask(
                editor,
                call,
                args,
                LspFeature::Hover,
                Some(args.path()?),
                None,
            )
        },
    },
    Tool {
        name: "lsp_symbols",
        description: "The functions, types and so on in a file (path), or in the whole project matching a name (query), from the language server, with where each is.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "A file: its symbols" },
                    "query": { "type": "string", "description": "Or: a name to find in the project" },
                },
            })
        },
        run: |editor, call, args| match (args.text("path"), args.text("query")) {
            (Some(path), _) => ask(
                editor,
                call,
                args,
                LspFeature::DocumentSymbols,
                Some(path),
                None,
            ),
            (None, Some(query)) => ask(
                editor,
                call,
                args,
                LspFeature::WorkspaceSymbols,
                None,
                Some(query),
            ),
            (None, None) => Err("Give a file (path) or a name to find (query)".into()),
        },
    },
    Tool {
        name: "lsp_rename",
        description: "Rename a symbol everywhere it's used, with the language server (more reliable than replacing text). Saves the files changed, unless the user has unsaved changes there.",
        schema: || {
            let mut schema = place("to rename");
            schema["properties"]["new_name"] = json!({ "type": "string" });
            schema["required"] = json!(["path", "line", "new_name"]);
            schema
        },
        run: |editor, call, args| match (args.text("path"), args.text("new_name")) {
            (Some(path), Some(name)) => ask(
                editor,
                call,
                args,
                LspFeature::Rename,
                Some(path),
                Some(name),
            ),
            _ => Err("Give path, line, the symbol and new_name".into()),
        },
    },
    Tool {
        name: "lsp_code_actions",
        description: "The language server's fixes and refactors for a place (quick fixes for a diagnostic, imports...), numbered; lsp_apply_code_action makes one. With no line: the file-wide ones (organize imports, fix all, remove unused imports).",
        schema: || {
            // A place, or just a file (its file-wide ones).
            let mut schema = place("they're for");
            schema["required"] = json!(["path"]);
            schema
        },
        run: |editor, call, args| {
            ask(
                editor,
                call,
                args,
                LspFeature::CodeAction,
                Some(args.path()?),
                None,
            )
        },
    },
    Tool {
        name: "lsp_apply_code_action",
        description: "Make one of the code actions lsp_code_actions listed.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file they were for" },
                    "list": { "type": "integer", "description": "The list's number, as lsp_code_actions said" },
                    "number": { "type": "integer", "description": "The action's number in it, from 1" },
                },
                "required": ["path", "list", "number"],
            })
        },
        run: |editor, call, args| match (
            args.text("path"),
            args.get("list").as_u64(),
            args.number("number"),
        ) {
            (Some(path), Some(list), Some(n)) => {
                editor.agent_apply_code_action(call, path, list, n)?;
                Ok(Called::Waiting)
            }
            _ => Err("Give path, list and number".into()),
        },
    },
    Tool {
        name: "lsp_format",
        description: "Format a file with its language server's formatter.",
        schema: path_only,
        run: |editor, call, args| {
            ask(
                editor,
                call,
                args,
                LspFeature::Format,
                Some(args.path()?),
                None,
            )
        },
    },
    Tool {
        name: "move_file",
        description: "Move or rename a file: the language server updates what refers to it (imports, mod paths) first, as edits that can be undone, then the file moves and its open buffer follows. Use instead of mv. Its answer says what changed and the problems after.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "The file" },
                    "to": { "type": "string", "description": "Its new path (folders are made)" },
                },
                "required": ["from", "to"],
            })
        },
        run: |editor, call, args| match (args.text("from"), args.text("to")) {
            (Some(from), Some(to)) => Ok(match editor.agent_move_file(call, from, to)? {
                Some(done) => changed(done),
                None => Called::Waiting,
            }),
            _ => Err("Give from and to".into()),
        },
    },
    Tool {
        name: "show_locations",
        description: "Give the user a list of places to step through, in the editor's panel (a Locations tab, by file; they press ↵ to go to one): the answer to 'where does this need changing?', the callers to update, the places a change touched. Better than paths in your reply when there are several. Each with a short note of what's there.",
        schema: || {
            json!({
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
            })
        },
        run: |editor, _, args| {
            let places = args
                .get("locations")
                .as_array()
                .ok_or("Give locations: a list of {path, line, note}")?;
            let title = args.text("title").unwrap_or("Locations");
            Ok(Called::Done(editor.agent_show_locations(title, places)))
        },
    },
    Tool {
        name: "close_file",
        description: "Close a file's buffer in the editor (one you opened for the user and they're done with). Files you only read or edited close by themselves when your turn ends.",
        schema: path_only,
        run: |editor, _, args| Ok(Called::Done(editor.agent_close_file(args.path()?))),
    },
    Tool {
        name: "open_file",
        description: "Show the user a file in the editor, at a line. Only when it helps them to see it.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "line": { "type": "integer", "description": "From 1" },
                },
                "required": ["path"],
            })
        },
        run: |editor, _, args| {
            let path = args.path()?;
            Ok(Called::Done(
                editor.agent_open_file(path, args.number("line")),
            ))
        },
    },
];

/// The tools, as MCP lists them.
pub fn list() -> Value {
    TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.schema)(),
            })
        })
        .collect()
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
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
        return Called::Done(Err(format!("No tool named {name}")));
    };
    (tool.run)(editor, call, &Args(args)).unwrap_or_else(|error| Called::Done(Err(error)))
}

/// No arguments.
fn nothing() -> Value {
    json!({ "type": "object", "properties": {} })
}

/// Just a file.
fn path_only() -> Value {
    json!({
        "type": "object",
        "properties": { "path": { "type": "string" } },
        "required": ["path"],
    })
}

/// A place in a file: the symbol `what`.
fn place(what: &str) -> Value {
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
}

/// A schema with `kind`: one of `kinds`, the first if left out.
fn with_kinds(mut schema: Value, kinds: &[(&str, LspFeature)]) -> Value {
    let names: Vec<_> = kinds.iter().map(|(name, _)| *name).collect();
    schema["properties"]["kind"] = json!({
        "type": "string",
        "enum": names,
        "description": format!("{} if left out", names[0]),
    });
    schema
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
    fn each_tool_is_listed_once_with_an_object_schema() {
        let listed = list();
        let names: Vec<_> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        for tool in listed.as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        }
    }

    #[test]
    fn kinds_listed_are_the_kinds_taken() {
        for (name, kinds) in [
            ("lsp_definition", DEFINITION_KINDS),
            ("lsp_references", REFERENCE_KINDS),
        ] {
            let tool = TOOLS.iter().find(|t| t.name == name).unwrap();
            let schema = (tool.schema)();
            let listed: Vec<_> = schema["properties"]["kind"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap())
                .collect();
            let taken: Vec<_> = kinds.iter().map(|(k, _)| *k).collect();
            assert_eq!(listed, taken, "{name}");
            for &(kind, feature) in kinds {
                let args = json!({ "kind": kind });
                assert_eq!(Args(&args).kind(kinds), Ok(feature));
            }
            assert_eq!(Args(&json!({})).kind(kinds), Ok(kinds[0].1), "{name}");
        }
        assert_eq!(
            Args(&json!({ "kind": "calls" })).kind(REFERENCE_KINDS),
            Err("kind is one of references, callers, callees, not \"calls\"".into())
        );
    }

    #[test]
    fn a_file_left_out_is_asked_for() {
        let mut editor = Editor::default();
        for name in [
            "read_buffer",
            "edit_buffer",
            "lsp_definition",
            "lsp_references",
            "lsp_hover",
            "lsp_code_actions",
            "lsp_format",
            "close_file",
            "open_file",
        ] {
            assert_eq!(
                call(&mut editor, 1, name, &json!({})),
                Called::Done(Err("Which file? (path)".into())),
                "{name}"
            );
        }
        assert_eq!(
            call(&mut editor, 1, "teleport", &json!({})),
            Called::Done(Err("No tool named teleport".into()))
        );
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
