//! One agent, talked to over ACP (the Agent Client Protocol): JSON-RPC
//! messages, one per line, over the agent's stdin and stdout. A thread
//! reads its output into a channel the app checks each time round its
//! loop (as for language servers), so a slow or stuck agent never holds
//! up the editor.
//!
//! The session: `initialize` (protocol version and what each side can
//! do), `session/new` (in the project's folder), then `session/prompt` for
//! each message, the agent streaming `session/update`s until it answers
//! with why the turn stopped. The agent asks things of the editor too:
//! permission for a tool call, a file's text.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::Sender;
use std::thread;

use serde_json::{Value, json};

use crate::editor::agent::{
    AgentEvent, AgentStatus, PermissionAsk, PermissionOption, PlanStep, ToolCall, ToolDiff,
    ToolStatus,
};

/// The protocol version ferra speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// What the reader thread sends: a message, or that the agent's output
/// ended.
#[derive(Debug)]
pub enum Incoming {
    Message(Value),
    Closed,
}

/// An agent's stdin and stdout (and its process, to stop it).
pub struct Streams {
    pub reader: Box<dyn BufRead + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Option<Child>,
}

/// What a request ferra sent was, for its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Initialize,
    NewSession,
    Prompt,
    SetMode,
}

pub struct AgentClient {
    /// Its name in `[agents]`.
    pub name: String,
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
    root: PathBuf,
    next_id: i64,
    pending: HashMap<i64, Pending>,
    session: Option<String>,
    /// Prompts sent before the session was ready.
    queued: Vec<Vec<Value>>,
    /// It stopped reading (or was stopped).
    stopped: bool,
    /// The MCP servers it's given with its session (ferra's own tools).
    mcp_servers: Value,
}

/// One JSON message, on its own line.
pub fn write_message(out: &mut impl Write, message: &Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    out.write_all(&line)?;
    out.flush()
}

/// The next message (blank lines skipped), or `None` at the end.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        return serde_json::from_str(text)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    }
}

