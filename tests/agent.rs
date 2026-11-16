//! The coding agent over ACP, end to end, with a fake agent running in this
//! process: starting it, a turn streaming in, a file read from its buffer,
//! a permission asked and answered, a turn cancelled, and an agent that
//! isn't installed; and its sidebar: typed in, drawn, clicked, resized.

mod common;

use std::fs;
use std::io::{self, BufReader, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use common::{app_with_config, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::agent::client::{Streams, read_message, write_message};
use ferra::agent::{AgentConfig, Spawner};
use ferra::editor::Mode;
use ferra::editor::agent::{AgentStatus, ChatEntry, ToolStatus};
use serde_json::{Value, json};

/// An agent in a thread, answering as a real one would (see `serve`).
struct FakeAgent;

impl Spawner for FakeAgent {
    fn spawn(&self, _: &AgentConfig, _: &Path) -> io::Result<Streams> {
        let (client_reads, agent_writes) = io::pipe()?;
        let (agent_reads, client_writes) = io::pipe()?;
        thread::spawn(move || serve(BufReader::new(agent_reads), agent_writes));
        Ok(Streams {
            reader: Box::new(BufReader::new(client_reads)),
            writer: Box::new(client_writes),
            child: None,
        })
    }
}

fn send(out: &mut impl Write, message: Value) {
    let _ = write_message(out, &message);
}

fn update(out: &mut impl Write, update: Value) {
    send(
        out,
        json!({"jsonrpc": "2.0", "method": "session/update",
               "params": {"sessionId": "s1", "update": update}}),
    );
}

fn text(out: &mut impl Write, text: &str) {
    update(
        out,
        json!({"sessionUpdate": "agent_message_chunk",
               "content": {"type": "text", "text": text}}),
    );
}

/// Reads until the answer to request `id` (ferra's answer to ours).
fn answer_to(input: &mut impl io::BufRead, id: i64) -> Value {
    while let Ok(Some(message)) = read_message(input) {
        if message["id"] == json!(id) && message.get("method").is_none() {
            return message;
        }
    }
    Value::Null
}

fn serve(mut input: BufReader<io::PipeReader>, mut out: io::PipeWriter) {
    while let Ok(Some(message)) = read_message(&mut input) {
        let id = message["id"].clone();
        let reply = |out: &mut io::PipeWriter, result: Value| {
            send(out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
        };
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                assert_eq!(message["params"]["protocolVersion"], 1);
                assert_eq!(
                    message["params"]["clientCapabilities"]["fs"]["readTextFile"],
                    true
                );
                reply(
                    &mut out,
                    json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}),
                );
            }
            "session/new" => {
                // ferra's tools, to be started through its bridge.
                let tools = &message["params"]["mcpServers"][0];
                assert_eq!(tools["name"], "ferra");
                assert_eq!(tools["args"][0], "--mcp-bridge");
                reply(
                    &mut out,
                    json!({"sessionId": "s1", "modes": {"currentModeId": "default",
                           "availableModes": [{"id": "default", "name": "Default"},
                                              {"id": "acceptEdits", "name": "Accept edits"}]}}),
                );
            }
            "session/prompt" => {
                let blocks = message["params"]["prompt"].as_array().cloned().unwrap_or_default();
                // What was typed: the text that isn't the editor's note.
                let typed = |b: &&Value| {
                    b["type"] == "text"
                        && !b["text"].as_str().unwrap_or_default().starts_with("<editor-context>")
                };
                let asked = blocks
                    .iter()
                    .find(typed)
                    .and_then(|b| b["text"].as_str())
                    .unwrap_or_default()
                    .to_string();
                match asked.split_whitespace().next().unwrap_or_default() {
                    "hello" => {
                        text(&mut out, "Hi ");
                        text(&mut out, "there.");
                        update(
                            &mut out,
                            json!({"sessionUpdate": "plan", "entries": [
                                {"content": "Say hi", "priority": "high", "status": "completed"}]}),
                        );
                    }
                    "read" => {
                        let path = asked.split_whitespace().nth(1).unwrap_or_default();
                        send(
                            &mut out,
                            json!({"jsonrpc": "2.0", "id": 100, "method": "fs/read_text_file",
                                   "params": {"sessionId": "s1", "path": path, "line": 2, "limit": 1}}),
                        );
                        let answer = answer_to(&mut input, 100);
                        let content = answer["result"]["content"].as_str().unwrap_or("?");
                        text(&mut out, &format!("Line 2 is {}", content.trim()));
                    }
                    "permission" => {
                        update(
                            &mut out,
                            json!({"sessionUpdate": "tool_call", "toolCallId": "t1",
                                   "title": "Run cargo test", "kind": "execute", "status": "pending"}),
                        );
                        send(
                            &mut out,
                            json!({"jsonrpc": "2.0", "id": 101, "method": "session/request_permission",
                                   "params": {"sessionId": "s1",
                                              "toolCall": {"toolCallId": "t1", "title": "Run cargo test"},
                                              "options": [
                                                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                                                {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]}}),
                        );
                        let answer = answer_to(&mut input, 101);
                        let chosen = answer["result"]["outcome"]["optionId"]
                            .as_str()
                            .unwrap_or("none");
                        update(
                            &mut out,
                            json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1",
                                   "status": "completed",
                                   "content": [{"type": "content",
                                                "content": {"type": "text", "text": format!("chose {chosen}")}}]}),
                        );
                    }
                    "blocks" => {
                        // What came: each block's type, the note, the links.
                        let kinds: Vec<&str> = blocks.iter().filter_map(|b| b["type"].as_str()).collect();
                        text(&mut out, &kinds.join(","));
                        for block in &blocks {
                            if let Some(note) = block["text"]
                                .as_str()
                                .filter(|t| t.starts_with("<editor-context>"))
                            {
                                text(&mut out, &format!("|{note}"));
                            }
                            if let Some(uri) = block["uri"].as_str() {
                                text(&mut out, &format!("|{uri}"));
                            }
                        }
                    }
                    "slow" => {
                        // Until it's cancelled.
                        while let Ok(Some(next)) = read_message(&mut input) {
                            if next["method"] == "session/cancel" {
                                break;
                            }
                        }
                        reply(&mut out, json!({"stopReason": "cancelled"}));
                        continue;
                    }
                    _ => {}
                }
                reply(&mut out, json!({"stopReason": "end_turn"}));
            }
            _ => {}
        }
    }
}

