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
    write_message(
        output,
        &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                "params": {"uri": uri, "diagnostics": diagnostics}}),
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