impl AgentClient {
    /// Talks to the agent on `streams`, its messages going to `sender`,
    /// and starts the session in `root`, giving it `mcp_servers` (ACP's
    /// `McpServer`s) for tools.
    pub fn start(
        name: &str,
        streams: Streams,
        root: &Path,
        sender: Sender<Incoming>,
        mcp_servers: Value,
    ) -> Self {
        let Streams {
            mut reader,
            writer,
            child,
        } = streams;
        thread::spawn(move || {
            loop {
                match read_message(&mut reader) {
                    Ok(Some(message)) => {
                        if sender.send(Incoming::Message(message)).is_err() {
                            return;
                        }
                    }
                    // A line that isn't JSON: skipped (it MUST NOT happen).
                    Err(e) if e.kind() == io::ErrorKind::InvalidData => continue,
                    Ok(None) | Err(_) => break,
                }
            }
            let _ = sender.send(Incoming::Closed);
        });
        let mut client = AgentClient {
            name: name.to_string(),
            writer,
            child,
            root: root.to_path_buf(),
            next_id: 1,
            pending: HashMap::new(),
            session: None,
            queued: Vec::new(),
            stopped: false,
            mcp_servers,
        };
        client.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": false },
                    "terminal": false,
                },
                "clientInfo": { "name": "ferra", "version": env!("CARGO_PKG_VERSION") },
            }),
            Pending::Initialize,
        );
        client
    }

    fn send(&mut self, message: &Value) {
        if write_message(&mut self.writer, message).is_err() {
            self.stopped = true;
        }
    }

    fn request(&mut self, method: &str, params: Value, kind: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, kind);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Answers request `id` (the agent's) with `result`.
    pub fn answer(&mut self, id: Value, result: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    /// Answers request `id` with an error.
    pub fn answer_error(&mut self, id: Value, code: i64, message: &str) {
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message },
        }));
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Sends a prompt, its content blocks (once the session's ready).
    pub fn prompt(&mut self, blocks: Vec<Value>) {
        let Some(session) = self.session.clone() else {
            self.queued.push(blocks);
            return;
        };
        self.request(
            "session/prompt",
            json!({ "sessionId": session, "prompt": blocks }),
            Pending::Prompt,
        );
    }

    /// Stops the turn being worked on.
    pub fn cancel(&mut self) {
        self.queued.clear();
        if let Some(session) = self.session.clone() {
            self.notify("session/cancel", json!({ "sessionId": session }));
        }
    }

    pub fn set_mode(&mut self, mode: &str) {
        if let Some(session) = self.session.clone() {
            self.request(
                "session/set_mode",
                json!({ "sessionId": session, "modeId": mode }),
                Pending::SetMode,
            );
        }
    }

    /// Stops the agent's process.
    pub fn stop(&mut self) {
        self.stopped = true;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// An answer to one of ferra's requests: what it means for the chat.
    pub fn handle_response(&mut self, id: i64, message: &Value) -> Vec<AgentEvent> {
        let Some(kind) = self.pending.remove(&id) else {
            return Vec::new();
        };
        if let Some(error) = message.get("error") {
            let text = error["message"].as_str().unwrap_or("an error").to_string();
            return match kind {
                Pending::Initialize | Pending::NewSession => {
                    self.stop();
                    // Most often: it needs logging in first.
                    let hint = if error["code"].as_i64() == Some(-32000) {
                        format!(" (log in to {} in a terminal first)", self.name)
                    } else {
                        String::new()
                    };
                    vec![AgentEvent::Status(AgentStatus::Stopped(format!("{text}{hint}")))]
                }
                Pending::Prompt => vec![
                    AgentEvent::Notice(format!("Error: {text}")),
                    AgentEvent::TurnEnded("end_turn".into()),
                ],
                Pending::SetMode => vec![AgentEvent::Notice(format!("Couldn't change mode: {text}"))],
            };
        }
        let result = &message["result"];
        match kind {
            Pending::Initialize => {
                let version = result["protocolVersion"].as_u64().unwrap_or(0);
                if version != PROTOCOL_VERSION {
                    self.stop();
                    return vec![AgentEvent::Status(AgentStatus::Stopped(format!(
                        "it speaks ACP version {version}; ferra speaks {PROTOCOL_VERSION}"
                    )))];
                }
                let info = &result["agentInfo"];
                let name = info["title"]
                    .as_str()
                    .or(info["name"].as_str())
                    .unwrap_or(&self.name)
                    .to_string();
                let started = match info["version"].as_str() {
                    Some(version) => format!("{name} {version}"),
                    None => name,
                };
                let cwd = self.root.to_string_lossy().into_owned();
                self.request(
                    "session/new",
                    json!({ "cwd": cwd, "mcpServers": self.mcp_servers.clone() }),
                    Pending::NewSession,
                );
                let images = result["agentCapabilities"]["promptCapabilities"]["image"]
                    .as_bool()
                    .unwrap_or(false);
                vec![AgentEvent::Started(started), AgentEvent::TakesImages(images)]
            }
            Pending::NewSession => {
                self.session = result["sessionId"].as_str().map(str::to_string);
                let mut events = vec![AgentEvent::Status(AgentStatus::Ready)];
                if let Some(modes) = result.get("modes").filter(|m| !m.is_null()) {
                    events.push(modes_event(modes));
                }
                let queued = std::mem::take(&mut self.queued);
                if !queued.is_empty() {
                    events.push(AgentEvent::Status(AgentStatus::Busy));
                }
                for blocks in queued {
                    self.prompt(blocks);
                }
                events
            }
            Pending::Prompt => {
                let reason = result["stopReason"].as_str().unwrap_or("end_turn");
                vec![AgentEvent::TurnEnded(reason.to_string())]
            }
            Pending::SetMode => Vec::new(),
        }
    }
}

impl Drop for AgentClient {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `modes` (`{currentModeId, availableModes: [{id, name}]}`) as an event.
fn modes_event(modes: &Value) -> AgentEvent {
    let list = modes["availableModes"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?;
                    Some((id.to_string(), m["name"].as_str().unwrap_or(id).to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    AgentEvent::Modes(list, modes["currentModeId"].as_str().map(str::to_string))
}

/// A tool call's title as shown: an MCP tool's (`mcp__ferra__open_file`)
/// said in words from what it was given ("Open src/main.rs:12"); others
/// as the agent put them.
pub fn tool_title(title: &str, input: &Value) -> String {
    let Some(rest) = title.strip_prefix("mcp__") else {
        return title.to_string();
    };
    let Some((server, tool)) = rest.split_once("__") else {
        return title.to_string();
    };
    let text = |key: &str| input[key].as_str().filter(|s| !s.is_empty());
    let path = text("path").map(crate::editor::agent::short_path);
    let line = input["line"].as_u64();
    let place = match (&path, line) {
        (Some(path), Some(line)) => format!("{path}:{line}"),
        (Some(path), None) => path.clone(),
        (None, _) => String::new(),
    };
    let about = |what: &str| match text("symbol") {
        Some(symbol) => format!("{what} {symbol}"),
        None if !place.is_empty() => format!("{what} {place}"),
        None => what.to_string(),
    };
    if server != "ferra" {
        return format!("{} ({server})", tool.replace('_', " "));
    }
    match tool {
        "editor_state" => "Look at the editor".into(),
        "get_diagnostics" => match &path {
            Some(path) => format!("Diagnostics in {path}"),
            None => "Diagnostics".into(),
        },
        "read_buffer" => format!("Read {place}"),
        "edit_buffer" => format!("Edit {}", path.unwrap_or_default()),
        "lsp_definition" => match text("kind") {
            Some("type") => about("Type of"),
            Some("implementation") => about("Implementations of"),
            Some("declaration") => about("Declaration of"),
            _ => about("Definition of"),
        },
        "lsp_references" => about("References to"),
        "lsp_hover" => about("Describe"),
        "lsp_symbols" => match (&path, text("query")) {
            (Some(path), _) => format!("Symbols in {path}"),
            (None, Some(query)) => format!("Find symbol {query}"),
            (None, None) => "Symbols".into(),
        },
        "lsp_rename" => match (text("symbol"), text("new_name")) {
            (Some(old), Some(new)) => format!("Rename {old} to {new}"),
            (_, Some(new)) => format!("Rename to {new}"),
            _ => "Rename".into(),
        },
        "lsp_code_actions" => format!("Code actions at {place}"),
        "lsp_apply_code_action" => match input["number"].as_u64() {
            Some(n) => format!("Make code action {n}"),
            None => "Make a code action".into(),
        },
        "lsp_format" => format!("Format {}", path.unwrap_or_default()),
        "open_file" => format!("Open {place}"),
        other => other.replace('_', " "),
    }
}

fn tool_status(value: &Value) -> Option<ToolStatus> {
    Some(match value.as_str()? {
        "pending" => ToolStatus::Pending,
        "in_progress" => ToolStatus::Running,
        "completed" => ToolStatus::Done,
        "failed" => ToolStatus::Failed,
        _ => return None,
    })
}

/// A tool call's `content` list: its text, and its diffs.
fn tool_content(content: &Value) -> (String, Vec<ToolDiff>) {
    let mut text = String::new();
    let mut diffs = Vec::new();
    for item in content.as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("content") => {
                if let Some(t) = item["content"]["text"].as_str() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t);
                }
            }
            Some("diff") => diffs.push(ToolDiff {
                path: item["path"].as_str().unwrap_or_default().to_string(),
                old: item["oldText"].as_str().map(str::to_string),
                new: item["newText"].as_str().unwrap_or_default().to_string(),
            }),
            _ => {}
        }
    }
    (text, diffs)
}

/// A content block's text (`{type: "text", text}`), if it's text.
fn block_text(block: &Value) -> Option<String> {
    (block["type"].as_str() == Some("text"))
        .then(|| block["text"].as_str().map(str::to_string))
        .flatten()
}

/// A `session/update` notification's `update`, for the chat.
pub fn update_event(update: &Value) -> Option<AgentEvent> {
    Some(match update["sessionUpdate"].as_str()? {
        "agent_message_chunk" => AgentEvent::Text(block_text(&update["content"])?),
        "agent_thought_chunk" => AgentEvent::Thought(block_text(&update["content"])?),
        "tool_call" => {
            let (text, diffs) = tool_content(&update["content"]);
            let locations = update["locations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    Some((
                        l["path"].as_str()?.to_string(),
                        l["line"].as_u64().map(|n| n as usize),
                    ))
                })
                .collect();
            AgentEvent::ToolStarted(ToolCall {
                id: update["toolCallId"].as_str()?.to_string(),
                title: tool_title(update["title"].as_str().unwrap_or("Tool"), &update["rawInput"]),
                kind: update["kind"].as_str().unwrap_or("other").to_string(),
                status: tool_status(&update["status"]).unwrap_or_default(),
                text,
                diffs,
                locations,
            })
        }
        "tool_call_update" => {
            let has_content = update.get("content").is_some_and(|c| !c.is_null());
            let (text, diffs) = tool_content(&update["content"]);
            AgentEvent::ToolUpdated {
                id: update["toolCallId"].as_str()?.to_string(),
                title: update["title"]
                    .as_str()
                    .map(|t| tool_title(t, &update["rawInput"])),
                status: tool_status(&update["status"]),
                text: has_content.then_some(text),
                diffs: has_content.then_some(diffs),
            }
        }
        "plan" => AgentEvent::Plan(
            update["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| PlanStep {
                    text: e["content"].as_str().unwrap_or_default().to_string(),
                    status: e["status"].as_str().unwrap_or("pending").to_string(),
                })
                .collect(),
        ),
        "available_commands_update" => AgentEvent::Commands(
            update["availableCommands"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some((
                        c["name"].as_str()?.to_string(),
                        c["description"].as_str().unwrap_or_default().to_string(),
                    ))
                })
                .collect(),
        ),
        "current_mode_update" => AgentEvent::Mode(update["currentModeId"].as_str()?.to_string()),
        _ => return None,
    })
}