fn app() -> App {
    let mut app =
        app_with_config("[editor]\nagent = \"fake\"\n[agents.fake]\ncommand = \"fake-agent\"\n");
    app.agents_mut().set_spawner(Box::new(FakeAgent));
    app
}

/// Lets the agent's messages through until `done`, or two seconds pass.
fn until(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        app.update_agents();
        if done(app) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out: {:?}", app.editor().agent_chat().entries);
}

fn said(app: &App) -> String {
    app.editor()
        .agent_chat()
        .entries
        .iter()
        .filter_map(|e| match e {
            ChatEntry::Agent(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn ready(app: &App) -> bool {
    app.editor().agent_chat().status == AgentStatus::Ready
}

#[test]
fn a_turn_streams_in_and_ends() {
    let mut app = app();
    keys(&mut app, ":agent_ask hello there<CR>");
    until(&mut app, |a| ready(a) && said(a) == "Hi there.");
    let chat = app.editor().agent_chat();
    assert_eq!(chat.agent_info.as_deref(), Some("fake 1.0"));
    assert_eq!(chat.mode.as_deref(), Some("default"));
    assert_eq!(chat.modes.len(), 2);
    assert!(matches!(&chat.entries[0], ChatEntry::User(t) if t == "hello there"));
    assert!(
        chat.entries
            .iter()
            .any(|e| matches!(e, ChatEntry::Plan(p) if p[0].status == "completed"))
    );
    // The sidebar's shown.
    assert!(app.editor().agent_sidebar_shown());
}

#[test]
fn the_sidebar_is_typed_in_and_drawn() {
    let mut app = app();
    keys(&mut app, ":agent<CR>");
    assert_eq!(app.editor().mode(), Mode::Agent);
    until(&mut app, ready);
    keys(&mut app, "hello");
    keys(&mut app, "<C-j>world");
    assert_eq!(app.editor().agent_chat().input.text(), "hello\nworld");
    keys(&mut app, "<C-u><C-a>hello there<CR>");
    until(&mut app, |a| ready(a) && said(a) == "Hi there.");
    assert_eq!(app.editor().agent_chat().input.text(), "");
    let screen = render(&mut app, 100, 30);
    let rows = &screen.rows;
    // Right of the windows, from the top: the header, the chat.
    let at = rows[0].find("fake 1.0").expect("the header");
    assert!(rows[0].contains("Default ▾"), "{rows:#?}");
    assert!(
        rows.iter().any(|r| r[at..].contains("› hello there")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r[at..].contains("Hi there.")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r[at..].contains("☑ Say hi")),
        "{rows:#?}"
    );
    assert!(rows[27].contains("⏎ send"), "{rows:#?}");
    // The cursor's in the box.
    assert_eq!(screen.cursor.1, 24);
    // Esc goes back to the text; the sidebar stays.
    keys(&mut app, "<Esc>");
    assert_ne!(app.editor().mode(), Mode::Agent);
    assert!(app.editor().agent_sidebar_shown());
    keys(&mut app, ":agent_sidebar<CR>");
    assert!(!app.editor().agent_sidebar_shown());
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.handle_mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}

fn click(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
}

/// Where `text` is on the screen.
fn find(app: &mut App, text: &str) -> (u16, u16) {
    let rows = render(app, 100, 30).rows;
    for (y, row) in rows.iter().enumerate() {
        if let Some(i) = row.find(text) {
            return (row[..i].chars().count() as u16, y as u16);
        }
    }
    panic!("{text} isn't shown: {rows:#?}");
}

#[test]
fn an_option_is_clicked_and_the_edge_dragged() {
    let mut app = app();
    keys(&mut app, ":agent<CR>permission<CR>");
    until(&mut app, |a| a.editor().agent_chat().permission.is_some());
    let (x, y) = find(&mut app, "2 Reject");
    click(&mut app, x + 1, y);
    until(&mut app, |a| {
        a.editor().agent_chat().entries.iter().any(|e| {
            matches!(e, ChatEntry::Tool(t) if t.status == ToolStatus::Done && t.text == "chose reject")
        })
    });
    // The tool call opens to show what it said.
    let (x, y) = find(&mut app, "Run cargo test ▸");
    click(&mut app, x, y);
    find(&mut app, "chose reject");
    // The edge, dragged left, widens it.
    let before = app.editor().options.agent_width;
    let edge = 100 - before.min(50) - 1;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), edge, 5);
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        edge - 4,
        5,
    );
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), edge - 4, 5);
    assert_eq!(app.editor().options.agent_width, before.min(50) + 4);
}

