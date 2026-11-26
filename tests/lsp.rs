//! Language servers, end to end, with a fake server running in this
//! process: it keeps its own copy of each file from what ferra sends, and
//! reports every `bad` as an error, so the copy (and the diagnostics' places)
//! show that syncing works.

mod common;

use std::fs;
use std::io::{self, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{app, app_with_config, keys, render, text};
use ferra::App;
use ferra::config::ServerConfig;
use ferra::editor::DocumentId;
use ferra::lsp::transport::{read_message, write_message};
use ferra::lsp::{Spawner, Streams};
use serde_json::{Value, json};

/// What the fake server saw.
#[derive(Default)]
struct Seen {
    methods: Vec<String>,
    text: String,
    /// The settings sent at start-up, and ferra's answer to the server's
    /// `workspace/configuration` request.
    settings: Value,
    configuration: Value,
    /// The options a formatting request came with, and the range a
    /// range formatting one was for.
    format_options: Value,
    format_range: Value,
    /// The range the last code action request was for, and ferra's
    /// answer to the server's `workspace/applyEdit`.
    action_range: Value,
    /// The kinds the last code action request asked for only.
    action_only: Value,
    /// The files the last `workspace/didRenameFiles` said moved.
    renamed: Value,
    applied: Value,
    /// The context the last signature help request came with, and the
    /// queries for the project's symbols.
    signature_context: Value,
    symbol_queries: Vec<String>,
    /// How many times the server has been started.
    starts: usize,
}

struct FakeServer {
    utf8: bool,
    seen: Arc<Mutex<Seen>>,
}

/// Where the fake server's `gd` on `other` goes: another file.
const OTHER: &str = "other.rs";

impl Spawner for FakeServer {
    fn spawn(&self, _: &ServerConfig, root: &Path) -> io::Result<Streams> {
        let (client_reads, server_writes) = io::pipe()?;
        let (server_reads, client_writes) = io::pipe()?;
        let seen = Arc::clone(&self.seen);
        seen.lock().unwrap().starts += 1;
        let utf8 = self.utf8;
        let dir = root.to_path_buf();
        std::thread::spawn(move || serve(server_reads, server_writes, utf8, &seen, &dir));
        Ok(Streams {
            reader: Box::new(BufReader::new(client_reads)),
            writer: Box::new(client_writes),
            child: None,
        })
    }
}

fn serve(
    input: io::PipeReader,
    mut output: io::PipeWriter,
    utf8: bool,
    seen: &Mutex<Seen>,
    root: &Path,
) {
    let mut input = BufReader::new(input);
    while let Ok(Some(message)) = read_message(&mut input) {
        if message["id"] == 99 && message.get("method").is_none() {
            seen.lock().unwrap().configuration = message["result"].clone();
            continue;
        }
        if message["id"] == 100 && message.get("method").is_none() {
            seen.lock().unwrap().applied = message["result"].clone();
            continue;
        }
        let method = message["method"].as_str().unwrap_or_default().to_string();
        seen.lock().unwrap().methods.push(method.clone());
        let params = &message["params"];
        let reply = |output: &mut io::PipeWriter, result: Value| {
            write_message(
                output,
                &json!({"jsonrpc": "2.0", "id": message["id"], "result": result}),
            )
            .unwrap();
        };
        match method.as_str() {
            "initialize" => {
                let encoding = if utf8 { "utf-8" } else { "utf-16" };
                // The UTF-16 server only syncs: it can't answer hovers.
                let features = utf8;
                // Asked first before renaming, unless the project says not
                // to (a `no-prepare` file).
                let rename = if features && !root.join("no-prepare").exists() {
                    json!({"prepareProvider": true})
                } else {
                    json!(features)
                };
                reply(
                    &mut output,
                    json!({"capabilities": {
                        "positionEncoding": encoding, "textDocumentSync": 2,
                        "hoverProvider": features, "definitionProvider": features,
                        "referencesProvider": features,
                        "completionProvider": {"triggerCharacters": ["."]},
                        "documentFormattingProvider": features,
                        // Unless the project says it formats only whole
                        // files (a `whole-only` file).
                        "documentRangeFormattingProvider":
                            features && !root.join("whole-only").exists(),
                        "renameProvider": rename,
                        "documentSymbolProvider": features,
                        "documentHighlightProvider": features,
                        "inlayHintProvider": features,
                        "workspaceSymbolProvider": features,
                        "callHierarchyProvider": features,
                        "workspace": {"fileOperations": {
                            "willRename": {"filters": [{"pattern": {"glob": "**/*.rs"}}]},
                            "didRename": {"filters": [{"pattern": {"glob": "**/*.rs"}}]},
                        }},
                        "signatureHelpProvider": {"triggerCharacters": ["("],
                                                  "retriggerCharacters": [","]},
                        "codeActionProvider": if features {
                            json!({"resolveProvider": true})
                        } else {
                            json!(false)
                        },
                    }}),
                );
                // Servers ask things too; this must be answered.
                write_message(
                    &mut output,
                    &json!({"jsonrpc": "2.0", "id": 99, "method": "workspace/configuration",
                            "params": {"items": [{"section": "check.on"}, {"section": "nope"}]}}),
                )
                .unwrap();
            }
            "textDocument/didOpen" => {
                let text = params["textDocument"]["text"].as_str().unwrap().to_string();
                seen.lock().unwrap().text = text.clone();
                publish(&mut output, &params["textDocument"]["uri"], seen, utf8);
                // As rust-analyzer does on starting: progress, begun and
                // then reported on.
                if text.contains("indexing") {
                    let progress = |value: Value| {
                        json!({"jsonrpc": "2.0", "method": "$/progress",
                               "params": {"token": "idx", "value": value}})
                    };
                    write_message(
                        &mut output,
                        &json!({"jsonrpc": "2.0", "id": 101,
                        "method": "window/workDoneProgress/create", "params": {"token": "idx"}}),
                    )
                    .unwrap();
                    write_message(
                        &mut output,
                        &progress(json!({"kind": "begin", "title": "Indexing", "percentage": 0})),
                    )
                    .unwrap();
                    write_message(
                        &mut output,
                        &progress(json!({"kind": "report", "message": "3/7", "percentage": 45})),
                    )
                    .unwrap();
                }
                // As rust-analyzer does after `cargo check`: a file that isn't
                // open has a problem too.
                let other = root.join(OTHER);
                if text.contains("project") && other.exists() {
                    let uri = ferra::lsp::convert::path_to_uri(&other.canonicalize().unwrap());
                    let at = |c: u64| json!({"line": 1, "character": c});
                    write_message(
                        &mut output,
                        &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                        "params": {"uri": uri, "diagnostics": [{
                            "range": {"start": at(4), "end": at(9)},
                            "severity": 2, "message": "unused thing", "source": "fake",
                        }]}}),
                    )
                    .unwrap();
                }
            }
            "textDocument/didChange" if seen.lock().unwrap().text.contains("indexing") => {
                for change in params["contentChanges"].as_array().unwrap() {
                    let mut seen = seen.lock().unwrap();
                    seen.text = apply(&seen.text, change, utf8);
                }
                write_message(
                    &mut output,
                    &json!({"jsonrpc": "2.0", "method": "$/progress",
                    "params": {"token": "idx", "value": {"kind": "end"}}}),
                )
                .unwrap();
            }
            "textDocument/didChange" => {
                for change in params["contentChanges"].as_array().unwrap() {
                    let mut seen = seen.lock().unwrap();
                    seen.text = apply(&seen.text, change, utf8);
                }
                publish(&mut output, &params["textDocument"]["uri"], seen, utf8);
            }
            "workspace/didChangeConfiguration" => {
                seen.lock().unwrap().settings = params["settings"].clone();
            }
            "textDocument/hover" => {
                let word = word_at(&seen.lock().unwrap().text, &params["position"]);
                // Asked about "die", it stops before answering.
                if word == "die" {
                    return;
                }
                let result = (word == "bad").then(|| {
                    json!({"contents": {
                        "kind": "markdown",
                        "value": "```rust\nfn bad()\n```\n\n---\n\nA **bad** word, see `fine`.",
                    }})
                });
                reply(&mut output, result.into());
            }
            "textDocument/definition" | "textDocument/references" => {
                let text = seen.lock().unwrap().text.clone();
                let uri = &params["textDocument"]["uri"];
                let word = word_at(&text, &params["position"]);
                let at = |line: usize, col: usize, uri: &Value| {
                    let start = json!({"line": line, "character": col});
                    json!({"uri": uri, "range": {"start": start, "end": start}})
                };
                let mut places: Vec<Value> = Vec::new();
                for (line, line_text) in text.split('\n').enumerate() {
                    for (col, _) in line_text.match_indices(&word) {
                        places.push(at(line, col, uri));
                    }
                }
                let result = if word == "other" {
                    let path = root.join(OTHER).canonicalize().unwrap();
                    at(1, 2, &json!(ferra::lsp::convert::path_to_uri(&path)))
                } else if method == "textDocument/definition" {
                    places.first().cloned().unwrap_or_default()
                } else {
                    Value::Array(places)
                };
                reply(&mut output, result);
            }
            "textDocument/completion" if seen.lock().unwrap().text.contains("maybe") => {
                // Items that go in from before the word: in place of the
                // `.` typed (as the TypeScript server and clangd do).
                let p = &params["position"];
                let (line, col) = (
                    p["line"].as_u64().unwrap(),
                    p["character"].as_u64().unwrap(),
                );
                let text = seen.lock().unwrap().text.clone();
                let line_text = text.split('\n').nth(line as usize).unwrap_or("");
                let word = line_text[..col as usize]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric())
                    .count() as u64;
                let dot = json!({"start": {"line": line, "character": col - word - 1},
                                 "end": {"line": line, "character": col}});
                reply(
                    &mut output,
                    json!([
                        {"label": "thing", "filterText": ".?.thing",
                         "textEdit": {"range": dot, "newText": "?.thing"}},
                        {"label": "field", "filterText": "field",
                         "textEdit": {"range": dot, "newText": "->field"}},
                        {"label": "call(arg)", "filterText": "call",
                         "insertText": "call(${1:arg})$0", "insertTextFormat": 2},
                    ]),
                );
            }
            "textDocument/completion" => {
                let import = json!({"range": {"start": {"line": 0, "character": 0},
                                              "end": {"line": 0, "character": 0}},
                                    "newText": "use imported;\n"});
                reply(
                    &mut output,
                    json!({"isIncomplete": false, "items": [
                        {"label": "bad_word", "kind": 6, "detail": "i32"},
                        {"label": "badge()", "insertText": "badge", "kind": 3},
                        {"label": "basket", "insertText": "basket(${1:n})$0",
                         "insertTextFormat": 2},
                        {"label": "fine", "kind": 14},
                        {"label": "imported", "additionalTextEdits": [import]},
                        {"label": "resolved", "data": {"id": 7}},
                    ]}),
                );
            }
            "textDocument/codeAction" => {
                seen.lock().unwrap().action_range = params["range"].clone();
                seen.lock().unwrap().action_only = params["context"]["only"].clone();
                let uri = &params["textDocument"]["uri"];
                // Asked for the file-wide (source) ones only.
                if params["context"]["only"] == json!(["source"]) {
                    reply(
                        &mut output,
                        json!([{"title": "Organize imports", "kind": "source.organizeImports",
                                "edit": {"changes": {}}},
                               {"title": "Replace with good", "kind": "quickfix"}]),
                    );
                    continue;
                }
                // A quick fix for each diagnostic sent back with the request.
                let mut actions: Vec<Value> = params["context"]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| {
                        json!({"title": "Replace with good", "kind": "quickfix",
                               "edit": {"changes": {uri.as_str().unwrap():
                                   [{"range": d["range"], "newText": "good"}]}}})
                    })
                    .collect();
                let line = params["range"]["start"]["line"].clone();
                actions.push(json!({"title": "Upper-case the line", "kind": "refactor",
                                    "data": {"uri": uri, "line": line}}));
                actions.push(json!({"title": "Add a note", "command": "fake.note",
                                    "arguments": [uri]}));
                actions.push(json!({"title": "Can't do this", "disabled": {"reason": "no"}}));
                reply(&mut output, Value::Array(actions));
            }
            // Moving a file: main.rs (beside it) has its "import" updated.
            "workspace/willRenameFiles" => {
                let old = params["files"][0]["oldUri"].as_str().unwrap();
                let new = params["files"][0]["newUri"].as_str().unwrap();
                if new.contains("noedit") {
                    reply(&mut output, Value::Null);
                    continue;
                }
                let main = format!("{}/main.rs", &old[..old.rfind('/').unwrap()]);
                let name = |uri: &str| {
                    uri.rsplit('/')
                        .next()
                        .unwrap()
                        .trim_end_matches(".rs")
                        .to_string()
                };
                let note = format!(
                    "// uses {} at {}\n",
                    name(old),
                    &new[new.find("sub").unwrap_or(0)..]
                );
                let edit = json!([{"range": {"start": {"line": 0, "character": 0},
                                             "end": {"line": 0, "character": 0}},
                                   "newText": note}]);
                let mut changes = json!({ main.clone(): edit });
                // uses.rs too, if there's one beside it (no one has it open).
                let uses = main.replace("main.rs", "uses.rs");
                if std::path::Path::new(uses.trim_start_matches("file://")).exists() {
                    changes[uses] = edit.clone();
                }
                reply(&mut output, json!({ "changes": changes }));
            }
            "workspace/didRenameFiles" => {
                seen.lock().unwrap().renamed = params["files"].clone();
            }
            // The function `bad` (line 1), called from `main` twice and
            // calling `helper`.
            "textDocument/prepareCallHierarchy" => {
                let uri = params["textDocument"]["uri"].clone();
                let line = params["position"]["line"].as_u64().unwrap();
                let answer = if line == 0 {
                    json!([{"name": "bad", "kind": 12, "uri": uri,
                            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 11}},
                            "selectionRange": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}}}])
                } else {
                    Value::Null
                };
                reply(&mut output, answer);
            }
            "callHierarchy/incomingCalls" => {
                let uri = params["item"]["uri"].clone();
                let range = |l: u64| json!({"start": {"line": l, "character": 4}, "end": {"line": l, "character": 7}});
                reply(
                    &mut output,
                    json!([{"from": {"name": "main", "kind": 12, "uri": uri,
                                     "range": range(4), "selectionRange": range(4)},
                            "fromRanges": [range(6), range(5)]}]),
                );
            }
            "callHierarchy/outgoingCalls" => {
                let uri = params["item"]["uri"].clone();
                let range = |l: u64| json!({"start": {"line": l, "character": 4}, "end": {"line": l, "character": 7}});
                reply(
                    &mut output,
                    json!([{"to": {"name": "helper", "kind": 12, "uri": uri,
                                   "range": range(9), "selectionRange": range(9)},
                            "fromRanges": [range(0)]},
                           {"to": {"name": "log", "kind": 6, "uri": uri, "detail": "impl Logger for Fake",
                                   "range": range(12), "selectionRange": range(12)},
                            "fromRanges": [range(0)]},
                           // The standard library's, and an enum member.
                           {"to": {"name": "push", "kind": 6, "uri": "file:///rustlib/alloc/src/vec.rs",
                                   "range": range(3), "selectionRange": range(3)},
                            "fromRanges": [range(0)]},
                           {"to": {"name": "Ok", "kind": 22, "uri": "file:///rustlib/core/src/result.rs",
                                   "range": range(3), "selectionRange": range(3)},
                            "fromRanges": [range(0)]}]),
                );
            }
            "codeAction/resolve" => {
                let text = seen.lock().unwrap().text.clone();
                let line = params["data"]["line"].as_u64().unwrap() as usize;
                let old = text.split('\n').nth(line).unwrap_or("");
                let mut action = params.clone();
                action["edit"] = json!({"changes": {params["data"]["uri"].as_str().unwrap(): [{
                    "range": {"start": {"line": line, "character": 0},
                              "end": {"line": line, "character": old.len()}},
                    "newText": old.to_uppercase(),
                }]}});
                reply(&mut output, action);
            }
            // The edit sent while it runs, then the command's answer, as
            // servers do.
            "workspace/executeCommand" => {
                let uri = params["arguments"][0].clone();
                write_message(
                    &mut output,
                    &json!({"jsonrpc": "2.0", "id": 100, "method": "workspace/applyEdit",
                    "params": {"label": "Note added", "edit": {"changes": {
                        uri.as_str().unwrap(): [{
                            "range": {"start": {"line": 0, "character": 0},
                                      "end": {"line": 0, "character": 0}},
                            "newText": "// note\n",
                        }]}}}}),
                )
                .unwrap();
                reply(&mut output, Value::Null);
            }
            "textDocument/signatureHelp" => {
                seen.lock().unwrap().signature_context = params["context"].clone();
                let text = seen.lock().unwrap().text.clone();
                let p = &params["position"];
                let line = text
                    .split('\n')
                    .nth(p["line"].as_u64().unwrap() as usize)
                    .unwrap_or("");
                let before = &line[..(p["character"].as_u64().unwrap() as usize).min(line.len())];
                // In `call(` unless it's closed: the commas since say which
                // parameter.
                let open = before
                    .rfind("call(")
                    .filter(|&at| !before[at..].contains(')'));
                // `over(` has two, the first always active (as a server
                // that pays no heed to `activeSignatureHelp`).
                let over = before
                    .rfind("over(")
                    .filter(|&at| !before[at..].contains(')'));
                let result = over.map(|at| {
                    let commas = before[at..].matches(',').count();
                    json!({"signatures": [
                        {"label": "over(a: u8)", "parameters": [{"label": "a: u8"}]},
                        {"label": "over(a: u8, b: u8)",
                         "parameters": [{"label": "a: u8"}, {"label": "b: u8"}]},
                    ], "activeSignature": 0, "activeParameter": commas})
                });
                let result = result.or_else(|| {
                    open.map(|at| {
                        let commas = before[at..].matches(',').count();
                        json!({"signatures": [{
                        "label": "call(first: u8, second: u8)",
                        "documentation": "Calls it.",
                        "parameters": [
                            {"label": "first: u8", "documentation": "The first."},
                            {"label": [16, 26]},
                        ],
                    }], "activeSignature": 0, "activeParameter": commas})
                    })
                });
                reply(&mut output, result.into());
            }
            // `: i32` after each `let name`.
            "textDocument/inlayHint" => {
                let text = seen.lock().unwrap().text.clone();
                let mut hints = Vec::new();
                for (line, line_text) in text.split('\n').enumerate() {
                    if let Some(rest) = line_text.strip_prefix("let ") {
                        let name: String =
                            rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                        let at = json!({"line": line, "character": 4 + name.len()});
                        hints.push(json!({"position": at, "label": ": i32", "kind": 1}));
                    }
                }
                reply(&mut output, Value::Array(hints));
            }
            "textDocument/documentHighlight" => {
                let text = seen.lock().unwrap().text.clone();
                let word = word_at(&text, &params["position"]);
                let mut found = Vec::new();
                for (line, line_text) in text.split('\n').enumerate() {
                    for (col, _) in line_text
                        .match_indices(word.as_str())
                        .filter(|_| !word.is_empty())
                    {
                        let at = |c: usize| json!({"line": line, "character": c});
                        found.push(json!({"range": {"start": at(col), "end": at(col + word.len())}, "kind": 2}));
                    }
                }
                reply(&mut output, Value::Array(found));
            }
            "textDocument/documentSymbol" => {
                let text = seen.lock().unwrap().text.clone();
                let symbols = text_symbols(&text);
                let result = if text.contains("flat") {
                    // The older, flat form: a location and a container each.
                    let uri = &params["textDocument"]["uri"];
                    symbols
                        .iter()
                        .map(|(name, kind, at, parent)| {
                            json!({"name": name, "kind": kind,
                                   "location": {"uri": uri, "range": {"start": at, "end": at}},
                                   "containerName": parent})
                        })
                        .collect()
                } else {
                    let mut top: Vec<Value> = Vec::new();
                    for (name, kind, at, parent) in symbols {
                        let symbol = json!({"name": name, "kind": kind, "children": [],
                                            "range": {"start": at, "end": at},
                                            "selectionRange": {"start": at, "end": at}});
                        match (parent, top.last_mut()) {
                            (Some(_), Some(last)) => {
                                last["children"].as_array_mut().unwrap().push(symbol)
                            }
                            _ => top.push(symbol),
                        }
                    }
                    top
                };
                reply(&mut output, Value::Array(result));
            }
            "workspace/symbol" => {
                let query = params["query"].as_str().unwrap().to_string();
                seen.lock().unwrap().symbol_queries.push(query.clone());
                let mut found = Vec::new();
                let main = root.join("main.rs").canonicalize().unwrap();
                let other = root.join(OTHER);
                let files = [
                    (main, Some(seen.lock().unwrap().text.clone())),
                    (other.clone(), fs::read_to_string(&other).ok()),
                ];
                for (path, text) in files {
                    let Some(text) = text else { continue };
                    let uri = ferra::lsp::convert::path_to_uri(&path.canonicalize().unwrap());
                    for (name, kind, at, parent) in text_symbols(&text) {
                        if !query.is_empty() && name.contains(&query) {
                            found.push(json!({"name": name, "kind": kind, "containerName": parent,
                                              "location": {"uri": uri, "range": {"start": at, "end": at}}}));
                        }
                    }
                }
                reply(&mut output, Value::Array(found));
            }
            // `let`: nothing to rename; `named`: a name of its own; after
            // a `'` (a lifetime), the range takes it in; else the word's.
            "textDocument/prepareRename" => {
                let text = seen.lock().unwrap().text.clone();
                let position = &params["position"];
                let word = word_at(&text, position);
                let row = position["line"].as_u64().unwrap() as usize;
                let line = text.split('\n').nth(row).unwrap_or("");
                let at = position["character"].as_u64().unwrap() as usize;
                let is_word = |c: char| c.is_alphanumeric() || c == '_';
                let mut start = line[..at].rfind(|c| !is_word(c)).map_or(0, |i| i + 1);
                if line[..start].ends_with('\'') {
                    start -= 1;
                }
                let end = line[at..]
                    .find(|c| !is_word(c))
                    .map_or(line.len(), |i| at + i);
                let range = json!({"start": {"line": row, "character": start},
                                   "end": {"line": row, "character": end}});
                let result = match word.as_str() {
                    "let" => Value::Null,
                    "named" => json!({"range": range, "placeholder": "the_name"}),
                    _ => range,
                };
                reply(&mut output, result);
            }
            "textDocument/rename" => {
                let text = seen.lock().unwrap().text.clone();
                let word = word_at(&text, &params["position"]);
                if word == "fine" {
                    write_message(
                        &mut output,
                        &json!({"jsonrpc": "2.0", "id": message["id"],
                                "error": {"code": -32602, "message": "can't rename that"}}),
                    )
                    .unwrap();
                    continue;
                }
                let new_name = params["newName"].as_str().unwrap();
                let renames = |text: &str| -> Vec<Value> {
                    let mut edits = Vec::new();
                    for (line, line_text) in text.split('\n').enumerate() {
                        for (col, _) in line_text.match_indices(&word) {
                            let at = |c: usize| json!({"line": line, "character": c});
                            edits.push(
                                json!({"range": {"start": at(col), "end": at(col + word.len())},
                                              "newText": new_name}),
                            );
                        }
                    }
                    edits
                };
                let mut changes = vec![json!({
                    "textDocument": {"uri": params["textDocument"]["uri"], "version": null},
                    "edits": renames(&text),
                })];
                let other = root.join(OTHER);
                if let Ok(other_text) = fs::read_to_string(&other) {
                    let uri = ferra::lsp::convert::path_to_uri(&other.canonicalize().unwrap());
                    changes.push(json!({"textDocument": {"uri": uri, "version": null},
                                        "edits": renames(&other_text)}));
                }
                reply(&mut output, json!({"documentChanges": changes}));
            }
            "textDocument/formatting" => {
                let text = seen.lock().unwrap().text.clone();
                seen.lock().unwrap().format_options = params["options"].clone();
                if text.contains("slow") {
                    continue; // Never answers.
                }
                reply(&mut output, Value::Array(format_edits(&text)));
            }
            "textDocument/rangeFormatting" => {
                let text = seen.lock().unwrap().text.clone();
                seen.lock().unwrap().format_range = params["range"].clone();
                let (first, last) = (
                    params["range"]["start"]["line"].as_u64().unwrap(),
                    params["range"]["end"]["line"].as_u64().unwrap(),
                );
                let edits = format_edits(&text)
                    .into_iter()
                    .filter(|e| {
                        (first..=last).contains(&e["range"]["start"]["line"].as_u64().unwrap())
                    })
                    .collect();
                reply(&mut output, Value::Array(edits));
            }
            "completionItem/resolve" => {
                let mut item = params.clone();
                item["additionalTextEdits"] = json!([{
                    "range": {"start": {"line": 0, "character": 0},
                              "end": {"line": 0, "character": 0}},
                    "newText": "use resolved;\n",
                }]);
                reply(&mut output, item);
            }
            "shutdown" => reply(&mut output, Value::Null),
            "exit" => return,
            _ => {}
        }
    }
}

