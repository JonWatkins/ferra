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
    /// The options a formatting request came with.
    format_options: Value,
    /// The range the last code action request was for, and ferra's
    /// answer to the server's `workspace/applyEdit`.
    action_range: Value,
    applied: Value,
    /// The context the last signature help request came with, and the
    /// queries for the project's symbols.
    signature_context: Value,
    symbol_queries: Vec<String>,
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
                reply(
                    &mut output,
                    json!({"capabilities": {
                        "positionEncoding": encoding, "textDocumentSync": 2,
                        "hoverProvider": features, "definitionProvider": features,
                        "referencesProvider": features,
                        "completionProvider": {"triggerCharacters": ["."]},
                        "documentFormattingProvider": features,
                        "renameProvider": features,
                        "documentSymbolProvider": features,
                        "workspaceSymbolProvider": features,
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
                seen.lock().unwrap().text = params["textDocument"]["text"].as_str().unwrap().into();
                publish(&mut output, &params["textDocument"]["uri"], seen, utf8);
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
                let uri = &params["textDocument"]["uri"];
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
            "workspace/executeCommand" => {
                reply(&mut output, Value::Null);
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
                let result = open.map(|at| {
                    let commas = before[at..].matches(',').count();
                    json!({"signatures": [{
                        "label": "call(first: u8, second: u8)",
                        "documentation": "Calls it.",
                        "parameters": [
                            {"label": "first: u8", "documentation": "The first."},
                            {"label": [16, 26]},
                        ],
                    }], "activeSignature": 0, "activeParameter": commas})
                });
                reply(&mut output, result.into());
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
    mut app: App,
    content: &str,
    utf8: bool,
) -> (tempfile::TempDir, App, Arc<Mutex<Seen>>) {
    let dir = tempfile::tempdir().unwrap();
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
        app.update_lsp();
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
    app.update_lsp();
    let message = app.editor().message().unwrap().text.clone();
    assert_eq!(
        message,
        "rust-analyzer not found: install it for rust language features (see :lsp)"
    );
    keys(&mut app, ":echo<CR>");
    app.open(&dir.path().join("b.rs"));
    app.update_lsp();
    assert!(
        !app.editor().message().unwrap().text.contains("not found"),
        "once"
    );
    keys(&mut app, ":lsp<CR>");
    let path = std::env::var("PATH").unwrap_or_default();
    assert_eq!(
        app.editor().message().unwrap().text,
        format!("rust-analyzer: not found on PATH  ·  PATH={path}")
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
        format!("fake-ls (rust): running in {root}")
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
    app.update_lsp();
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
    app.update_lsp();
    until(&mut app, |a| a.editor().message().is_some());
    assert_eq!(
        app.editor().message().unwrap().text,
        "No hover information here"
    );
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
    app.update_lsp();
    assert_eq!(
        app.editor().message().unwrap().text,
        "rust-analyzer doesn't offer hover information"
    );
    keys(&mut app, ":e notes.txt<CR>gd");
    app.update_lsp();
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
    app.update_lsp();
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
    app.update_lsp();
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
    app.update_lsp();
    assert!(menu(&app).is_empty(), "`;` isn't one of the server's");
    keys(&mut app, "x.");
    until(&mut app, |a| menu(a).len() == 6);
    keys(&mut app, "fi");
    app.update_lsp();
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
    app.update_lsp();
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
fn f2_starts_a_rename_of_the_word_under_the_cursor() {
    let (_dir, mut app, _) = with_server("let bad = 1;\n", true);
    ready(&mut app);
    keys(&mut app, "w<F2>");
    assert_eq!(app.editor().command_line.input(), "rename bad");
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
    app.update_lsp();
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
        app.update_lsp();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        diagnostic_starts(&app),
        [(0, 2)],
        "not the old list's (0, 0)"
    );
}

fn active_parameter(app: &App) -> Option<String> {
    let help = app.editor().signature_help()?;
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