/// A `session/request_permission` request's params, for the chat.
pub fn permission_ask(id: Value, params: &Value) -> PermissionAsk {
    let title = params["toolCall"]["title"]
        .as_str()
        .unwrap_or("The agent asks permission")
        .to_string();
    let options = params["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| {
            Some(PermissionOption {
                id: o["optionId"].as_str()?.to_string(),
                name: o["name"].as_str().unwrap_or_default().to_string(),
                kind: o["kind"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect();
    PermissionAsk {
        request: id,
        title,
        options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_a_line_each() {
        let mut wire = Vec::new();
        write_message(&mut wire, &json!({"a": "x\ny"})).unwrap();
        write_message(&mut wire, &json!({"b": 2})).unwrap();
        assert_eq!(String::from_utf8(wire.clone()).unwrap().lines().count(), 2);
        let mut input = io::Cursor::new(wire);
        assert_eq!(read_message(&mut input).unwrap(), Some(json!({"a": "x\ny"})));
        assert_eq!(read_message(&mut input).unwrap(), Some(json!({"b": 2})));
        assert_eq!(read_message(&mut input).unwrap(), None);
    }

    #[test]
    fn updates_become_chat_events() {
        let text = json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Hi"}});
        assert_eq!(update_event(&text), Some(AgentEvent::Text("Hi".into())));
        let tool = json!({
            "sessionUpdate": "tool_call", "toolCallId": "t", "title": "Edit a.rs", "kind": "edit",
            "status": "pending",
            "content": [{"type": "diff", "path": "/p/a.rs", "oldText": "x", "newText": "y"}],
            "locations": [{"path": "/p/a.rs", "line": 3}],
        });
        let Some(AgentEvent::ToolStarted(call)) = update_event(&tool) else {
            panic!()
        };
        assert_eq!(call.diffs[0].new, "y");
        assert_eq!(call.locations, [("/p/a.rs".to_string(), Some(3))]);
        assert_eq!(update_event(&json!({"sessionUpdate": "usage_update"})), None);
    }

    #[test]
    fn mcp_tools_are_named_in_words() {
        let input = json!({"path": "/no/such/main.rs", "line": 12, "symbol": "area"});
        assert_eq!(tool_title("mcp__ferra__open_file", &input), "Open /no/such/main.rs:12");
        assert_eq!(tool_title("mcp__ferra__lsp_references", &input), "References to area");
        let kind = json!({"symbol": "Shape", "kind": "implementation"});
        assert_eq!(tool_title("mcp__ferra__lsp_definition", &kind), "Implementations of Shape");
        assert_eq!(tool_title("mcp__github__list_issues", &json!({})), "list issues (github)");
        assert_eq!(tool_title("Read main.rs", &json!({})), "Read main.rs");
    }
}