/// The text with `change` (a range and its new text, or the whole text)
/// applied, counting columns as the server does.
fn apply(text: &str, change: &Value, utf8: bool) -> String {
    let Some(range) = change.get("range") else {
        return change["text"].as_str().unwrap().into();
    };
    let offset = |p: &Value| {
        let (line, character) = (
            p["line"].as_u64().unwrap(),
            p["character"].as_u64().unwrap(),
        );
        let start: usize = text
            .split('\n')
            .take(line as usize)
            .map(|l| l.len() + 1)
            .sum();
        let line_text = text.split('\n').nth(line as usize).unwrap_or("");
        let mut units = 0;
        let mut bytes = 0;
        for c in line_text.chars() {
            if units >= character as usize {
                break;
            }
            units += if utf8 { c.len_utf8() } else { c.len_utf16() };
            bytes += c.len_utf8();
        }
        start + bytes
    };
    let (from, to) = (offset(&range["start"]), offset(&range["end"]));
    format!(
        "{}{}{}",
        &text[..from],
        change["text"].as_str().unwrap(),
        &text[to..]
    )
}

/// How the fake server formats: runs of spaces after a line's indentation
/// become one, and trailing spaces go; an edit for each line that changes.
fn format_edits(text: &str) -> Vec<Value> {
    let mut edits = Vec::new();
    for (line, old) in text.split('\n').enumerate() {
        let indent = old.len() - old.trim_start().len();
        let words: Vec<&str> = old.split(' ').filter(|w| !w.is_empty()).collect();
        let new = format!("{}{}", &old[..indent], words.join(" "));
        if new != old {
            let end = old.chars().count();
            edits.push(json!({
                "range": {"start": {"line": line, "character": 0},
                          "end": {"line": line, "character": end}},
                "newText": new,
            }));
        }
    }
    edits
}

