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
}

struct FakeServer {
    utf8: bool,
    seen: Arc<Mutex<Seen>>,
}

impl Spawner for FakeServer {
    fn spawn(&self, _: &ServerConfig, _: &Path) -> io::Result<Streams> {
        let (client_reads, server_writes) = io::pipe()?;
        let (server_reads, client_writes) = io::pipe()?;
        let seen = Arc::clone(&self.seen);
        let utf8 = self.utf8;
        std::thread::spawn(move || serve(server_reads, server_writes, utf8, &seen));
        Ok(Streams {
            reader: Box::new(BufReader::new(client_reads)),
            writer: Box::new(client_writes),
            child: None,
        })
    }
}

fn serve(input: io::PipeReader, mut output: io::PipeWriter, utf8: bool, seen: &Mutex<Seen>) {
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
                reply(
                    &mut output,
                    json!({"capabilities": {"positionEncoding": encoding, "textDocumentSync": 2}}),
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
    until(&mut app, |a| diagnostic_starts(a).len() == 2);
    // "é finebad" and "a bad", as the server's copy says too.
    assert_eq!(text(&app), "é finebad\n\na bad");
    assert_eq!(diagnostic_starts(&app), [(0, 6), (2, 2)]);
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
