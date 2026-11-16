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
            "session/new" => reply(
                &mut out,
                json!({"sessionId": "s1", "modes": {"currentModeId": "default",
                       "availableModes": [{"id": "default", "name": "Default"},
                                          {"id": "acceptEdits", "name": "Accept edits"}]}}),
            ),
            "session/prompt" => {
                let asked = message["params"]["prompt"][0]["text"]
                    .as_str()
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