/// The symbols the fake server finds: `struct X` lines, the `fn`s indented
/// under them (their methods), and other `fn`s: name, kind, where the name
/// is, and what it's in.
fn text_symbols(text: &str) -> Vec<(String, u64, Value, Option<String>)> {
    let mut symbols = Vec::new();
    let mut current: Option<String> = None;
    for (line, line_text) in text.split('\n').enumerate() {
        let trimmed = line_text.trim_start();
        let indented = trimmed.len() < line_text.len();
        let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("struct ") {
            (23, rest)
        } else if let Some(rest) = trimmed.strip_prefix("fn ") {
            (if indented { 6 } else { 12 }, rest)
        } else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let col = line_text.find(&name).unwrap();
        let at = json!({"line": line, "character": col});
        let parent = if kind == 6 { current.clone() } else { None };
        if kind == 23 {
            current = Some(name.clone());
        } else if !indented {
            current = None;
        }
        symbols.push((name, kind, at, parent));
    }
    symbols
}

/// The word at an LSP position (in bytes) in `text`.
fn word_at(text: &str, position: &Value) -> String {
    let line = text
        .split('\n')
        .nth(position["line"].as_u64().unwrap() as usize)
        .unwrap_or("");
    let at = (position["character"].as_u64().unwrap() as usize).min(line.len());
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = line[..at].rfind(|c| !is_word(c)).map_or(0, |i| i + 1);
    let end = line[at..]
        .find(|c| !is_word(c))
        .map_or(line.len(), |i| at + i);
    line[start..end].to_string()
}

/// Reports each `bad` in the server's copy as an error.
fn publish(output: &mut io::PipeWriter, uri: &Value, seen: &Mutex<Seen>, utf8: bool) {
    let text = seen.lock().unwrap().text.clone();
    let mut diagnostics = Vec::new();
    for (line, line_text) in text.split('\n').enumerate() {
        for (byte, _) in line_text.match_indices("bad") {
            let prefix = &line_text[..byte];
            let col = if utf8 {
                prefix.len()
            } else {
                prefix.encode_utf16().count()
            };
            diagnostics.push(json!({
                "range": {"start": {"line": line, "character": col},
                          "end": {"line": line, "character": col + 3}},
                "severity": 1, "message": "bad word", "source": "fake",
            }));
        }
    }
    // A file saying `oncheck` also gets each `wrong` in the file as saved,
    // said to be from rustc, at its place there (as rust-analyzer passes on
    // cargo check's results whatever the buffer's become).
    if text.contains("oncheck")
        && let Some(path) = uri.as_str().and_then(ferra::lsp::convert::uri_to_path)
    {
        let saved = fs::read_to_string(path).unwrap_or_default();
        for (line, line_text) in saved.split('\n').enumerate() {
            if let Some(col) = line_text.find("wrong") {
                diagnostics.push(json!({
                    "range": {"start": {"line": line, "character": col},
                              "end": {"line": line, "character": col + 5}},
                    "severity": 1, "message": "wrong word", "source": "rustc",
                }));
            }
        }
    }
    let mut params = json!({"uri": uri, "diagnostics": diagnostics});
    // A file saying `stale` gets a list said to be for its first version
    // (as a slow server's can be), with a diagnostic at its start.
    if text.contains("stale") {
        params["version"] = json!(1);
        params["diagnostics"] = json!([{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "severity": 1, "message": "old", "source": "fake",
        }]);
    }
    write_message(
        output,
        &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                "params": params}),
    )
    .unwrap();
}

/// An app with the fake server, editing `content` in a Rust file.
fn with_server(content: &str, utf8: bool) -> (tempfile::TempDir, App, Arc<Mutex<Seen>>) {
    with_server_in(app(), content, utf8)
}

fn with_server_in(
    app: App,
    content: &str,
    utf8: bool,
) -> (tempfile::TempDir, App, Arc<Mutex<Seen>>) {
    with_server_at(tempfile::tempdir().unwrap(), app, content, utf8)
}

/// The same, in `dir` (with files of its own made first).
fn with_server_at(
    dir: tempfile::TempDir,
    mut app: App,
    content: &str,
    utf8: bool,
) -> (tempfile::TempDir, App, Arc<Mutex<Seen>>) {
    let path = dir.path().join("main.rs");
    fs::write(&path, content).unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    app.lsp_mut().set_spawner(Box::new(FakeServer {
        utf8,
        seen: Arc::clone(&seen),
    }));
    app.open(&path);
    (dir, app, seen)
}

/// Lets the servers' messages through until `done`, or a second passes.
fn until(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        app.update();
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out");
}

fn doc(app: &App) -> DocumentId {
    app.editor().document.id()
}

fn diagnostic_starts(app: &App) -> Vec<(usize, usize)> {
    app.editor()
        .diagnostics(doc(app))
        .iter()
        .map(|d| (d.start.row, d.start.col))
        .collect()
}

#[test]
fn diagnostics_show_in_the_text_and_the_status_line() {
    let (_dir, mut app, _) = with_server("let bad = 1;\nfine\n", true);
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    assert_eq!(diagnostic_starts(&app), [(0, 4)]);
    let screen = render(&mut app, 60, 6);
    assert!(
        screen.rows[0].contains("let bad = 1;  ● bad word"),
        "{:?}",
        screen.rows
    );
    assert!(
        screen.rows.iter().any(|r| r.contains(" E1 ")),
        "{:?}",
        screen.rows
    );
}

#[test]
fn edits_reach_the_server_one_at_a_time() {
    let (_dir, mut app, seen) = with_server("fine\n", true);
    until(&mut app, |a| {
        a.editor()
            .lsp_status()
            .iter()
            .any(|l| l.contains("running"))
    });
    for step in ["Abad<Esc>", "0ié <Esc>", "o<CR>a bad<Esc>", "kdd", "u", "x"] {
        keys(&mut app, step);
        until(&mut app, |a| seen.lock().unwrap().text == text(a));
    }
    // "é finebad" and "a bad", as the server's copy says too. (Waiting for
    // these places, not just two diagnostics: the list for the text before
    // the last edit can still be the latest one in.)
    assert_eq!(text(&app), "é finebad\n\na bad");
    until(&mut app, |a| diagnostic_starts(a) == [(0, 6), (2, 2)]);
}

#[test]
fn servers_counting_utf16_get_whole_texts_and_columns_convert() {
    let (_dir, mut app, seen) = with_server("é😀 bad\n", false);
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    assert_eq!(diagnostic_starts(&app), [(0, 3)], "chars, not UTF-16 units");
    keys(&mut app, "0ix<Esc>");
    until(&mut app, |a| seen.lock().unwrap().text == text(a));
    until(&mut app, |a| diagnostic_starts(a) == [(0, 4)]);
}

#[test]
fn bracket_d_goes_to_diagnostics() {
    let (_dir, mut app, _) = with_server("bad\nok\nbad again\n", true);
    until(&mut app, |a| a.editor().diagnostics(doc(a)).len() == 2);
    keys(&mut app, "]d");
    assert_eq!(common::cursor(&app), (2, 0));
    let message = app.editor().message().unwrap().text.clone();
    assert_eq!(message, "error: bad word (fake)");
    keys(&mut app, "]d");
    assert_eq!(common::cursor(&app), (0, 0), "wrapping round");
}

#[test]
fn closing_a_buffer_and_quitting_tell_the_server() {
    let (dir, mut app, seen) = with_server("x\n", true);
    until(&mut app, |_| {
        seen.lock()
            .unwrap()
            .methods
            .contains(&"textDocument/didOpen".into())
    });
    app.open(&dir.path().join("other.txt"));
    keys(&mut app, ":b main<CR>:bd<CR>");
    until(&mut app, |_| {
        seen.lock()
            .unwrap()
            .methods
            .contains(&"textDocument/didClose".into())
    });
    app.lsp_mut().shutdown();
    let methods = seen.lock().unwrap().methods.clone();
    assert!(
        methods.ends_with(&["shutdown".into(), "exit".into()]),
        "{methods:?}"
    );
}

/// A spawner whose server isn't installed.
struct Missing;

impl Spawner for Missing {
    fn spawn(&self, _: &ServerConfig, _: &Path) -> io::Result<Streams> {
        Err(io::Error::new(io::ErrorKind::NotFound, "no such program"))
    }
}

#[test]
fn a_server_that_isnt_installed_is_reported_once() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "x\n").unwrap();
    fs::write(dir.path().join("b.rs"), "y\n").unwrap();
    let mut app = app();
    app.lsp_mut().set_spawner(Box::new(Missing));
    app.open(&dir.path().join("a.rs"));
    app.update();
    let message = app.editor().message().unwrap().text.clone();
    assert_eq!(
        message,
        "rust-analyzer not found: install it for rust language features (see :lsp)"
    );
    keys(&mut app, ":echo<CR>");
    app.open(&dir.path().join("b.rs"));
    app.update();
    assert!(
        !app.editor().message().unwrap().text.contains("not found"),
        "once"
    );
    keys(&mut app, ":lsp<CR>");
    let path = std::env::var("PATH").unwrap_or_default();
    assert_eq!(
        app.editor().message().unwrap().text,
        format!("rust-analyzer: not found on PATH\nPATH={path}")
    );
}

#[test]
fn servers_get_their_settings_and_lsp_lists_them() {
    let config = r#"
[languages.rust]
language-server = { command = "fake-ls", settings = { check = { on = { level = 2 } } } }
"#;
    let (dir, mut app, seen) = with_server_in(app_with_config(config), "x\n", true);
    until(&mut app, |_| !seen.lock().unwrap().configuration.is_null());
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    assert_eq!(seen.settings, json!({"check": {"on": {"level": 2}}}));
    assert_eq!(
        seen.configuration,
        json!([{"level": 2}, {}]),
        "each section, or {{}} for one there isn't"
    );
    keys(&mut app, ":lsp<CR>");
    let root = dir.path().canonicalize().unwrap().display().to_string();
    assert_eq!(
        app.editor().message().unwrap().text,
        format!("fake-ls (rust): running in {root}, 1 file open")
    );
}

#[test]
fn a_new_file_named_relatively_gets_a_server_in_this_folder() {
    let (_dir, mut app, _) = with_server("x\n", true);
    keys(&mut app, ":e not-yet-there.rs<CR>");
    until(&mut app, |a| {
        let status = a.editor().lsp_status();
        status.len() == 2 && status.iter().all(|l| l.contains("running"))
    });
    let here = std::env::current_dir().unwrap().canonicalize().unwrap();
    let status = app.editor().lsp_status().join("\n");
    assert!(
        status.contains(&format!("running in {}", here.display())),
        "{status}"
    );
}

#[test]
fn a_new_unsaved_file_gets_its_diagnostics() {
    let (_dir, mut app, _) = with_server("x\n", true);
    // Started first, so the wait below is only for the diagnostics.
    ready(&mut app);
    keys(&mut app, ":e not-yet-there.rs<CR>ilet bad = 1;<Esc>");
    until(&mut app, |a| diagnostic_starts(a) == [(0, 4)]);
    keys(&mut app, "0]d");
    assert_eq!(common::cursor(&app), (0, 4));
}

#[test]
fn saving_tells_the_server() {
    let (_dir, mut app, seen) = with_server("x\n", true);
    let count = |seen: &Mutex<Seen>| {
        let seen = seen.lock().unwrap();
        seen.methods
            .iter()
            .filter(|m| *m == "textDocument/didSave")
            .count()
    };
    until(&mut app, |_| {
        seen.lock()
            .unwrap()
            .methods
            .contains(&"textDocument/didOpen".into())
    });
    keys(&mut app, "ibad<Esc>:w<CR>");
    until(&mut app, |_| count(&seen) == 1);
    keys(&mut app, ":w<CR>");
    until(&mut app, |_| count(&seen) == 2);
    let methods = seen.lock().unwrap().methods.clone();
    let save = methods.iter().position(|m| m == "textDocument/didSave");
    let change = methods.iter().position(|m| m == "textDocument/didChange");
    assert!(change < save, "the change goes first: {methods:?}");
}

/// Waits until the server is ready for requests.
fn ready(app: &mut App) {
    until(app, |a| {
        a.editor()
            .lsp_status()
            .iter()
            .any(|l| l.contains("running"))
    });
    // And until it has the document open.
    app.update();
}

#[test]
fn k_shows_the_hover_and_any_other_key_closes_it() {
    let (_dir, mut app, _) = with_server("let bad = 1;\nfine\n", true);
    ready(&mut app);
    keys(&mut app, "wK");
    until(&mut app, |a| a.editor().hover().is_some());
    let screen = render(&mut app, 60, 12);
    let rows: Vec<&str> = screen.rows.iter().map(|r| r.trim_end()).collect();
    let text = rows.join("\n");
    assert!(text.contains("│ fn bad()"), "no fences:\n{text}");
    assert!(text.contains("│ ─────"), "the rule:\n{text}");
    assert!(
        text.contains("│ A bad word, see fine."),
        "no Markdown markers:\n{text}"
    );
    assert!(!text.contains("```"), "{text}");
    keys(&mut app, "l");
    assert!(app.editor().hover().is_none());
    assert!(
        !render(&mut app, 60, 12)
            .rows
            .join("\n")
            .contains("A bad word")
    );

    keys(&mut app, "0K");
    app.update();
    until(&mut app, |a| a.editor().message().is_some());
    assert_eq!(
        app.editor().message().unwrap().text,
        "No hover information here"
    );
}