#[test]
fn files_are_read_from_their_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let mut app = app();
    app.open(&path);
    // Unsaved: what the agent reads is the buffer's.
    keys(&mut app, "jcwTWO<Esc>");
    let path = fs::canonicalize(&path).unwrap();
    keys(&mut app, &format!(":agent_ask read {}<CR>", path.display()));
    until(&mut app, |a| said(a).starts_with("Line 2"));
    assert_eq!(said(&app), "Line 2 is TWO");
}

#[test]
fn permission_is_asked_and_the_answer_goes_back() {
    let mut app = app();
    keys(&mut app, ":agent<CR>permission<CR>");
    until(&mut app, |a| a.editor().agent_chat().permission.is_some());
    let rows = render(&mut app, 100, 30).rows.join("\n");
    assert!(
        rows.contains(" 1 Allow ") && rows.contains(" 2 Reject "),
        "{rows}"
    );
    keys(&mut app, "2");
    until(&mut app, |a| {
        a.editor().agent_chat().entries.iter().any(|e| {
            matches!(e, ChatEntry::Tool(t) if t.status == ToolStatus::Done && t.text == "chose reject")
        })
    });
}

#[test]
fn a_turn_is_cancelled() {
    let mut app = app();
    keys(&mut app, ":agent<CR>slow<CR>");
    until(&mut app, |a| {
        a.editor().agent_chat().status == AgentStatus::Busy
    });
    keys(&mut app, "<C-c>");
    until(&mut app, ready);
    assert!(
        app.editor()
            .agent_chat()
            .entries
            .iter()
            .any(|e| matches!(e, ChatEntry::Notice(n) if n == "Stopped"))
    );
}

