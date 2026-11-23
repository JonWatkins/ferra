//! The coding agent over ACP, end to end, with a fake agent running in this
//! process: starting it, a turn streaming in, a file read from its buffer,
//! a permission asked and answered, a turn cancelled, and an agent that
//! isn't installed; and its sidebar: typed in, drawn, clicked, resized;
//! its slash commands, settings, context used and new sessions.

mod common;

use std::cell::RefCell;
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

thread_local! {
    /// The fake agent's session (each runs in its own thread).
    static SESSION: RefCell<String> = RefCell::new("s1".into());
}

fn update(out: &mut impl Write, update: Value) {
    let session = SESSION.with(|s| s.borrow().clone());
    update_in(out, &session, update);
}

fn update_in(out: &mut impl Write, session: &str, update: Value) {
    send(
        out,
        json!({"jsonrpc": "2.0", "method": "session/update",
               "params": {"sessionId": session, "update": update}}),
    );
}

/// The fake's settings: a model and an effort.
fn config(model: &str) -> Value {
    json!([
        {"id": "mode", "name": "Mode", "category": "mode", "type": "select",
         "currentValue": "default", "options": [{"value": "default", "name": "Default"}]},
        {"id": "model", "name": "Model", "category": "model", "type": "select",
         "currentValue": model, "options": [
            {"value": "opus", "name": "Opus"}, {"value": "sonnet", "name": "Sonnet"}]},
        {"id": "effort", "name": "Effort", "category": "thought_level", "type": "select",
         "currentValue": "high", "options": [
            {"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]},
    ])
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
                    json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"},
                           "agentCapabilities": {"promptCapabilities": {"image": true},
                                                 "sessionCapabilities": {"close": {}}}}),
                );
            }
            "session/new" => {
                // ferra's tools, to be started through its bridge.
                let tools = &message["params"]["mcpServers"][0];
                assert_eq!(tools["name"], "ferra");
                assert_eq!(tools["args"][0], "--mcp-bridge");
                // s1, then s2 after a /clear...
                let session = SESSION.with(|s| {
                    let mut s = s.borrow_mut();
                    if message["id"] != json!(2) {
                        let n: u32 = s[1..].parse().unwrap();
                        *s = format!("s{}", n + 1);
                    }
                    s.clone()
                });
                reply(
                    &mut out,
                    json!({"sessionId": session, "modes": {"currentModeId": "default",
                           "availableModes": [{"id": "default", "name": "Default"},
                                              {"id": "acceptEdits", "name": "Accept edits"}]},
                           "configOptions": config("opus")}),
                );
                update(
                    &mut out,
                    json!({"sessionUpdate": "available_commands_update", "availableCommands": [
                        {"name": "review", "description": "Review a branch",
                         "input": {"hint": "[branch]"}},
                        {"name": "compact", "description": "Compact the conversation"},
                        {"name": "clear", "description": "The agent's own clear"}]}),
                );
            }
            "session/close" => {
                assert_eq!(message["params"]["sessionId"], "s1");
                reply(&mut out, json!({}));
            }
            "session/set_config_option" => {
                let params = &message["params"];
                assert_eq!(params["configId"], "model");
                reply(
                    &mut out,
                    json!({"configOptions": config(params["value"].as_str().unwrap())}),
                );
            }
            "session/prompt" => {
                let blocks = message["params"]["prompt"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                // What was typed: the text that isn't the editor's note.
                let typed = |b: &&Value| {
                    b["type"] == "text"
                        && !b["text"]
                            .as_str()
                            .unwrap_or_default()
                            .starts_with("<editor-context>")
                };
                let asked = blocks
                    .iter()
                    .find(typed)
                    .and_then(|b| b["text"].as_str())
                    .unwrap_or_default()
                    .to_string();
                // A slash command runs only when it's all that's sent, as in
                // Claude Code (with more, it goes to the model as a message).
                let first = blocks[0]["text"].as_str().unwrap_or_default();
                if first.starts_with('/') && blocks.len() == 1 {
                    text(&mut out, &format!("ran {first}"));
                    reply(&mut out, json!({"stopReason": "end_turn"}));
                    continue;
                }
                match asked.split_whitespace().next().unwrap_or_default() {
                    "session" => {
                        let session = SESSION.with(|s| s.borrow().clone());
                        text(&mut out, &format!("in {session}"));
                    }
                    "usage" => {
                        // A subagent's session: not shown.
                        update_in(
                            &mut out,
                            "sub",
                            json!({"sessionUpdate": "agent_message_chunk",
                                   "content": {"type": "text", "text": "from a subagent"}}),
                        );
                        update(
                            &mut out,
                            json!({"sessionUpdate": "usage_update", "used": 42000, "size": 200000,
                                   "cost": {"amount": 0.31, "currency": "USD"}}),
                        );
                        text(&mut out, "counted");
                    }
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
                        let kinds: Vec<&str> =
                            blocks.iter().filter_map(|b| b["type"].as_str()).collect();
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
                            if block["type"] == "image" {
                                let (mime, data) = (&block["mimeType"], &block["data"]);
                                text(
                                    &mut out,
                                    &format!(
                                        "|{} {}",
                                        mime.as_str().unwrap(),
                                        data.as_str().unwrap()
                                    ),
                                );
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
        app.update();
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
    // Its settings on the box's last row, lined up with what's typed.
    assert!(rows[24].contains("│ Opus ▾  High effort ▾"), "{rows:#?}");
    // The cursor's in the box.
    assert_eq!(screen.cursor.1, 22);
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
    app.update();
    assert!(matches!(
        &app.editor().agent_chat().status,
        AgentStatus::Stopped(why) if why.contains("ferra-no-such-agent isn't installed")
    ));
    keys(&mut app, "<Esc>:agent_ask hi<CR>");
    app.update();
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
    assert_eq!(
        app.editor().picker().map(|p| p.title()),
        Some("Mention a file")
    );
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
            app.update();
            if let Ok(answer) = answers.try_recv() {
                assert_eq!(answer["id"], id);
                return answer["result"].clone();
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("no answer to {method}");
    };
    let started = ask(
        &mut app,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    );
    assert_eq!(started["serverInfo"]["name"], "ferra");
    assert_eq!(started["protocolVersion"], "2025-06-18");
    let listed = ask(&mut app, 2, "tools/list", json!({}));
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        names.contains(&"editor_state") && names.contains(&"lsp_definition"),
        "{names:?}"
    );

    let call = |name: &str, arguments: Value| json!({"name": name, "arguments": arguments});
    let state = ask(&mut app, 3, "tools/call", call("editor_state", json!({})));
    let state = state["content"][0]["text"].as_str().unwrap().to_string();
    assert!(
        state.contains("a.txt (cursor at line 1, column 1)"),
        "{state}"
    );
    let problems = ask(
        &mut app,
        4,
        "tools/call",
        call("get_diagnostics", json!({})),
    );
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

    // Reading and editing through the editor: unsaved changes are seen;
    // an edit is made in the buffer, undoable, saved when it was clean.
    let path_text = path.to_str().unwrap();
    keys(&mut app, &format!(":e {path_text}<CR>ggAX<Esc>"));
    let read = ask(
        &mut app,
        7,
        "tools/call",
        call("read_buffer", json!({"path": path_text})),
    );
    let read = read["content"][0]["text"].as_str().unwrap().to_string();
    assert!(read.contains("unsaved changes"), "{read}");
    assert!(read.contains("     1\toneX\n     2\ttwo"), "{read}");
    let edit = |old: &str, new: &str| {
        call(
            "edit_buffer",
            json!({"path": other, "old_text": old, "new_text": new}),
        )
    };
    let twice = ask(
        &mut app,
        8,
        "tools/call",
        call(
            "edit_buffer",
            json!({"path": path_text, "old_text": "o", "new_text": "0"}),
        ),
    );
    assert!(
        twice["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("2 times"),
        "{twice}"
    );
    let edited = ask(&mut app, 9, "tools/call", edit("y\n", "why\n"));
    let edited = edited["content"][0]["text"].as_str().unwrap().to_string();
    assert!(
        edited.ends_with("b.txt: 1 replacement"),
        "saved, nothing else said: {edited}"
    );
    assert_eq!(fs::read_to_string(&other).unwrap(), "x\nwhy\nz\n");
    // The file's last line, with its newline (as the file has it).
    let last = ask(&mut app, 18, "tools/call", edit("z\n", "zz\n"));
    assert_eq!(last["isError"], false, "{last}");
    assert_eq!(fs::read_to_string(&other).unwrap(), "x\nwhy\nzz\n");
    let back = ask(&mut app, 19, "tools/call", edit("zz", "z"));
    assert_eq!(back["isError"], false, "{back}");
    let missing = ask(&mut app, 10, "tools/call", edit("nowhere", "x"));
    assert_eq!(missing["isError"], true);
    // a.txt, with the user's change unsaved: edited, but left unsaved.
    let mine = ask(
        &mut app,
        11,
        "tools/call",
        call(
            "edit_buffer",
            json!({"path": path_text, "old_text": "two", "new_text": "2"}),
        ),
    );
    let mine = mine["content"][0]["text"].as_str().unwrap().to_string();
    assert!(mine.contains("Not saved"), "{mine}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    assert_eq!(common::text(&app), "oneX\n2");
    keys(&mut app, "u");
    assert_eq!(
        common::text(&app),
        "oneX\ntwo",
        "the agent's edit undone alone"
    );

    // Several changes at once: all checked first, then made as one.
    let edits = |list: Value| call("edit_buffer", json!({"path": other, "edits": list}));
    let bad = ask(
        &mut app,
        12,
        "tools/call",
        edits(json!([
        {"old_text": "x", "new_text": "ex"}, {"old_text": "nowhere", "new_text": "?"}])),
    );
    assert!(
        bad["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("edit 2: "),
        "{bad}"
    );
    assert_eq!(
        fs::read_to_string(&other).unwrap(),
        "x\nwhy\nz\n",
        "none made"
    );
    let both = ask(
        &mut app,
        13,
        "tools/call",
        edits(json!([
        {"old_text": "x", "new_text": "ex"}, {"old_text": "z", "new_text": "zed"}])),
    );
    assert!(
        both["content"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("2 replacements"),
        "{both}"
    );
    // Overlapping old_texts are refused even if what changes in them doesn't.
    let overlap = ask(
        &mut app,
        22,
        "tools/call",
        edits(json!([
        {"old_text": "ex\nw", "new_text": "eX\nw"}, {"old_text": "x\nwhy", "new_text": "x\nwhY"}])),
    );
    assert!(
        overlap["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("overlap"),
        "{overlap}"
    );
    assert_eq!(fs::read_to_string(&other).unwrap(), "ex\nwhy\nzed\n");

    // A new file; not over an old one.
    let new = dir.path().join("sub/c.txt");
    let made = ask(
        &mut app,
        14,
        "tools/call",
        call("create_file", json!({"path": new, "text": ""})),
    );
    assert_eq!(made["isError"], false, "{made}");
    let again = ask(
        &mut app,
        15,
        "tools/call",
        call("create_file", json!({"path": new, "text": "x"})),
    );
    assert_eq!(again["isError"], true);
    let empty = ask(
        &mut app,
        16,
        "tools/call",
        call("read_buffer", json!({"path": new})),
    );
    assert_eq!(empty["content"][0]["text"], "(empty file)");

    // The buffers it opened go when its turn ends; those with tabs stay
    // (b.txt was opened for the user, above).
    assert!(app.editor().find_open_path(&new).is_some());
    keys(&mut app, ":agent_ask hello<CR>");
    until(&mut app, |a| ready(a) && said(a).contains("Hi there."));
    assert!(app.editor().find_open_path(&new).is_none(), "closed");
    assert!(app.editor().find_open_path(&other).is_some(), "a tab: kept");
    assert!(app.editor().find_open_path(&path).is_some(), "shown: kept");
    let closed = ask(
        &mut app,
        17,
        "tools/call",
        call("close_file", json!({"path": path})),
    );
    assert!(
        closed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unsaved"),
        "{closed}"
    );

    // Saved, and in two splits: left for the user (not closed in both).
    keys(&mut app, ":w<CR>:vsplit<CR>");
    let state = ask(&mut app, 20, "tools/call", call("editor_state", json!({})));
    assert!(
        state["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("2 splits, showing: "),
        "{state}"
    );
    let split = ask(
        &mut app,
        21,
        "tools/call",
        call("close_file", json!({"path": path})),
    );
    assert!(
        split["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("another split"),
        "{split}"
    );
    assert!(
        app.editor().document.name().ends_with("a.txt"),
        "still shown"
    );
}

#[test]
fn images_dropped_or_pasted_go_with_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let shot = dir.path().join("Screen Shot 1.png");
    fs::write(&shot, b"PNG1").unwrap();
    let notes = dir.path().join("notes.txt");
    fs::write(&notes, "n").unwrap();
    let clipboard = ferra::clipboard::MemoryClipboard::default();
    clipboard.set_image(Some(b"PNG2".to_vec()));
    let mut app = app();
    app.use_clipboard(Box::new(clipboard.clone()));
    keys(&mut app, ":agent<CR>");
    until(&mut app, ready);
    keys(&mut app, "blocks ");
    // Dropped on the terminal: a path, shell-quoted. The image goes in;
    // another file's mentioned.
    let dropped = format!(
        "{} {}",
        shot.display().to_string().replace(' ', "\\ "),
        notes.display()
    );
    app.handle_paste(&dropped);
    keys(&mut app, "<C-v>");
    let typed = app.editor().agent_chat().input.text().to_string();
    assert!(typed.starts_with("blocks [Image #1] @"), "{typed}");
    assert!(typed.ends_with("notes.txt [Image #2] "), "{typed}");
    keys(&mut app, "<CR>");
    until(&mut app, |a| ready(a) && said(a).contains("image/png"));
    let said = said(&app);
    assert!(said.starts_with("text,resource_link,image,image"), "{said}");
    // Base64 of the files' bytes.
    assert!(said.contains("|image/png UE5HMQ=="), "{said}");
    assert!(said.contains("|image/png UE5HMg=="), "{said}");
    assert!(
        app.editor().agent_chat().images.is_empty(),
        "sent, and gone"
    );
}

#[test]
fn image_files_dropped_anywhere_go_to_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    let shot = dir.path().join("shot.png");
    fs::write(&shot, b"PNG").unwrap();
    let mut app = app();
    keys(&mut app, ":agent<CR><Esc>");
    assert_ne!(app.editor().mode(), Mode::Agent);
    app.handle_paste(&shot.display().to_string());
    assert_eq!(app.editor().mode(), Mode::Agent);
    assert_eq!(app.editor().agent_chat().input.text(), "[Image #1] ");
    // Text pasted in the editor stays there.
    keys(&mut app, "<Esc>");
    app.handle_paste("hello");
    assert_eq!(app.editor().agent_chat().input.text(), "[Image #1] ");
}

fn commands_ready(app: &App) -> bool {
    ready(app) && !app.editor().agent_chat().commands.is_empty()
}

#[test]
fn slash_lists_the_commands_and_runs_one() {
    // With a file open, so there's editor context that mustn't go with it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let mut app = app();
    app.open(&path);
    keys(&mut app, ":agent<CR>");
    until(&mut app, commands_ready);
    // The agent's commands and ferra's own; its /clear is ferra's.
    keys(&mut app, "/");
    let names: Vec<String> = app
        .editor()
        .agent_slash_matches()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["clear", "compact", "effort", "model", "review"]);
    keys(&mut app, "c");
    let names: Vec<String> = app
        .editor()
        .agent_slash_matches()
        .into_iter()
        .map(|c| c.name)
        .collect();
    // Starting with c first, then containing it.
    assert_eq!(names, ["clear", "compact"]);
    let rows = render(&mut app, 100, 30).rows;
    assert!(
        rows.iter()
            .any(|r| r.contains("/compact") && r.contains("Compact the")),
        "{rows:#?}"
    );
    // Down, then Enter: /compact (it takes nothing) is sent at once, first.
    keys(&mut app, "<Down><CR>");
    until(&mut app, |a| ready(a) && said(a) == "ran /compact");
    assert_eq!(app.editor().agent_chat().input.text(), "");
    // One that takes something: filled in, its hint shown after it.
    keys(&mut app, "/rev<Tab>");
    assert_eq!(app.editor().agent_chat().input.text(), "/review ");
    assert!(app.editor().agent_slash_matches().is_empty());
    let rows = render(&mut app, 100, 30).rows;
    assert!(
        rows.iter().any(|r| r.contains("/review [branch]")),
        "{rows:#?}"
    );
    keys(&mut app, "main<CR>");
    until(&mut app, |a| {
        ready(a) && said(a).ends_with("ran /review main")
    });
    // Esc puts the list away (the box keeps focus), then leaves.
    keys(&mut app, "/");
    assert!(!app.editor().agent_slash_matches().is_empty());
    keys(&mut app, "<Esc>");
    assert!(app.editor().agent_slash_matches().is_empty());
    assert_eq!(app.editor().mode(), Mode::Agent);
    keys(&mut app, "<Esc>");
    assert_ne!(app.editor().mode(), Mode::Agent);
}

#[test]
fn a_slash_row_is_clicked() {
    let mut app = app();
    keys(&mut app, ":agent<CR>");
    until(&mut app, commands_ready);
    keys(&mut app, "/");
    let (x, y) = find(&mut app, "/compact");
    click(&mut app, x, y);
    until(&mut app, |a| ready(a) && said(a) == "ran /compact");
}

#[test]
fn clear_starts_a_new_session() {
    let mut app = app();
    keys(&mut app, ":agent<CR>session<CR>");
    until(&mut app, |a| ready(a) && said(a) == "in s1");
    keys(&mut app, "/clear<CR>");
    let chat = app.editor().agent_chat();
    assert!(matches!(&chat.entries[..], [ChatEntry::Notice(n)] if n == "A new conversation"));
    until(&mut app, ready);
    keys(&mut app, "session<CR>");
    until(&mut app, |a| ready(a) && said(a) == "in s2");
}

#[test]
fn settings_are_chosen_from_their_chips() {
    let mut app = app();
    keys(&mut app, ":agent<CR>");
    until(&mut app, commands_ready);
    let chat = app.editor().agent_chat();
    assert_eq!(chat.config.len(), 3);
    // A click on the model's chip: its values.
    let (x, y) = find(&mut app, "Opus ▾");
    click(&mut app, x + 1, y);
    assert_eq!(app.editor().mode(), Mode::Picker);
    keys(&mut app, "sonnet<CR>");
    until(&mut app, |a| {
        a.editor()
            .agent_chat()
            .config
            .iter()
            .any(|o| o.id == "model" && o.current == "sonnet")
    });
    find(&mut app, "Sonnet ▾");
    // /model and :agent_model open the same list.
    keys(&mut app, "<Esc>:agent<CR>/model<CR>");
    assert_eq!(app.editor().mode(), Mode::Picker);
}

#[test]
fn the_context_used_is_shown_and_other_sessions_ignored() {
    let mut app = app();
    keys(&mut app, ":agent<CR>usage<CR>");
    until(&mut app, |a| ready(a) && said(a) == "counted");
    find(&mut app, "42k / 200k · $0.31");
}