#[test]
fn resting_the_pointer_shows_the_hover_under_the_problem() {
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let text = format!("{}let bad = 1;\nfine\n", "\n".repeat(12));
    let (_dir, mut app, _) = with_server(&text, true);
    ready(&mut app);
    until(&mut app, |a| !diagnostic_starts(a).is_empty());
    keys(&mut app, "G");
    let screen = render(&mut app, 60, 24);
    let (x, y) = screen
        .rows
        .iter()
        .enumerate()
        .find_map(|(y, row)| row.find("bad").map(|b| (row[..b].chars().count(), y)))
        .unwrap();
    let rest = |app: &mut App, x: usize, y: usize| {
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: x as u16,
            row: y as u16,
            modifiers: KeyModifiers::NONE,
        });
        app.show_pending_hover();
    };
    rest(&mut app, x + 1, y);
    until(&mut app, |a| {
        a.editor()
            .hover()
            .is_some_and(|h| h.document.buffer().text().contains("A **bad** word"))
    });
    let text = render(&mut app, 60, 24).rows.join("\n");
    let above = text.find("A bad word").unwrap() < text.find("let bad").unwrap();
    assert!(above, "above the word:\n{text}");
    assert!(
        text.contains("⊗ bad word (fake)"),
        "the problem first:\n{text}"
    );
    assert!(
        text.contains("A bad word, see fine."),
        "then the hover:\n{text}"
    );

    // Nothing to say about `fine`: nothing shows, and nothing's said.
    rest(&mut app, x, y + 1);
    app.update();
    std::thread::sleep(Duration::from_millis(50));
    app.update();
    assert!(app.editor().hover().is_none());
    assert!(app.editor().message().is_none());
}

#[test]
fn ctrl_resting_peeks_at_the_definition() {
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let text = format!(
        "fn area() {{\n    1\n}}\nfn next() {{}}\n{}area();\n",
        "\n".repeat(10)
    );
    let (_dir, mut app, _) = with_server(&text, true);
    ready(&mut app);
    keys(&mut app, "G");
    let screen = render(&mut app, 60, 24);
    let (x, y) = screen
        .rows
        .iter()
        .enumerate()
        .find_map(|(y, row)| row.find("area();").map(|b| (row[..b].chars().count(), y)))
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: x as u16 + 1,
        row: y as u16,
        modifiers: KeyModifiers::CONTROL,
    });
    app.show_pending_hover();
    until(&mut app, |a| a.editor().hover().is_some());
    let shown = render(&mut app, 60, 24).rows.join("\n");
    assert!(shown.contains("main.rs:1"), "where it is:\n{shown}");
    assert!(shown.contains("│ fn area() {"), "{shown}");
    assert!(shown.contains("│     1"), "{shown}");
    assert!(
        !shown.contains("│ fn next"),
        "only that definition:\n{shown}"
    );
    assert!(!shown.contains("```"), "{shown}");
}

#[test]
fn gd_goes_to_the_definition_and_ctrl_o_comes_back() {
    let (_dir, mut app, _) = with_server("fn bad() {}\nfine\nbad();\n", true);
    ready(&mut app);
    keys(&mut app, "G");
    keys(&mut app, "gd");
    until(&mut app, |a| common::cursor(a) == (0, 3));
    keys(&mut app, "<C-o>");
    assert_eq!(common::cursor(&app), (2, 0));
}

#[test]
fn gd_opens_the_file_the_definition_is_in() {
    let (dir, mut app, _) = with_server("use other;\n", true);
    fs::write(dir.path().join(OTHER), "// other\n  pub fn other() {}\n").unwrap();
    ready(&mut app);
    keys(&mut app, "wgd");
    until(&mut app, |a| a.editor().document.name().ends_with(OTHER));
    assert_eq!(common::cursor(&app), (1, 2));
    keys(&mut app, "<C-o>");
    assert!(app.editor().document.name().ends_with("main.rs"));
}

#[test]
fn gr_lists_the_references_in_a_picker() {
    let (_dir, mut app, _) = with_server("fn bad() {}\nbad();\nlet x = bad;\n", true);
    ready(&mut app);
    keys(&mut app, "wgr");
    until(&mut app, |a| a.editor().picker().is_some());
    let picker = app.editor().picker().unwrap();
    assert_eq!(picker.title(), "References");
    assert_eq!(picker.total(), 3);
    let screen = render(&mut app, 80, 14).rows.join("\n");
    assert!(screen.contains("main.rs:3:9"), "{screen}");
    assert!(screen.contains("let x = bad;"), "the line: {screen}");
    keys(&mut app, "<C-n><C-n><CR>");
    assert_eq!(common::cursor(&app), (2, 8));
}

#[test]
fn servers_that_cant_answer_say_so() {
    let (_dir, mut app, _) = with_server("bad\n", false);
    ready(&mut app);
    keys(&mut app, "K");
    app.update();
    assert_eq!(
        app.editor().message().unwrap().text,
        "rust-analyzer doesn't offer hover information"
    );
    keys(&mut app, ":e notes.txt<CR>gd");
    app.update();
    assert_eq!(
        app.editor().message().unwrap().text,
        "No language server for this buffer (see :lsp)"
    );
}

/// The labels in the completion menu.
fn menu(app: &App) -> Vec<String> {
    app.editor().completion().map_or(Vec::new(), |c| {
        (0..c.items.len()).map(|i| c.label(i).to_string()).collect()
    })
}

#[test]
fn typing_shows_the_servers_completions_and_more_narrows_them() {
    let (_dir, mut app, _) = with_server("\n", true);
    ready(&mut app);
    keys(&mut app, "ib");
    app.update();
    assert!(menu(&app).is_empty(), "not after one letter");
    keys(&mut app, "a");
    until(&mut app, |a| !menu(a).is_empty());
    assert_eq!(
        menu(&app),
        ["bad_word", "badge()", "basket"],
        "matching `ba`"
    );
    assert_eq!(text(&app), "ba", "nothing goes in until chosen");
    let screen = render(&mut app, 60, 10).rows.join("\n");
    assert!(
        screen.contains("│ bad_word  i32"),
        "with details:\n{screen}"
    );
    assert!(
        screen.contains("│ badge()   function"),
        "or kinds:\n{screen}"
    );

    keys(&mut app, "dg");
    app.update();
    assert_eq!(menu(&app), ["badge()"]);
    keys(&mut app, "<Tab>");
    assert_eq!(text(&app), "badge", "the text to put in, not the label");
    assert!(menu(&app).is_empty());
}

#[test]
fn ctrl_n_puts_items_in_and_ctrl_e_puts_back_what_was_typed() {
    let (_dir, mut app, _) = with_server("\n", true);
    ready(&mut app);
    keys(&mut app, "iba");
    until(&mut app, |a| !menu(a).is_empty());
    keys(&mut app, "<C-n><C-n>");
    assert_eq!(text(&app), "badge");
    keys(&mut app, "<C-n>");
    assert_eq!(
        text(&app),
        "basket()",
        "a snippet, its first stop left to fill"
    );
    keys(&mut app, "<C-e>");
    assert_eq!(text(&app), "ba");
    assert!(menu(&app).is_empty());
}

#[test]
fn a_space_or_moving_away_closes_it() {
    let (_dir, mut app, _) = with_server("\n", true);
    ready(&mut app);
    keys(&mut app, "iba");
    until(&mut app, |a| !menu(a).is_empty());
    keys(&mut app, " ");
    assert!(menu(&app).is_empty());
    assert_eq!(text(&app), "ba ");
}

#[test]
fn characters_the_server_completes_after_start_it() {
    let (_dir, mut app, _) = with_server("\n", true);
    ready(&mut app);
    keys(&mut app, "ix;");
    app.update();
    assert!(menu(&app).is_empty(), "`;` isn't one of the server's");
    keys(&mut app, "x.");
    until(&mut app, |a| menu(a).len() == 6);
    keys(&mut app, "fi");
    app.update();
    assert_eq!(menu(&app), ["fine"]);
}

#[test]
fn taking_an_item_makes_its_import_too() {
    let (_dir, mut app, _) = with_server("fn main() {\n    \n}\n", true);
    ready(&mut app);
    keys(&mut app, "jA");
    keys(&mut app, "imp");
    until(&mut app, |a| menu(a) == ["imported"]);
    keys(&mut app, "<C-y>");
    assert_eq!(text(&app), "use imported;\nfn main() {\n    imported\n}");
    assert_eq!(common::cursor(&app), (2, 12), "still after the word");

    // One whose import comes from resolving it, once taken.
    keys(&mut app, "<CR>reso");
    until(&mut app, |a| menu(a) == ["resolved"]);
    keys(&mut app, "<Tab>");
    until(&mut app, |a| text(a).starts_with("use resolved;"));
    assert_eq!(common::cursor(&app), (4, 12));
    keys(&mut app, "x");
    assert!(text(&app).contains("    resolvedx\n"), "{}", text(&app));
}

#[test]
fn without_auto_complete_ctrl_space_asks() {
    let (_dir, mut app, _) = with_server("\n", true);
    ready(&mut app);
    keys(&mut app, ":set noauto-complete<CR>ibad");
    app.update();
    assert!(menu(&app).is_empty());
    keys(&mut app, "<C-Space>");
    until(&mut app, |a| menu(a) == ["bad_word", "badge()"]);
}

#[test]
fn format_makes_the_servers_edits_as_one_change() {
    let (_dir, mut app, seen) = with_server("let  x =   1;  \nfine\n    y  =  2\n", true);
    ready(&mut app);
    keys(&mut app, "jj$");
    keys(&mut app, ":format<CR>");
    assert_eq!(text(&app), "let x = 1;\nfine\n    y = 2");
    assert_eq!(common::cursor(&app), (2, 8), "still on the `2`");
    let options = seen.lock().unwrap().format_options.clone();
    assert_eq!(options, json!({"tabSize": 4, "insertSpaces": true}));
    keys(&mut app, "u");
    assert_eq!(text(&app), "let  x =   1;  \nfine\n    y  =  2", "one undo");
}

#[test]
fn format_with_a_range_formats_only_its_lines() {
    let (_dir, mut app, seen) = with_server("a  =  1\nb  =  2\nc  =  3\nd  =  4\n", true);
    ready(&mut app);
    keys(&mut app, "jVj:format<CR>");
    assert_eq!(text(&app), "a  =  1\nb = 2\nc = 3\nd  =  4");
    let range = seen.lock().unwrap().format_range.clone();
    assert_eq!(
        range,
        json!({"start": {"line": 1, "character": 0}, "end": {"line": 2, "character": 7}})
    );
    keys(&mut app, ":4format<CR>");
    assert_eq!(text(&app), "a  =  1\nb = 2\nc = 3\nd = 4");
    keys(&mut app, "u");
    assert_eq!(text(&app), "a  =  1\nb = 2\nc = 3\nd  =  4", "one undo");
}

#[test]
fn gq_has_a_server_that_formats_ranges_format_its_lines() {
    let (_dir, mut app, seen) = with_server("a  =  1\nb  =  2\nc  =  3\n", true);
    ready(&mut app);
    keys(&mut app, "jgqj");
    assert_eq!(
        text(&app),
        "a  =  1\nb = 2\nc = 3",
        "the server's, not a reflow"
    );
    assert_eq!(seen.lock().unwrap().format_range["start"]["line"], 1);
    assert_eq!(common::cursor(&app), (1, 0), "at the start, as Neovim");
    keys(&mut app, "u");
    assert_eq!(text(&app), "a  =  1\nb  =  2\nc  =  3", "one undo");
    // `gw` is always ferra's own reflow.
    keys(&mut app, "ggjgwj");
    assert_eq!(text(&app), "a  =  1\nb = 2 c = 3");
}

#[test]
fn gq_reflows_without_a_server_that_formats_ranges() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("whole-only"), "").unwrap();
    let (_dir, mut app, _) = with_server_at(dir, app(), "a  b\nc\n", true);
    ready(&mut app);
    keys(&mut app, "gqj");
    assert_eq!(text(&app), "a b c");
}

#[cfg(unix)]
#[test]
fn gq_asks_the_server_before_format_program() {
    let (_dir, mut app, _) = with_server("b  =  2\na  =  1\n", true);
    ready(&mut app);
    keys(&mut app, ":setlocal fp=sort<CR>gqj");
    assert_eq!(
        text(&app),
        "b = 2\na = 1",
        "the server's, as Neovim's formatexpr"
    );
}

#[cfg(unix)]
#[test]
fn gq_uses_format_program_when_the_server_formats_only_whole_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("whole-only"), "").unwrap();
    let (_dir, mut app, _) = with_server_at(dir, app(), "b\na\n", true);
    ready(&mut app);
    keys(&mut app, ":setlocal fp=sort<CR>gqj");
    assert_eq!(text(&app), "a\nb");
}

#[test]
fn a_server_that_formats_only_whole_files_says_so_for_a_range() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("whole-only"), "").unwrap();
    let (_dir, mut app, _) = with_server_at(dir, app(), "a  =  1\nb  =  2\n", true);
    ready(&mut app);
    keys(&mut app, ":2format<CR>");
    assert_eq!(
        app.editor().message().unwrap().text,
        "rust-analyzer doesn't format part of a file"
    );
    assert_eq!(text(&app), "a  =  1\nb  =  2");
    keys(&mut app, ":format<CR>");
    assert_eq!(text(&app), "a = 1\nb = 2", "the whole file it does");
}