#[test]
fn an_agent_that_isnt_installed_says_so() {
    let mut app = app_with_config(
        "[editor]\nagent = \"nope\"\n[agents.nope]\ncommand = \"ferra-no-such-agent\"\n",
    );
    keys(&mut app, ":agent<CR>");
    app.update_agents();
    assert!(matches!(
        &app.editor().agent_chat().status,
        AgentStatus::Stopped(why) if why.contains("ferra-no-such-agent isn't installed")
    ));
    keys(&mut app, "<Esc>:agent_ask hi<CR>");
    app.update_agents();
    assert!(matches!(
        app.editor().agent_chat().status,
        AgentStatus::Stopped(_)
    ));
}

#[test]
fn messages_say_whats_open_and_carry_the_files_mentioned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "one\ntwo\n").unwrap();
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    let mut app = app();
    app.open(&path);
    keys(&mut app, "j");
    // Mentions are found from the project's folder (this test's).
    let mentioned = dir.path().join("b.txt");
    keys(
        &mut app,
        &format!(":agent_ask blocks @{} please<CR>", mentioned.display()),
    );
    until(&mut app, |a| ready(a) && said(a).contains("|file:"));
    let said = said(&app);
    assert!(said.starts_with("text,text,resource_link"), "{said}");
    assert!(said.contains("a.txt:2:1"), "the file and cursor: {said}");
    assert!(said.contains("b.txt"), "{said}");
}

#[test]
fn at_in_the_box_picks_a_file_to_mention() {
    let mut app = app();
    keys(&mut app, ":agent<CR>see @");
    assert_eq!(app.editor().mode(), Mode::Picker);
    assert_eq!(app.editor().picker().map(|p| p.title()), Some("Mention a file"));
    // None picked: back to the box, the @ kept.
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Agent);
    assert_eq!(app.editor().agent_chat().input.text(), "see @");
}

#[cfg(unix)]
#[test]
fn the_editors_tools_are_served_to_the_agent() {
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "one\ntwo\n").unwrap();
    let other = dir.path().join("b.txt");
    fs::write(&other, "x\ny\nz\n").unwrap();
    let mut app = app();
    app.open(&path);
    keys(&mut app, ":agent<CR>");
    until(&mut app, ready);
    // Talked to as the bridge does: JSON-RPC lines over the socket.
    let socket = app.agents_mut().mcp_socket().expect("served").to_path_buf();
    let stream = UnixStream::connect(&socket).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let (sender, answers) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        while let Ok(Some(message)) = read_message(&mut reader) {
            let _ = sender.send(message);
        }
    });
    let mut ask = |app: &mut App, id: i64, method: &str, params: Value| -> Value {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        write_message(&mut writer, &request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            app.update_agents();
            if let Ok(answer) = answers.try_recv() {
                assert_eq!(answer["id"], id);
                return answer["result"].clone();
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("no answer to {method}");
    };
    let started = ask(&mut app, 1, "initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(started["serverInfo"]["name"], "ferra");
    assert_eq!(started["protocolVersion"], "2025-06-18");
    let listed = ask(&mut app, 2, "tools/list", json!({}));
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"editor_state") && names.contains(&"lsp_definition"), "{names:?}");

    let call = |name: &str, arguments: Value| json!({"name": name, "arguments": arguments});
    let state = ask(&mut app, 3, "tools/call", call("editor_state", json!({})));
    let state = state["content"][0]["text"].as_str().unwrap().to_string();
    assert!(state.contains("a.txt (cursor at line 1, column 1)"), "{state}");
    let problems = ask(&mut app, 4, "tools/call", call("get_diagnostics", json!({})));
    assert_eq!(problems["content"][0]["text"], "No diagnostics");
    let opened = ask(
        &mut app,
        5,
        "tools/call",
        call("open_file", json!({"path": other, "line": 2})),
    );
    assert_eq!(opened["isError"], false, "{opened}");
    assert!(app.editor().document.name().ends_with("b.txt"));
    assert_eq!(app.editor().document.cursor().row, 1);
    let wrong = ask(&mut app, 6, "tools/call", call("nope", json!({})));
    assert_eq!(wrong["isError"], true);
}