#[test]
fn format_on_save_formats_what_is_written() {
    let config = "[languages.rust]\noptions = { format-on-save = true }\n";
    let (dir, mut app, _) = with_server_in(app_with_config(config), "a  =  1\n", true);
    ready(&mut app);
    keys(&mut app, ":w<CR>");
    assert_eq!(
        fs::read_to_string(dir.path().join("main.rs")).unwrap(),
        "a = 1\n"
    );
    assert!(!app.editor().document.is_modified());
    // Other languages don't.
    let other = dir.path().join("notes.txt");
    fs::write(&other, "b  =  2\n").unwrap();
    keys(
        &mut app,
        &format!(":e {}<CR>A <Esc>:w<CR>", other.display()),
    );
    assert_eq!(fs::read_to_string(&other).unwrap(), "b  =  2 \n");
}

#[test]
fn format_says_why_it_cant() {
    let (_dir, mut app, _) = with_server("a  b\n", false);
    ready(&mut app);
    keys(&mut app, ":format<CR>");
    assert_eq!(
        app.editor().message().unwrap().text,
        "rust-analyzer doesn't format"
    );
    keys(&mut app, ":e notes.txt<CR>:format<CR>");
    assert_eq!(
        app.editor().message().unwrap().text,
        "No language server for this buffer (see :lsp)"
    );
}

#[test]
fn a_server_that_doesnt_answer_doesnt_stop_the_save() {
    let config = "[languages.rust]\noptions = { format-on-save = true }\n";
    let (dir, mut app, _) = with_server_in(app_with_config(config), "slow  one\n", true);
    ready(&mut app);
    let started = std::time::Instant::now();
    keys(&mut app, ":w<CR>");
    assert!(started.elapsed() < Duration::from_secs(3), "gives up");
    assert_eq!(
        fs::read_to_string(dir.path().join("main.rs")).unwrap(),
        "slow  one\n"
    );
    let message = app.editor().message().unwrap().text.clone();
    assert!(
        message.ends_with("written (not formatted: rust-analyzer took too long to format)"),
        "{message}"
    );
}

#[test]
fn items_can_replace_the_dot_typed_before_them() {
    let (_dir, mut app, _) = with_server("// maybe\n\n", true);
    ready(&mut app);
    // The TypeScript server's optional `?.`, matched with its `.`.
    keys(&mut app, "jix.th");
    until(&mut app, |a| menu(a) == ["thing"]);
    keys(&mut app, "<Tab>");
    assert_eq!(text(&app), "// maybe\nx?.thing");
    assert_eq!(common::cursor(&app), (1, 8));
    // clangd's `->`, whose filter text leaves the `.` out: offered as soon
    // as the `.` is typed.
    keys(&mut app, "<Esc>ccp.");
    until(&mut app, |a| menu(a).len() == 3);
    keys(&mut app, "fi<Tab>");
    assert_eq!(text(&app), "// maybe\np->field");
}

#[test]
fn a_snippet_leaves_the_cursor_at_its_first_stop() {
    let (_dir, mut app, _) = with_server("// maybe\n\n", true);
    ready(&mut app);
    keys(&mut app, "jio.ca");
    until(&mut app, |a| menu(a) == ["call(arg)"]);
    keys(&mut app, "<C-y>");
    assert_eq!(text(&app), "// maybe\no.call()");
    assert_eq!(common::cursor(&app), (1, 7), "inside the brackets");
    keys(&mut app, "42");
    assert_eq!(text(&app), "// maybe\no.call(42)");
}

#[test]
fn dot_puts_a_snippet_back_with_the_cursor_inside_it() {
    let (_dir, mut app, _) = with_server("// maybe\n\n", true);
    ready(&mut app);
    keys(&mut app, "jio.ca");
    until(&mut app, |a| menu(a) == ["call(arg)"]);
    keys(&mut app, "<C-y>42<Esc>");
    keys(&mut app, ".");
    assert_eq!(text(&app), "// maybe\no.call(4o.call(42)2)");
}

#[test]
fn dot_after_taking_an_item_with_tab_puts_in_its_text_not_a_tab() {
    let (_dir, mut app, _) = with_server("fn main() {\n    \n}\n", true);
    ready(&mut app);
    keys(&mut app, "jA");
    keys(&mut app, "imp");
    until(&mut app, |a| menu(a) == ["imported"]);
    keys(&mut app, "<Tab><Esc>");
    assert_eq!(text(&app), "use imported;\nfn main() {\n    imported\n}");
    keys(&mut app, "j.");
    assert_eq!(
        text(&app),
        "use imported;\nfn main() {\n    imported\n}imported",
        "the text, not the import again"
    );
}

#[test]
fn rename_changes_the_name_everywhere_as_one_change() {
    let (_dir, mut app, _) = with_server("fn bad() {}\nbad();\nlet x = bad;\n", true);
    ready(&mut app);
    keys(&mut app, "jw:rename good<CR>");
    until(&mut app, |a| text(a).starts_with("fn good"));
    assert_eq!(text(&app), "fn good() {}\ngood();\nlet x = good;");
    assert_eq!(
        app.editor().message().unwrap().text,
        "Renamed to good: 3 changes in 1 file"
    );
    keys(&mut app, "u");
    assert_eq!(text(&app), "fn bad() {}\nbad();\nlet x = bad;", "one undo");
}

#[cfg(unix)]
#[test]
fn the_agents_rename_saves_the_files_it_opened_and_they_are_its() {
    let (dir, mut app, _, mut agent) = with_agent("use other;\nother();\n");
    let other = dir.path().join(OTHER);
    fs::write(&other, "pub fn other() {}\n").unwrap();
    ready(&mut app);
    let main = dir.path().join("main.rs");
    agent.call(1, "lsp_rename", json!({"path": main.to_str().unwrap(), "line": 2, "symbol": "other", "new_name": "another"}));
    let said = agent.answer(&mut app, 1).unwrap();
    let (said, problems) = said.split_once('\n').unwrap();
    assert_eq!(said, "Renamed to another: 3 changes in 2 files");
    // Then what the server says of both files now.
    assert!(problems.contains("No errors or warnings in "), "{problems}");
    assert!(problems.contains("other.rs."), "{problems}");
    // Both saved (neither had the user's unsaved changes), the other file
    // opened for the agent's tools, to close after its turn.
    assert_eq!(fs::read_to_string(&other).unwrap(), "pub fn another() {}\n");
    assert_eq!(
        fs::read_to_string(&main).unwrap(),
        "use another;\nanother();\n"
    );
    let state = app.editor().agent_editor_state();
    assert!(state.contains("other.rs (opened for your tools"), "{state}");
    assert!(!state.contains("main.rs (opened for your tools"), "{state}");
}

#[test]
fn rename_changes_other_files_and_leaves_them_to_save() {
    let (dir, mut app, _) = with_server("use other;\nother();\n", true);
    let other = dir.path().join(OTHER);
    fs::write(&other, "pub fn other() {}\n").unwrap();
    ready(&mut app);
    keys(&mut app, "w:rename another<CR>");
    until(&mut app, |a| text(a).starts_with("use another"));
    assert_eq!(
        app.editor().message().unwrap().text,
        "Renamed to another: 3 changes in 2 files (1 other file not saved yet: :wa)"
    );
    assert!(
        app.editor().document.name().ends_with("main.rs"),
        "still here"
    );
    assert_eq!(fs::read_to_string(&other).unwrap(), "pub fn other() {}\n");
    keys(&mut app, ":wa<CR>");
    assert_eq!(fs::read_to_string(&other).unwrap(), "pub fn another() {}\n");
}

#[test]
fn f2_starts_a_rename_of_the_name_the_server_gives() {
    let (_dir, mut app, seen) = with_server("let bad = 1;\nfn f<'a>() {}\nnamed;\n", true);
    ready(&mut app);
    keys(&mut app, "w<F2>");
    until(&mut app, |a| {
        a.editor().command_line.input() == "rename bad"
    });
    assert!(
        seen.lock()
            .unwrap()
            .methods
            .contains(&"textDocument/prepareRename".into())
    );
    keys(&mut app, "<C-w>better<CR>");
    until(&mut app, |a| text(a).starts_with("let better = 1;"));
    // The range's text, not the word: a lifetime's `'`.
    keys(&mut app, "j0f'l<F2>");
    until(&mut app, |a| a.editor().command_line.input() == "rename 'a");
    // Its placeholder.
    keys(&mut app, "<Esc>j0<F2>");
    until(&mut app, |a| {
        a.editor().command_line.input() == "rename the_name"
    });
}

#[test]
fn f2_says_when_theres_nothing_to_rename() {
    let (_dir, mut app, _) = with_server("let bad = 1;\n", true);
    ready(&mut app);
    keys(&mut app, "<F2>");
    until(&mut app, |a| {
        a.editor()
            .message()
            .is_some_and(|m| m.text == "Nothing to rename here")
    });
    assert_ne!(
        app.editor().mode(),
        ferra::editor::Mode::Command,
        "no prompt"
    );
}

#[test]
fn f2_takes_the_word_when_the_server_cant_say_first() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("no-prepare"), "").unwrap();
    let (_dir, mut app, seen) = with_server_at(dir, app(), "let bad = 1;\n", true);
    ready(&mut app);
    keys(&mut app, "w<F2>");
    until(&mut app, |a| {
        a.editor().command_line.input() == "rename bad"
    });
    assert!(
        !seen
            .lock()
            .unwrap()
            .methods
            .contains(&"textDocument/prepareRename".into())
    );
    keys(&mut app, "<C-w>better<CR>");
    until(&mut app, |a| text(a) == "let better = 1;");
}

#[test]
fn rename_says_why_it_cant() {
    let (_dir, mut app, _) = with_server("fine\n", true);
    ready(&mut app);
    keys(&mut app, ":rename ok<CR>");
    until(&mut app, |a| {
        a.editor()
            .message()
            .is_some_and(|m| m.text == "rust-analyzer: can't rename that")
    });
    keys(&mut app, ":rename<CR>");
    assert_eq!(
        app.editor().message().unwrap().text,
        "Rename to what? :rename new_name"
    );
}

/// The labels in the open picker.
fn picker_labels(app: &mut App) -> Vec<String> {
    let screen = render(app, 80, 14).rows.join("\n");
    let picker = app.editor().picker().unwrap();
    let labels: Vec<String> = picker
        .matches()
        .iter()
        .map(|m| picker.item(m).label.clone())
        .collect();
    for label in &labels {
        assert!(screen.contains(label.as_str()), "{label} shown:\n{screen}");
    }
    labels
}

#[test]
fn space_a_lists_code_actions_and_makes_a_quick_fix() {
    let (_dir, mut app, _) = with_server("let x = bad;\nfine\n", true);
    ready(&mut app);
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    keys(&mut app, "<Space>a");
    until(&mut app, |a| a.editor().picker().is_some());
    assert_eq!(
        picker_labels(&mut app),
        ["Replace with good", "Upper-case the line", "Add a note"],
        "the fix for the diagnostic sent back; the disabled one left out"
    );
    keys(&mut app, "<CR>");
    until(&mut app, |a| text(a).starts_with("let x = good;"));
    assert_eq!(
        app.editor().message().unwrap().text,
        "Replace with good: 1 change in 1 file"
    );
    keys(&mut app, "u");
    assert_eq!(text(&app), "let x = bad;\nfine", "one undo");
}

#[test]
fn actions_that_need_resolving_or_run_a_command() {
    let (_dir, mut app, seen) = with_server("fine\nalso fine\n", true);
    ready(&mut app);
    keys(&mut app, "j<Space>a");
    until(&mut app, |a| a.editor().picker().is_some());
    assert_eq!(
        picker_labels(&mut app),
        ["Upper-case the line", "Add a note"]
    );
    keys(&mut app, "<CR>");
    until(&mut app, |a| text(a) == "fine\nALSO FINE");

    // A command: the server sends the edit back to be made.
    keys(&mut app, "<Space>a");
    until(&mut app, |a| a.editor().picker().is_some());
    keys(&mut app, "<C-n><CR>");
    until(&mut app, |a| text(a).starts_with("// note"));
    assert_eq!(text(&app), "// note\nfine\nALSO FINE");
    assert_eq!(
        app.editor().message().unwrap().text,
        "Note added: 1 change in 1 file"
    );
    until(&mut app, |_| !seen.lock().unwrap().applied.is_null());
    assert_eq!(seen.lock().unwrap().applied, json!({"applied": true}));
}

#[test]
fn code_actions_for_a_selection_ask_about_its_range() {
    let (_dir, mut app, seen) = with_server("one\ntwo\nthree\n", true);
    ready(&mut app);
    keys(&mut app, "lvjl<Space>a");
    until(&mut app, |a| a.editor().picker().is_some());
    let range = seen.lock().unwrap().action_range.clone();
    assert_eq!(
        range,
        json!({"start": {"line": 0, "character": 1}, "end": {"line": 1, "character": 3}})
    );
    assert_eq!(
        app.editor().mode(),
        ferra::editor::Mode::Picker,
        "visual mode ended"
    );
}

#[test]
fn a_server_without_code_actions_says_so() {
    let (_dir, mut app, _) = with_server("x\n", false);
    ready(&mut app);
    keys(&mut app, "<Space>a");
    app.update();
    assert_eq!(
        app.editor().message().unwrap().text,
        "rust-analyzer doesn't offer code actions"
    );
}

#[test]
fn diagnostics_move_with_edits_before_the_server_answers() {
    let (_dir, mut app, _) = with_server("let bad = 1;\nfine\n", true);
    until(&mut app, |a| diagnostic_starts(a) == [(0, 4)]);
    // A line added above, and the line indented: drawn where the text went
    // straight away, not where it was, before the server's new list.
    keys(&mut app, "Onew line<Esc>j>>");
    assert_eq!(diagnostic_starts(&app), [(1, 8)]);
    let rows = render(&mut app, 60, 6).rows;
    assert!(!rows[0].contains('●'), "{rows:?}");
    assert!(rows[1].contains("let bad = 1;  ● bad word"), "{rows:?}");
    keys(&mut app, "u");
    assert_eq!(diagnostic_starts(&app), [(1, 4)], "undo moves it back");
    // And the server's list, when it comes, agrees.
    until(&mut app, |a| diagnostic_starts(a) == [(1, 4)]);
}

#[test]
fn a_list_for_text_since_changed_is_left_out() {
    let (_dir, mut app, seen) = with_server("x bad\n", true);
    until(&mut app, |a| diagnostic_starts(a) == [(0, 2)]);
    keys(&mut app, "Astale<Esc>");
    until(&mut app, |a| seen.lock().unwrap().text == text(a));
    // Let the server's (out of date) list through.
    for _ in 0..20 {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        diagnostic_starts(&app),
        [(0, 2)],
        "not the old list's (0, 0)"
    );
}

fn active_parameter(app: &App) -> Option<String> {
    let help = app.editor().signature_help()?.shown();
    let (start, end) = help.active?;
    Some(help.label.chars().skip(start).take(end - start).collect())
}

#[test]
fn typing_a_call_shows_its_signature_above_the_line() {
    // Room above the line for it (with none, it goes below).
    let (_dir, mut app, seen) = with_server(&"\n".repeat(8), true);
    ready(&mut app);
    keys(&mut app, "Gocall(");
    until(&mut app, |a| a.editor().signature_help().is_some());
    assert_eq!(active_parameter(&app).as_deref(), Some("first: u8"));
    let context = seen.lock().unwrap().signature_context.clone();
    assert_eq!(context["triggerCharacter"], "(");
    let rows = render(&mut app, 60, 12).rows;
    let at = rows.iter().position(|r| r.contains("call(first")).unwrap();
    let typing = rows
        .iter()
        .position(|r| r.contains("call(") && !r.contains("first"))
        .unwrap();
    assert!(at < typing, "above the line: {rows:#?}");
    assert!(rows[at + 2].contains("The first."), "{rows:#?}");

    // Typing on: the parameter follows.
    keys(&mut app, "1, ");
    until(&mut app, |a| {
        active_parameter(a).as_deref() == Some("second: u8")
    });
    assert_eq!(seen.lock().unwrap().signature_context["isRetrigger"], true);
    // Closing the call closes it.
    keys(&mut app, "2)");
    until(&mut app, |a| a.editor().signature_help().is_none());
}

#[test]
fn ctrl_s_steps_through_a_calls_overloads() {
    let (_dir, mut app, seen) = with_server(&"\n".repeat(8), true);
    ready(&mut app);
    keys(&mut app, "Goover(");
    until(&mut app, |a| a.editor().signature_help().is_some());
    let shown = |a: &App| a.editor().signature_help().unwrap().shown().label.clone();
    assert_eq!(shown(&app), "over(a: u8)");
    assert!(
        render(&mut app, 60, 12)
            .rows
            .iter()
            .any(|r| r.contains(" 1/2 "))
    );
    keys(&mut app, "<C-s>");
    assert_eq!(shown(&app), "over(a: u8, b: u8)");
    assert!(
        render(&mut app, 60, 12)
            .rows
            .iter()
            .any(|r| r.contains(" 2/2 "))
    );

    // Typing on asks again, saying which is picked; the server answers
    // with its own first, but the one picked stays, its parameter
    // following.
    keys(&mut app, "1, ");
    until(&mut app, |a| {
        active_parameter(a).as_deref() == Some("b: u8")
    });
    assert_eq!(shown(&app), "over(a: u8, b: u8)");
    let context = seen.lock().unwrap().signature_context.clone();
    assert_eq!(context["activeSignatureHelp"]["activeSignature"], 1);
    assert_eq!(
        context["activeSignatureHelp"]["signatures"][1]["label"],
        "over(a: u8, b: u8)"
    );
    // Round to the first.
    keys(&mut app, "<C-s>");
    assert_eq!(shown(&app), "over(a: u8)");
}

#[test]
fn leaving_insert_mode_closes_the_signature() {
    let (_dir, mut app, _) = with_server("\n", true);
    ready(&mut app);
    keys(&mut app, "icall(");
    until(&mut app, |a| a.editor().signature_help().is_some());
    keys(&mut app, "<Esc>");
    assert!(app.editor().signature_help().is_none());
    assert!(
        !render(&mut app, 60, 8)
            .rows
            .join("\n")
            .contains("first: u8")
    );
}

#[test]
fn ctrl_s_asks_for_the_signature() {
    let (_dir, mut app, _) = with_server("call(1, \nx\n", true);
    ready(&mut app);
    keys(&mut app, "A<C-s>");
    until(&mut app, |a| {
        active_parameter(a).as_deref() == Some("second: u8")
    });
    keys(&mut app, "<Esc>jA<C-s>");
    until(&mut app, |a| {
        a.editor()
            .message()
            .is_some_and(|m| m.text == "No signature here")
    });
}

const SHAPES: &str = "struct Circle {\n    fn area() {}\n    fn scale() {}\n}\nfn main() {}\n";

#[test]
fn space_s_lists_the_files_symbols_to_go_to() {
    let (_dir, mut app, _) = with_server(SHAPES, true);
    ready(&mut app);
    keys(&mut app, "G<Space>s");
    until(&mut app, |a| a.editor().picker().is_some());
    assert_eq!(picker_labels(&mut app), ["Circle", "area", "scale", "main"]);
    let screen = render(&mut app, 80, 14).rows.join("\n");
    assert!(screen.contains("method in Circle"), "{screen}");
    assert!(screen.contains("struct"), "{screen}");
    keys(&mut app, "sca<CR>");
    assert_eq!(common::cursor(&app), (2, 7));
    keys(&mut app, "<C-o>");
    assert_eq!(common::cursor(&app).0, 4, "a jump back to the last line");
}

#[test]
fn flat_symbol_lists_say_what_each_is_in() {
    let (_dir, mut app, _) = with_server(&format!("// flat\n{SHAPES}"), true);
    ready(&mut app);
    keys(&mut app, "<Space>s");
    until(&mut app, |a| a.editor().picker().is_some());
    assert_eq!(picker_labels(&mut app), ["Circle", "area", "scale", "main"]);
    let screen = render(&mut app, 80, 14).rows.join("\n");
    assert!(screen.contains("method in Circle"), "{screen}");
}

#[test]
fn space_shift_s_searches_the_project_as_you_type() {
    let (dir, mut app, seen) = with_server(SHAPES, true);
    fs::write(dir.path().join(OTHER), "fn helper() {}\nfn area_of() {}\n").unwrap();
    ready(&mut app);
    keys(&mut app, "<Space>S");
    assert_eq!(app.editor().picker().unwrap().title(), "Project symbols");
    keys(&mut app, "are");
    until(&mut app, |a| a.editor().picker().unwrap().total() == 2);
    assert_eq!(picker_labels(&mut app), ["area", "area_of"]);
    assert_eq!(seen.lock().unwrap().symbol_queries, ["", "a", "ar", "are"]);
    keys(&mut app, "_<CR>");
    until(&mut app, |a| a.editor().document.name().ends_with(OTHER));
    assert_eq!(common::cursor(&app), (1, 3));
}

#[test]
fn space_d_lists_this_files_diagnostics_to_go_to() {
    let (_dir, mut app, _) = with_server("let bad = 1;\nfine\nbad;\n", true);
    until(&mut app, |a| a.editor().diagnostics(doc(a)).len() == 2);
    keys(&mut app, "<Space>d");
    assert_eq!(picker_labels(&mut app), ["1:5  bad word", "3:1  bad word"]);
    let screen = render(&mut app, 80, 14).rows.join("\n");
    assert!(screen.contains("error fake"), "{screen}");
    keys(&mut app, "<C-n><CR>");
    assert_eq!(common::cursor(&app), (2, 0));
}

#[test]
fn space_shift_d_lists_the_projects_with_files_not_open() {
    let (dir, mut app, _) = with_server("// project\nlet bad = 1;\n", true);
    fs::write(dir.path().join(OTHER), "fn x() {\n    let thing = 1;\n}\n").unwrap();
    until(&mut app, |a| a.editor().diagnostics(doc(a)).len() == 1);
    // The other file's comes in after (no way to wait on it but to look).
    for _ in 0..20 {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    keys(&mut app, "<Space>D");
    // The files aren't under the current folder here, so their paths are
    // in full (resolved alike): just their ends.
    let picker = app.editor().picker().unwrap();
    let labels: Vec<String> = picker
        .matches()
        .iter()
        .map(|m| picker.item(m).label.clone())
        .collect();
    let dir = dir.path().canonicalize().unwrap().display().to_string();
    assert_eq!(
        labels,
        [
            format!("{dir}/main.rs:2:5  bad word"),
            format!("{dir}/other.rs:2:5  unused thing"),
        ],
        "errors first"
    );
    keys(&mut app, "unused");
    assert!(
        render(&mut app, 120, 14)
            .rows
            .join("\n")
            .contains("unused thing")
    );
    keys(&mut app, "<C-u>");
    // Going to it opens it with its diagnostic already there.
    keys(&mut app, "<C-n><CR>");
    assert!(
        app.editor().document.name().ends_with(OTHER),
        "{}",
        app.editor().document.name()
    );
    assert_eq!(common::cursor(&app), (1, 4));
    assert_eq!(diagnostic_starts(&app), [(1, 4)]);
    keys(&mut app, ":b main<CR><Space>d<C-n>");
    assert_eq!(
        picker_labels(&mut app),
        ["2:5  bad word"],
        "only this file's"
    );
}

#[test]
fn no_diagnostics_says_so() {
    let (_dir, mut app, _) = with_server("fine\n", true);
    ready(&mut app);
    keys(&mut app, "<Space>d");
    assert_eq!(
        app.editor().message().unwrap().text,
        "No diagnostics in this buffer"
    );
}

#[test]
fn a_servers_progress_shows_in_the_status_line_until_it_ends() {
    let (_dir, mut app, _) = with_server("// indexing\n", true);
    until(&mut app, |a| a.editor().lsp_progress().is_some());
    assert_eq!(
        app.editor().lsp_progress(),
        Some("rust-analyzer: Indexing 45%")
    );
    let rows = render(&mut app, 80, 6).rows;
    assert!(
        rows.iter()
            .any(|r| r.contains("rust-analyzer: Indexing 45%")),
        "{rows:#?}"
    );
    keys(&mut app, ":lsp<CR>");
    assert!(
        app.editor()
            .message()
            .unwrap()
            .text
            .contains("running (Indexing 45%)"),
        "{}",
        app.editor().message().unwrap().text
    );
    // It ends (on an edit, here).
    keys(&mut app, "Ax<Esc>");
    until(&mut app, |a| a.editor().lsp_progress().is_none());
    assert!(!render(&mut app, 80, 6).rows.join("").contains("Indexing"));
}

#[test]
fn lsp_restart_starts_the_server_again() {
    let (_dir, mut app, seen) = with_server("let bad = 1;\n", true);
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    assert_eq!(seen.lock().unwrap().starts, 1);
    keys(&mut app, ":lsp-restart<CR>");
    app.update();
    assert_eq!(
        app.editor().message().unwrap().text,
        "Restarting rust-analyzer"
    );
    // Started again, with the file open in it and its diagnostics back.
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    assert_eq!(seen.lock().unwrap().starts, 2);
    keys(&mut app, ":lsp-restart nothing<CR>");
    app.update();
    assert_eq!(
        app.editor().message().unwrap().text,
        "No language server to restart (see :lsp)"
    );
}

/// A server that isn't installed, counting the tries.
struct MissingCounted(Arc<Mutex<usize>>);

impl Spawner for MissingCounted {
    fn spawn(&self, _: &ServerConfig, _: &Path) -> io::Result<Streams> {
        *self.0.lock().unwrap() += 1;
        Err(io::Error::new(io::ErrorKind::NotFound, "no such program"))
    }
}

#[test]
fn lsp_restart_tries_servers_that_werent_found_again() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "x\n").unwrap();
    let tries = Arc::new(Mutex::new(0));
    let mut app = app();
    app.lsp_mut()
        .set_spawner(Box::new(MissingCounted(Arc::clone(&tries))));
    app.open(&dir.path().join("a.rs"));
    app.update();
    app.update();
    assert_eq!(*tries.lock().unwrap(), 1, "tried once");
    keys(&mut app, ":lsp-restart<CR>");
    app.update();
    assert_eq!(
        *tries.lock().unwrap(),
        2,
        "and again (once it's installed, say)"
    );
}

fn references(app: &App) -> Vec<(usize, usize)> {
    app.editor()
        .references_shown()
        .iter()
        .map(|(start, _)| (start.row, start.col))
        .collect()
}

#[test]
fn resting_on_a_symbol_highlights_its_other_uses() {
    let (_dir, mut app, seen) = with_server("let value = 1;\nfine\nvalue + value\n", true);
    ready(&mut app);
    keys(&mut app, "w");
    until(&mut app, |a| references(a).len() == 3);
    assert_eq!(references(&app), [(0, 4), (2, 0), (2, 8)]);
    // Moving off it hides them straight away; resting elsewhere asks again.
    keys(&mut app, "j");
    assert!(references(&app).is_empty());
    keys(&mut app, "k");
    assert_eq!(references(&app).len(), 3, "back on it: shown again");
    // An edit hides them until the server's asked about the new text.
    keys(&mut app, "x");
    assert!(references(&app).is_empty());
    let asked = |seen: &Mutex<Seen>| {
        seen.lock()
            .unwrap()
            .methods
            .iter()
            .filter(|m| *m == "textDocument/documentHighlight")
            .count()
    };
    let before = asked(&seen);
    until(&mut app, |_| asked(&seen) > before);
}

#[test]
fn highlight_references_can_be_turned_off() {
    let (_dir, mut app, seen) = with_server("let value = 1;\nvalue\n", true);
    ready(&mut app);
    keys(&mut app, ":set nohighlight-references<CR>w");
    for _ in 0..40 {
        app.update();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(references(&app).is_empty());
    assert!(
        !seen
            .lock()
            .unwrap()
            .methods
            .contains(&"textDocument/documentHighlight".into())
    );
}

fn hints(app: &App) -> Vec<(usize, usize, String)> {
    app.editor()
        .document
        .virtual_text()
        .iter()
        .map(|v| (v.at.row, v.at.col, v.text.clone()))
        .collect()
}

#[test]
fn inlay_hints_show_in_the_text_and_follow_edits() {
    let (_dir, mut app, seen) = with_server("let x = 1;\nlet yy = 2;\n", true);
    ready(&mut app);
    until(&mut app, |a| hints(a).len() == 2);
    assert_eq!(hints(&app)[0], (0, 5, ": i32".into()));
    let screen = render(&mut app, 40, 6);
    assert!(
        screen.rows[0].contains("let x: i32 = 1;"),
        "{:?}",
        screen.rows
    );
    assert!(
        screen.rows[1].contains("let yy: i32 = 2;"),
        "{:?}",
        screen.rows
    );

    // An edit moves them along at once, and they're asked for again.
    let asked = |seen: &Mutex<Seen>| {
        seen.lock()
            .unwrap()
            .methods
            .iter()
            .filter(|m| *m == "textDocument/inlayHint")
            .count()
    };
    let before = asked(&seen);
    keys(&mut app, "O// note<Esc>");
    assert_eq!(
        hints(&app)[0],
        (1, 5, ": i32".into()),
        "moved down with its line"
    );
    until(&mut app, |_| asked(&seen) > before);

    // Turned off, they go; on again, they're back.
    keys(&mut app, ":inlay_hints<CR>");
    let screen = render(&mut app, 40, 6);
    assert!(screen.rows[1].contains("let x = 1;"), "{:?}", screen.rows);
    keys(&mut app, ":inlay_hints<CR>");
    until(&mut app, |a| hints(a).len() == 2);
}

/// Draws, as the event loop does (which asks for the outline), until
/// `done`.
fn drawn_until(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        render(app, 90, 20);
        app.update();
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out");
}

#[test]
fn the_outline_and_the_breadcrumbs_show_the_files_symbols() {
    let (_dir, mut app, _) = with_server_in(
        app_with_config("[editor]\nbreadcrumbs = true\n"),
        SHAPES,
        true,
    );
    ready(&mut app);
    drawn_until(&mut app, |a| {
        a.editor().outline().is_some_and(|s| s.len() == 4)
    });
    let symbols = app.editor().outline().unwrap();
    let names: Vec<(&str, usize)> = symbols.iter().map(|s| (s.name.as_str(), s.depth)).collect();
    assert_eq!(
        names,
        [("Circle", 0), ("area", 1), ("scale", 1), ("main", 0)]
    );

    // The breadcrumbs: the file, and the symbol the cursor's on.
    keys(&mut app, "2G0fa");
    let screen = render(&mut app, 90, 20).rows.join("\n");
    assert!(screen.contains("main.rs › ƒ area"), "{screen}");

    // The Outline view: each symbol, nested; <CR> goes to one.
    keys(&mut app, "<Space>e:sidebar_view outline<CR>");
    assert!(app.editor().outline_shown());
    let screen = render(&mut app, 90, 20).rows.join("\n");
    assert!(screen.contains("◆ Circle"), "{screen}");
    assert!(screen.contains("  ƒ area"), "{screen}");
    keys(&mut app, "G<CR>");
    assert_eq!(common::cursor(&app), (4, 3), "main");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Normal);

    // An edit: asked again, and the new symbol shows.
    keys(&mut app, "Ofn added() {}<Esc>");
    drawn_until(&mut app, |a| {
        a.editor().outline().is_some_and(|s| s.len() == 5)
    });
}

/// The coding agent, as these tests talk to it: a fake one that only
/// starts a session (so ferra serves its tools), and a connection to those
/// tools as its MCP bridge makes. Answers come back through `App::update`,
/// as they do for a real agent.
#[cfg(unix)]
struct Agent {
    writer: std::os::unix::net::UnixStream,
    answers: std::sync::mpsc::Receiver<Value>,
}

/// A fake agent that answers `initialize` and `session/new`, and nothing
/// else (the tests here make tool calls, not prompts).
#[cfg(unix)]
struct SessionOnly;

#[cfg(unix)]
impl ferra::agent::Spawner for SessionOnly {
    fn spawn(
        &self,
        _: &ferra::agent::AgentConfig,
        _: &Path,
    ) -> io::Result<ferra::agent::client::Streams> {
        use ferra::agent::client::{read_message, write_message};
        let (client_reads, mut agent_writes) = io::pipe()?;
        let (agent_reads, client_writes) = io::pipe()?;
        std::thread::spawn(move || {
            let mut input = BufReader::new(agent_reads);
            while let Ok(Some(message)) = read_message(&mut input) {
                let result = match message["method"].as_str().unwrap_or_default() {
                    "initialize" => json!({"protocolVersion": 1}),
                    "session/new" => json!({"sessionId": "s1"}),
                    _ => continue,
                };
                let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": result});
                let _ = write_message(&mut agent_writes, &reply);
            }
        });
        Ok(ferra::agent::client::Streams {
            reader: Box::new(BufReader::new(client_reads)),
            writer: Box::new(client_writes),
            child: None,
        })
    }
}

/// The agent started and connected to ferra's tools, then a file with a
/// language server, as `with_server` makes. The chat's closed again, so
/// keys go to the buffer.
#[cfg(unix)]
fn with_agent(content: &str) -> (tempfile::TempDir, App, Arc<Mutex<Seen>>, Agent) {
    use std::os::unix::net::UnixStream;
    let mut app =
        app_with_config("[editor]\nagent = \"fake\"\n[agents.fake]\ncommand = \"fake-agent\"\n");
    app.agents_mut().set_spawner(Box::new(SessionOnly));
    keys(&mut app, ":agent<CR>");
    until(&mut app, |a| {
        a.editor().agent_chat().status() == &ferra::editor::agent::AgentStatus::Ready
    });
    app.editor_mut().agent_close();
    // The file's opened after, so its server's only just starting.
    let (dir, mut app, seen) = with_server_in(app, content, true);
    let socket = app.agents_mut().mcp_socket().expect("served").to_path_buf();
    let stream = UnixStream::connect(&socket).unwrap();
    let writer = stream.try_clone().unwrap();
    let (sender, answers) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        while let Ok(Some(message)) = ferra::agent::client::read_message(&mut reader) {
            let _ = sender.send(message);
        }
    });
    (dir, app, seen, Agent { writer, answers })
}

#[cfg(unix)]
impl Agent {
    /// Calls tool `name` with `args`, as request `id`: its answer comes
    /// from `answer`.
    fn call(&mut self, id: u64, name: &str, args: Value) {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                             "params": {"name": name, "arguments": args}});
        ferra::agent::client::write_message(&mut self.writer, &request).unwrap();
    }

    /// The answer to request `id`, once it comes: its text, or the error.
    /// A call that changed files is answered once their diagnostics have
    /// settled, so this waits longer than the servers take.
    fn answer(&mut self, app: &mut App, id: u64) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            app.update();
            if let Ok(answer) = self.answers.try_recv() {
                assert_eq!(answer["id"], id, "{answer}");
                let result = &answer["result"];
                let text = result["content"][0]["text"].as_str().unwrap().to_string();
                return if result["isError"] == true {
                    Err(text)
                } else {
                    Ok(text)
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no answer to {id}");
    }

    /// Calls tool `name` with `args` and waits for its answer.
    fn ask(&mut self, app: &mut App, id: u64, name: &str, args: Value) -> Result<String, String> {
        self.call(id, name, args);
        self.answer(app, id)
    }
}

#[cfg(unix)]
#[test]
fn the_agent_asks_servers_and_gets_text() {
    let (dir, mut app, _, mut agent) = with_agent("fn bad() {}\nbad();\nlet x = bad;\n");
    ready(&mut app);
    let path = dir.path().join("main.rs");
    let file = path.to_str().unwrap();
    let uses = agent.ask(
        &mut app,
        1,
        "lsp_references",
        json!({"path": file, "line": 3, "symbol": "bad"}),
    );
    let uses = uses.unwrap();
    assert!(uses.contains("main.rs:1:4  fn bad() {}"), "{uses}");
    assert!(uses.contains("main.rs:3:9  let x = bad;"), "{uses}");
    assert!(app.editor().picker().is_none(), "nothing shown");
    let found = agent.ask(
        &mut app,
        2,
        "lsp_definition",
        json!({"path": file, "line": 2, "column": 1}),
    );
    assert!(found.unwrap().contains("main.rs:1:4"));
    // A symbol that isn't on the line is said to be missing at once.
    let missing = agent.ask(
        &mut app,
        8,
        "lsp_hover",
        json!({"path": file, "line": 1, "symbol": "nope"}),
    );
    assert!(missing.unwrap_err().contains("isn't on line 1"));
    let past = agent.ask(
        &mut app,
        9,
        "lsp_hover",
        json!({"path": file, "line": 1, "column": 500}),
    );
    assert_eq!(
        past.unwrap_err(),
        "Column 500 is past the end of line 1 (11 columns)"
    );
}

#[cfg(unix)]
#[test]
fn the_agent_renames_formats_and_makes_code_actions() {
    let (dir, mut app, _, mut agent) = with_agent("fn bad() {}\nlet  x =  bad;\n");
    ready(&mut app);
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    let path = dir.path().join("main.rs");
    let file = path.to_str().unwrap();

    // The fixes for the problem on line 2, then the first made: saved, as
    // the file had no unsaved changes.
    agent.call(
        1,
        "lsp_code_actions",
        json!({"path": file, "line": 2, "symbol": "bad"}),
    );
    let listed = agent.answer(&mut app, 1).unwrap();
    assert!(
        listed.starts_with("1. Replace with good (quickfix)"),
        "{listed}"
    );
    let list: u64 = listed
        .split("list ")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    agent.call(
        2,
        "lsp_apply_code_action",
        json!({"path": file, "list": list, "number": 1}),
    );
    let made = agent.answer(&mut app, 2).unwrap();
    let (said, problems) = made.split_once('\n').unwrap();
    assert!(
        said.starts_with("Made Replace with good in ") && said.ends_with("main.rs"),
        "{made}"
    );
    // Then the problem left: the `bad` it didn't touch.
    assert!(problems.contains("1:4  error: bad word (fake)"), "{made}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "fn bad() {}\nlet  x =  good;\n"
    );

    agent.call(3, "lsp_format", json!({"path": file}));
    let formatted = agent.answer(&mut app, 3).unwrap();
    let said = formatted.lines().next().unwrap();
    assert!(
        said.starts_with("Formatted ") && said.ends_with("main.rs (1 change)"),
        "{formatted}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "fn bad() {}\nlet x = good;\n"
    );

    // With unsaved changes of the user's, the rename's left unsaved.
    keys(&mut app, "ggA // mine<Esc>");
    agent.call(
        4,
        "lsp_rename",
        json!({"path": file, "line": 1, "symbol": "bad", "new_name": "worse"}),
    );
    let renamed = agent.answer(&mut app, 4).unwrap();
    assert!(
        renamed.starts_with("Renamed to worse: 1 change in 1 file"),
        "{renamed}"
    );
    assert!(renamed.contains("Not saved"), "{renamed}");
    assert_eq!(text(&app), "fn worse() {} // mine\nlet x = good;");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "fn bad() {}\nlet x = good;\n"
    );
    // Undone as the user's own changes are.
    keys(&mut app, "u");
    assert_eq!(text(&app), "fn bad() {} // mine\nlet x = good;");
}

#[cfg(unix)]
#[test]
fn the_agent_asks_for_a_files_source_actions() {
    let (dir, mut app, seen, mut agent) = with_agent("fn bad() {}\nlet  x =  bad;\n");
    ready(&mut app);
    let path = dir.path().join("main.rs");
    // No line: the file-wide ones, for all of it.
    agent.call(
        1,
        "lsp_code_actions",
        json!({"path": path.to_str().unwrap()}),
    );
    let listed = agent.answer(&mut app, 1).unwrap();
    assert!(
        listed.starts_with("1. Organize imports (source.organizeImports)\nTo make one"),
        "only the source actions: {listed}"
    );
    // The file changes: that list's edits were for the old text.
    let list: u64 = listed
        .split("list ")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    keys(&mut app, "ggOuse x;<Esc>");
    agent.call(
        2,
        "lsp_apply_code_action",
        json!({"path": path.to_str().unwrap(), "list": list, "number": 1}),
    );
    assert_eq!(
        agent.answer(&mut app, 2),
        Err("The file has changed since that list of code actions: ask for them again".into())
    );
    assert_eq!(
        text(&app),
        "use x;\nfn bad() {}\nlet  x =  bad;",
        "untouched"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.action_only, json!(["source"]));
    assert_eq!(
        seen.action_range["start"],
        json!({"line": 0, "character": 0})
    );
    assert_eq!(
        seen.action_range["end"],
        json!({"line": 1, "character": 14}),
        "to the end of the last line"
    );
}

#[cfg(unix)]
#[test]
fn a_server_that_stops_still_answers_the_agent() {
    let (dir, mut app, _, mut agent) = with_agent("fn die() {}\n");
    ready(&mut app);
    let main = dir.path().join("main.rs");
    let said = agent.ask(
        &mut app,
        1,
        "lsp_hover",
        json!({"path": main.to_str().unwrap(), "line": 1, "symbol": "die"}),
    );
    assert_eq!(said, Err("rust-analyzer stopped before answering".into()));
}

#[cfg(unix)]
#[test]
fn the_agent_hears_of_the_edits_an_actions_command_makes() {
    let (dir, mut app, _, mut agent) = with_agent("fine\nalso fine\n");
    ready(&mut app);
    let path = dir.path().join("main.rs");
    let file = path.to_str().unwrap();
    agent.call(1, "lsp_code_actions", json!({"path": file, "line": 2}));
    let listed = agent.answer(&mut app, 1).unwrap();
    let number: usize = listed
        .lines()
        .find(|l| l.contains("Add a note"))
        .and_then(|l| l.split('.').next())
        .unwrap()
        .parse()
        .unwrap();
    let list: u64 = listed
        .split("list ")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    agent.call(
        2,
        "lsp_apply_code_action",
        json!({"path": file, "list": list, "number": number}),
    );
    // The command's server sends the edit as it runs: the agent hears of
    // it, and it's saved, as the file had no unsaved changes.
    let made = agent.answer(&mut app, 2).unwrap();
    let said = made.lines().next().unwrap();
    assert!(
        said.starts_with("Made Add a note in ") && said.ends_with("main.rs"),
        "{made}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "// note\nfine\nalso fine\n"
    );
}

#[cfg(unix)]
#[test]
fn the_agent_gets_code_actions_on_an_empty_line() {
    let (dir, mut app, _, mut agent) = with_agent("fn bad() {}\n\nlet x = bad;\n");
    ready(&mut app);
    let path = dir.path().join("main.rs");
    // Asked at the symbol and for the whole line, which on an empty line
    // are the same place: both answers still have to be told apart.
    agent.call(
        1,
        "lsp_code_actions",
        json!({"path": path.to_str().unwrap(), "line": 2}),
    );
    let listed = agent.answer(&mut app, 1).unwrap();
    assert!(listed.contains("Upper-case the line"), "{listed}");
}

#[cfg(unix)]
#[test]
fn the_agent_asks_who_calls_a_function_and_what_it_calls() {
    let (dir, mut app, _, mut agent) = with_agent("fn bad() {}\nlet  x =  bad;\n");
    ready(&mut app);
    let path = dir.path().join("main.rs");
    let file = path.to_str().unwrap();
    let mut ask = |app: &mut App, call: u64, kind: &str, line: usize| {
        let args = json!({"path": file, "line": line, "kind": kind});
        agent.ask(app, call, "lsp_references", args)
    };
    let callers = ask(&mut app, 1, "callers", 1).unwrap();
    assert!(
        callers.starts_with("1 function calls it:\nmain (function)  "),
        "{callers}"
    );
    assert!(
        callers.ends_with("main.rs:5 (calls it on lines 6, 7)"),
        "{callers}"
    );
    let callees = ask(&mut app, 2, "callees", 1).unwrap();
    assert!(
        callees.starts_with("It calls 2 functions in the project:\n"),
        "{callees}"
    );
    assert!(
        callees.contains("helper (function)  ")
            && callees.contains("main.rs:10 (called on line 1)"),
        "{callees}"
    );
    assert!(
        callees.contains("log (method, impl Logger for Fake)"),
        "{callees}"
    );
    assert!(
        callees.ends_with("\nAnd 1 outside the project (the standard library, dependencies): push"),
        "the library's last, enum members left out: {callees}"
    );
    // A kind that isn't one: said.
    assert_eq!(
        ask(&mut app, 4, "calls", 1),
        Err("kind is one of references, callers, callees, not \"calls\"".into())
    );
    // Not on a function: said, no second request.
    let nothing = ask(&mut app, 3, "callers", 2).unwrap();
    assert!(
        nothing.starts_with("Nothing there to find calls of"),
        "{nothing}"
    );
}

#[cfg(unix)]
#[test]
fn the_agents_question_waits_for_a_server_still_starting() {
    // Asked straight away, before the server's answered `initialize`.
    let (dir, mut app, _, mut agent) = with_agent("fn bad() {}\nlet  x =  bad;\n");
    let path = dir.path().join("main.rs");
    app.update();
    agent.call(
        1,
        "lsp_references",
        json!({"path": path.to_str().unwrap(), "line": 1, "symbol": "bad"}),
    );
    let answer = agent.answer(&mut app, 1);
    assert!(answer.is_ok(), "answered once it was ready: {answer:?}");
}

#[test]
fn moving_a_file_in_the_tree_tells_the_server() {
    let (dir, mut app, seen) = with_server("fn bad() {}\n", true);
    ready(&mut app);
    let helper = dir.path().join("helper.rs");
    fs::write(&helper, "pub fn help() {}\n").unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    app.editor_mut().tree_ask_move(helper, sub).unwrap();
    app.editor_mut().tree_move().unwrap();
    until(&mut app, |_| !seen.lock().unwrap().renamed.is_null());
    let real = fs::canonicalize(dir.path()).unwrap();
    let renamed = seen.lock().unwrap().renamed.clone();
    let uri = |p: &std::path::Path| format!("file://{}", p.display());
    assert_eq!(
        renamed[0]["oldUri"],
        uri(&real.join("helper.rs")),
        "{renamed}"
    );
    assert_eq!(
        renamed[0]["newUri"],
        uri(&real.join("sub/helper.rs")),
        "{renamed}"
    );
}

#[cfg(unix)]
#[test]
fn the_agent_moves_a_file_and_the_server_updates_what_uses_it() {
    let (dir, mut app, seen, mut agent) = with_agent("fn bad() {}\nlet  x =  bad;\n");
    ready(&mut app);
    let helper = dir.path().join("helper.rs");
    fs::write(&helper, "pub fn help() {}\n").unwrap();
    fs::write(dir.path().join("uses.rs"), "use helper;\n").unwrap();
    let to = dir.path().join("sub/helper.rs");
    let mut call = |app: &mut App, id: u64, args: Value| agent.ask(app, id, "move_file", args);
    let said = call(
        &mut app,
        1,
        json!({"from": helper.to_str().unwrap(), "to": to.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        said.starts_with("Moved ") && said.contains("sub/helper.rs"),
        "{said}"
    );
    assert!(
        said.contains("updated for it: 2 changes in 2 files"),
        "{said}"
    );
    // Moved, the "import" updated and saved, the server told.
    assert!(!helper.exists() && to.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("main.rs")).unwrap(),
        "// uses helper at sub/helper.rs\nfn bad() {}\nlet  x =  bad;\n"
    );
    // Both ends as the file system has them (the test's folder is under a
    // link on macOS: /var is /private/var), so the server's paths agree.
    let real = fs::canonicalize(dir.path()).unwrap();
    // The server is told after the move, with nothing to answer: wait
    // until it has read it.
    let deadline = Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().renamed.is_null() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let renamed = seen.lock().unwrap().renamed.clone();
    let uri = |p: &std::path::Path| format!("file://{}", p.display());
    assert_eq!(
        renamed[0]["oldUri"],
        uri(&real.join("helper.rs")),
        "{renamed}"
    );
    assert_eq!(
        renamed[0]["newUri"],
        uri(&real.join("sub/helper.rs")),
        "{renamed}"
    );
    // uses.rs, opened for the edit, is the agent's: it closes after the turn
    // (main.rs is the user's, open already).
    let state = app.editor().agent_editor_state();
    assert!(state.contains("uses.rs (opened for your tools"), "{state}");
    assert!(!state.contains("main.rs (opened for your tools"), "{state}");

    // The server made no edits: said, so what refers to it gets checked.
    let quiet = dir.path().join("sub/noedit.rs");
    let said = call(
        &mut app,
        5,
        json!({"from": to.to_str().unwrap(), "to": quiet.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        said.contains("the language server made no edits for it"),
        "{said}"
    );
    let to = quiet;
    // A folder on the way that's a file.
    let under_file = dir.path().join("main.rs/x.rs");
    let bad = call(
        &mut app,
        6,
        json!({"from": to.to_str().unwrap(), "to": under_file.to_str().unwrap()}),
    );
    assert!(
        bad.as_ref()
            .is_err_and(|e| e.contains("main.rs is a file, not a folder")),
        "{bad:?}"
    );

    // What can't be: said, nothing done.
    let main = dir.path().join("main.rs");
    let taken = call(
        &mut app,
        2,
        json!({"from": to.to_str().unwrap(), "to": main.to_str().unwrap()}),
    );
    assert!(
        taken.as_ref().is_err_and(|e| e.contains("already exists")),
        "{taken:?}"
    );
    // A file no server's for: just moved.
    let notes = dir.path().join("notes.txt");
    fs::write(&notes, "notes\n").unwrap();
    let moved = dir.path().join("docs/notes.txt");
    let said = call(
        &mut app,
        4,
        json!({"from": notes.to_str().unwrap(), "to": moved.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        said.starts_with("Moved ") && !said.contains("updated"),
        "{said}"
    );
    assert!(moved.exists() && !notes.exists());
    let missing = call(&mut app, 3, json!({"from": "no/such.rs", "to": "x.rs"}));
    assert!(
        missing
            .as_ref()
            .is_err_and(|e| e.starts_with("No such file")),
        "{missing:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_agents_fix_in_a_file_not_shown_isnt_called_unsaved() {
    let (dir, mut app, _, mut agent) = with_agent("fn bad() {}\nlet  x =  bad;\n");
    ready(&mut app);
    until(&mut app, |a| !a.editor().diagnostics(doc(a)).is_empty());
    let path = dir.path().join("main.rs");
    let file = path.to_str().unwrap();
    // The user's moved on to another file: main.rs is in the background.
    let other = dir.path().join("notes.txt");
    fs::write(&other, "notes\n").unwrap();
    keys(&mut app, &format!(":e {}<CR>", other.display()));
    agent.call(
        1,
        "lsp_code_actions",
        json!({"path": file, "line": 2, "symbol": "bad"}),
    );
    let listed = agent.answer(&mut app, 1).unwrap();
    let list: u64 = listed
        .split("list ")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    agent.call(
        2,
        "lsp_apply_code_action",
        json!({"path": file, "list": list, "number": 1}),
    );
    agent.answer(&mut app, 2).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "fn bad() {}\nlet  x =  good;\n"
    );
    let said = app.editor().message().unwrap().text.clone();
    assert!(!said.contains(":wa"), "it was saved: {said}");
}

#[test]
fn after_the_agents_edit_its_new_problems_are_told() {
    let (dir, mut app, _) = with_server("fine\n", true);
    ready(&mut app);
    let path = dir.path().join("main.rs");
    let id = doc(&app);
    // The report on the file as it opened, first.
    until(&mut app, |a| a.editor().diagnostics_state(id).version > 0);
    let version = app.editor().diagnostics_state(id).version;
    assert!(
        app.editor().diagnostics_state(id).server.is_some(),
        "a server has it open"
    );
    let (done, edited) = app
        .editor_mut()
        .agent_edit_buffer(
            path.to_str().unwrap(),
            &[("fine".into(), "bad".into(), false)],
        )
        .unwrap();
    assert_eq!(edited, [id]);
    assert!(done.ends_with("1 replacement"), "{done}");
    until(&mut app, |a| {
        a.editor().diagnostics_state(id).version > version
    });
    let problems = app.editor().agent_problems(&[id]);
    assert!(
        problems.contains("1:1  error: bad word (fake)"),
        "{problems}"
    );
}

#[test]
fn checks_on_save_are_placed_where_their_lines_are_now() {
    let (_dir, mut app, _) = with_server("// oncheck\nlet x = wrong;\n", true);
    ready(&mut app);
    until(&mut app, |a| diagnostic_starts(a).contains(&(1, 8)));
    // Two lines put in above, not saved: the server says it again at the
    // saved file's place (line 2), which is line 4 now.
    keys(&mut app, "ggO// one<CR>// two<Esc>");
    until(&mut app, |a| {
        a.editor().document.buffer().line_count() == 4 && diagnostic_starts(a).contains(&(3, 8))
    });
    // And again on the next change: still there.
    keys(&mut app, "ggA!<Esc>");
    app.update();
    until(&mut app, |a| {
        a.editor().document.buffer().line(0).ends_with('!')
    });
    std::thread::sleep(Duration::from_millis(50));
    app.update();
    assert!(
        diagnostic_starts(&app).contains(&(3, 8)),
        "{:?}",
        diagnostic_starts(&app)
    );
    assert!(
        !diagnostic_starts(&app).contains(&(1, 8)),
        "not back at the saved place"
    );
}
