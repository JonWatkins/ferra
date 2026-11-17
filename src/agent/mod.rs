//! Coding agents (Claude Code, Gemini CLI, Codex...), talked to over ACP,
//! the Agent Client Protocol: each runs as its own program, started from
//! its `[agents.<name>]` entry in config. ferra shows the chat, asks you
//! the agent's questions, and answers its requests for files; the agent
//! does the rest (its model, tools, memory, login).
//!
//! The editor keeps the chat (`editor/agent.rs`) and says what it wants
//! sent; `Agents::update`, called each time round the app's loop, sends it
//! and hands back what the agent said.

pub mod client;
pub mod mcp;
pub mod tools;

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::editor::Editor;
use crate::editor::agent::{AgentEvent, AgentRequest, AgentStatus};
use client::{AgentClient, Incoming, Streams, permission_ask, update_event};

/// How to start an agent: `[agents.claude] command = "claude-agent-acp"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct AgentConfig {
    /// The program (found on `PATH`).
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for it.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Starts agents. Tests replace it with one talking to a fake agent.
pub trait Spawner {
    fn spawn(&self, config: &AgentConfig, root: &Path) -> io::Result<Streams>;
}

/// Runs the agent's command in the project's folder, talking over its
/// stdin and stdout (its stderr, its own log, is dropped).
pub struct ProcessSpawner;

impl Spawner for ProcessSpawner {
    fn spawn(&self, config: &AgentConfig, root: &Path) -> io::Result<Streams> {
        let program =
            crate::lsp::find_program(&config.command, std::env::var_os("PATH").as_deref())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let mut child = Command::new(program)
            .args(&config.args)
            .envs(&config.env)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped");
        let stdin = child.stdin.take().expect("piped");
        Ok(Streams {
            reader: Box::new(BufReader::new(stdout)),
            writer: Box::new(stdin),
            child: Some(child),
        })
    }
}

/// The agent being talked to (one at a time), and how to start one.
pub struct Agents {
    configs: HashMap<String, AgentConfig>,
    spawner: Box<dyn Spawner>,
    client: Option<AgentClient>,
    receiver: Option<Receiver<Incoming>>,
    /// ferra's tools for it (`tools.rs`), served while it runs.
    #[cfg(unix)]
    mcp: Option<mcp::McpServer>,
    /// Tool calls waiting for a language server: by call, the connection
    /// and the request's id.
    waiting: HashMap<u64, (u64, Value)>,
    next_call: u64,
    /// Edits answered once their file's language server has said what's
    /// wrong in it now (or a moment's passed).
    after_edits: Vec<AfterEdit>,
}

/// A change's answer, waiting for what the language servers find wrong in
/// the files it changed.
struct AfterEdit {
    conn: u64,
    id: Value,
    text: String,
    /// The files, and their diagnostics' version when it was made.
    documents: Vec<(crate::editor::DocumentId, u64)>,
    started: Instant,
    /// When a server was last seen busy (checking, indexing).
    last_busy: Option<Instant>,
}

/// How long a change's answer waits for its files' diagnostics, at most.
const DIAGNOSTICS_WAIT: Duration = Duration::from_secs(30);
/// How long the servers must have been quiet (no new diagnostics, not
/// busy) before what they've said counts: rust-analyzer starts `cargo
/// check` a moment after a save, and reports the old check's results
/// (moved) before the new one's.
const SETTLED: Duration = Duration::from_millis(1000);
/// A server that hasn't said anything new this long after the change (or
/// after it was last busy) has nothing new to say.
const SILENT: Duration = Duration::from_secs(4);

/// What came of waiting on a change's diagnostics.
enum Settled {
    Waiting,
    /// What the servers say now.
    Reported,
    /// Nothing checks these files.
    Unchecked,
    /// Still checking when the wait ran out: which server.
    StillChecking(String),
}

impl AfterEdit {
    fn new(
        conn: u64,
        id: Value,
        text: String,
        documents: &[crate::editor::DocumentId],
        editor: &Editor,
    ) -> Self {
        Self {
            conn,
            id,
            text,
            documents: documents
                .iter()
                .map(|&d| (d, editor.diagnostics_version(d)))
                .collect(),
            started: Instant::now(),
            last_busy: None,
        }
    }

    /// Whether the servers have had their say about the changed files.
    fn settled(&mut self, editor: &Editor) -> Settled {
        let now = Instant::now();
        let checked: Vec<_> = self
            .documents
            .iter()
            .filter_map(|&(d, version)| editor.lsp_check(d).map(|check| (d, version, check)))
            .collect();
        if checked.is_empty() {
            // A server may yet open the file it was just given.
            let maybe = self
                .documents
                .iter()
                .any(|&(d, _)| editor.has_language_server(d));
            return if !maybe || now - self.started >= SETTLED {
                Settled::Unchecked
            } else {
                Settled::Waiting
            };
        }
        if let Some((_, _, (_, Some(_)))) = checked.iter().find(|(_, _, (_, busy))| busy.is_some())
        {
            self.last_busy = Some(now);
        }
        let quiet = |at: Option<Instant>| at.is_none_or(|at| now - at >= SETTLED);
        let busy_now = checked.iter().any(|(_, _, (_, busy))| busy.is_some());
        let reported = checked
            .iter()
            .all(|&(d, version, _)| editor.diagnostics_version(d) > version);
        let last_report = checked
            .iter()
            .filter_map(|&(d, _, _)| editor.diagnostics_set_at(d))
            .max();
        if !busy_now && quiet(self.last_busy) && quiet(last_report) {
            if reported {
                return Settled::Reported;
            }
            // Nothing new for a while since the change (or since it was
            // last busy): nothing new to say.
            if now - self.last_busy.unwrap_or(self.started) >= SILENT {
                return Settled::Reported;
            }
        }
        if now - self.started >= DIAGNOSTICS_WAIT {
            let (_, _, (server, _)) = checked[0];
            return Settled::StillChecking(server.to_string());
        }
        Settled::Waiting
    }
}

impl Default for Agents {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl Agents {
    pub fn new(configs: HashMap<String, AgentConfig>) -> Self {
        Self {
            configs,
            spawner: Box::new(ProcessSpawner),
            client: None,
            receiver: None,
            #[cfg(unix)]
            mcp: None,
            waiting: HashMap::new(),
            next_call: 0,
            after_edits: Vec::new(),
        }
    }

    /// The socket ferra's tools are served on, while the agent runs.
    pub fn mcp_socket(&self) -> Option<&Path> {
        #[cfg(unix)]
        return self.mcp.as_ref().map(mcp::McpServer::path);
        #[cfg(not(unix))]
        None
    }

    /// The tools, as the session's MCP servers: the bridge to start.
    fn mcp_servers(&mut self) -> Value {
        #[cfg(unix)]
        {
            if self.mcp.is_none() {
                self.mcp = mcp::McpServer::start().ok();
            }
            let (Some(server), Ok(exe)) = (&self.mcp, std::env::current_exe()) else {
                return json!([]);
            };
            json!([{
                "name": "ferra",
                "command": exe.to_string_lossy(),
                "args": ["--mcp-bridge", server.path().to_string_lossy()],
                "env": [],
            }])
        }
        #[cfg(not(unix))]
        json!([])
    }

    /// Answers what came over MCP, and the language servers' answers to
    /// tool calls. Returns whether anything did.
    fn update_tools(&mut self, editor: &mut Editor) -> bool {
        let mut changed = false;
        #[cfg(unix)]
        while let Some(incoming) = self.mcp.as_ref().and_then(mcp::McpServer::try_recv) {
            changed = true;
            match incoming {
                mcp::McpIncoming::Message(conn, message) => self.handle_mcp(conn, message, editor),
                mcp::McpIncoming::Closed(conn) => {
                    if let Some(server) = &self.mcp {
                        server.forget(conn);
                    }
                    self.waiting.retain(|_, (c, _)| *c != conn);
                    self.after_edits.retain(|e| e.conn != conn);
                }
            }
        }
        for answer in editor.take_agent_lsp_answers() {
            changed = true;
            let Some((conn, id)) = self.waiting.remove(&answer.call) else {
                continue;
            };
            match answer.answer {
                Ok(text) if !answer.changed.is_empty() => {
                    let edit = AfterEdit::new(conn, id, text, &answer.changed, editor);
                    self.after_edits.push(edit);
                }
                answer => self.reply(conn, id, tool_result(answer)),
            }
        }
        // Changes whose files' servers have had their say (or that waited
        // long enough).
        let mut waiting = Vec::new();
        for mut edit in std::mem::take(&mut self.after_edits) {
            let ids: Vec<_> = edit.documents.iter().map(|&(d, _)| d).collect();
            let text = match edit.settled(editor) {
                Settled::Waiting => {
                    waiting.push(edit);
                    continue;
                }
                Settled::Reported => format!("{}{}", edit.text, editor.agent_problems(&ids)),
                Settled::Unchecked => edit.text,
                Settled::StillChecking(server) => format!(
                    "{}\n{server} is still checking: get_diagnostics will say what's wrong in a moment.",
                    edit.text
                ),
            };
            changed = true;
            self.reply(edit.conn, edit.id, tool_result(Ok(text)));
        }
        self.after_edits = waiting;
        changed
    }

    fn reply(&self, conn: u64, id: Value, result: Value) {
        #[cfg(unix)]
        if let Some(server) = &self.mcp {
            server.send(
                conn,
                &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            );
        }
        #[cfg(not(unix))]
        let _ = (conn, id, result);
    }

    fn reply_error(&self, conn: u64, id: Value, code: i64, message: &str) {
        #[cfg(unix)]
        if let Some(server) = &self.mcp {
            server.send(
                conn,
                &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
            );
        }
        #[cfg(not(unix))]
        let _ = (conn, id, code, message);
    }

    /// An MCP request (or notification) from the agent.
    fn handle_mcp(&mut self, conn: u64, message: Value, editor: &mut Editor) {
        let Some(id) = message.get("id").cloned().filter(|id| !id.is_null()) else {
            // A notification (`notifications/initialized`...): nothing to say.
            return;
        };
        let params = &message["params"];
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                let version = params["protocolVersion"].as_str().unwrap_or("2025-06-18");
                self.reply(
                    conn,
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "ferra", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": tools::INSTRUCTIONS,
                    }),
                );
            }
            "ping" => self.reply(conn, id, json!({})),
            "tools/list" => self.reply(conn, id, json!({ "tools": tools::list() })),
            "tools/call" => {
                let call = self.next_call;
                self.next_call += 1;
                let name = params["name"].as_str().unwrap_or_default();
                match tools::call(editor, call, name, &params["arguments"]) {
                    tools::Called::Done(answer) => self.reply(conn, id, tool_result(answer)),
                    tools::Called::Waiting => {
                        self.waiting.insert(call, (conn, id));
                    }
                    tools::Called::Changed(text, documents) => {
                        self.after_edits
                            .push(AfterEdit::new(conn, id, text, &documents, editor));
                    }
                }
            }
            _ => self.reply_error(conn, id, -32601, "Method not found"),
        }
    }

    pub fn set_spawner(&mut self, spawner: Box<dyn Spawner>) {
        self.spawner = spawner;
    }

    pub fn set_configs(&mut self, configs: HashMap<String, AgentConfig>) {
        self.configs = configs;
    }

    /// Whether an agent is running (so the app keeps checking for its
    /// messages).
    pub fn running(&self) -> bool {
        self.client.is_some()
    }

    /// Sends what the editor wants sent, and hands it what the agent said.
    /// Returns whether anything changed.
    pub fn update(&mut self, editor: &mut Editor) -> bool {
        let mut changed = self.update_tools(editor);
        for request in editor.take_agent_requests() {
            changed = true;
            self.send(request, editor);
        }
        while let Some(Ok(incoming)) = self.receiver.as_ref().map(Receiver::try_recv) {
            changed = true;
            match incoming {
                Incoming::Message(message) => self.handle(message, editor),
                Incoming::Closed => {
                    self.receiver = None;
                    if self.client.take().is_some() {
                        editor.agent_event(AgentEvent::Status(AgentStatus::Stopped(
                            "it exited".into(),
                        )));
                    }
                }
            }
        }
        if self.client.as_ref().is_some_and(AgentClient::stopped) {
            self.client = None;
            self.receiver = None;
            if !matches!(editor.agent_chat().status, AgentStatus::Stopped(_)) {
                editor.agent_event(AgentEvent::Status(AgentStatus::Stopped(
                    "it stopped reading its input".into(),
                )));
            }
        }
        changed
    }

    fn start(&mut self, editor: &mut Editor) {
        if self.client.is_some() {
            return;
        }
        let name = editor.agent_chat().agent.clone();
        let Some(config) = self.configs.get(&name).cloned() else {
            editor.agent_failed(format!("No agent named \"{name}\" in [agents]"));
            return;
        };
        let here = std::env::current_dir().unwrap_or_default();
        let root = std::fs::canonicalize(&here).unwrap_or(here);
        match self.spawner.spawn(&config, &root) {
            Ok(streams) => {
                let (sender, receiver) = channel();
                let servers = self.mcp_servers();
                self.client = Some(AgentClient::start(&name, streams, &root, sender, servers));
                self.receiver = Some(receiver);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => editor.agent_failed(format!(
                "{} isn't installed (or set [agents.{name}] command)",
                config.command
            )),
            Err(e) => editor.agent_failed(format!("Couldn't start {}: {e}", config.command)),
        }
    }

    fn send(&mut self, request: AgentRequest, editor: &mut Editor) {
        match request {
            AgentRequest::Start => self.start(editor),
            AgentRequest::Restart => {
                if let Some(mut client) = self.client.take() {
                    client.stop();
                }
                self.receiver = None;
                self.start(editor);
            }
            AgentRequest::Prompt {
                text,
                context,
                files,
                images,
            } => {
                self.start(editor);
                if let Some(client) = &mut self.client {
                    let mut blocks = prompt_blocks(&text, context.as_deref(), &files);
                    // Nothing goes with a slash command (see prompt_blocks).
                    let images = if is_command(&text) {
                        Vec::new()
                    } else {
                        images
                    };
                    for (mime, data) in images {
                        blocks.push(json!({
                            "type": "image",
                            "mimeType": mime,
                            "data": crate::clipboard::base64(&data),
                        }));
                    }
                    client.prompt(blocks);
                }
            }
            AgentRequest::Cancel => {
                if let Some(client) = &mut self.client {
                    client.cancel();
                }
            }
            AgentRequest::Permission { request, option } => {
                if let Some(client) = &mut self.client {
                    let outcome = match option {
                        Some(id) => json!({ "outcome": "selected", "optionId": id }),
                        None => json!({ "outcome": "cancelled" }),
                    };
                    client.answer(request, json!({ "outcome": outcome }));
                }
            }
            AgentRequest::SetMode(mode) => {
                if let Some(client) = &mut self.client {
                    client.set_mode(&mode);
                }
            }
            AgentRequest::SetConfig { id, value } => {
                if let Some(client) = &mut self.client {
                    client.set_config(&id, value);
                }
            }
            AgentRequest::NewSession => match &mut self.client {
                Some(client) => client.new_session(),
                None => self.start(editor),
            },
            AgentRequest::Stop => {
                if let Some(mut client) = self.client.take() {
                    client.stop();
                }
                self.receiver = None;
                self.waiting.clear();
                self.after_edits.clear();
                #[cfg(unix)]
                {
                    self.mcp = None;
                }
                editor.agent_event(AgentEvent::Status(AgentStatus::Stopped("stopped".into())));
            }
        }
    }

    /// A message from the agent: a request, a notification or an answer.
    fn handle(&mut self, message: Value, editor: &mut Editor) {
        let Some(client) = &mut self.client else {
            return;
        };
        let method = message["method"].as_str().map(str::to_string);
        let id = message.get("id").cloned().filter(|id| !id.is_null());
        match (method.as_deref(), id) {
            (Some("session/update"), None) => {
                if !client.is_current(message["params"]["sessionId"].as_str()) {
                    return;
                }
                if let Some(event) = update_event(&message["params"]["update"]) {
                    editor.agent_event(event);
                }
            }
            (Some(_), None) => {}
            (Some("session/request_permission"), Some(id)) => {
                editor.agent_event(AgentEvent::Permission(permission_ask(
                    id,
                    &message["params"],
                )));
            }
            (Some("fs/read_text_file"), Some(id)) => {
                match read_text_file(editor, &message["params"]) {
                    Ok(content) => client.answer(id, json!({ "content": content })),
                    Err(e) => client.answer_error(id, -32603, &e),
                }
            }
            (Some(_), Some(id)) => client.answer_error(id, -32601, "Method not found"),
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else { return };
                for event in client.handle_response(id, &message) {
                    editor.agent_event(event);
                }
            }
            (None, None) => {}
        }
    }
}

/// A tool's answer, as MCP gives it: text, marked if it's an error.
fn tool_result(answer: Result<String, String>) -> Value {
    let (text, error) = match answer {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": error })
}

/// A prompt's content blocks: a note of what's open in the editor (if
/// any), what was typed, and a link to each file it mentions. A slash
/// command goes alone: Claude Code runs one only when it's the whole
/// prompt (with a note after it, it reaches the model as a message).
pub fn prompt_blocks(text: &str, context: Option<&str>, files: &[PathBuf]) -> Vec<Value> {
    let typed = json!({ "type": "text", "text": text });
    if is_command(text) {
        return vec![typed];
    }
    let mut blocks = Vec::new();
    if let Some(context) = context {
        blocks.push(json!({
            "type": "text",
            "text": format!("<editor-context>\n{context}\n</editor-context>"),
        }));
    }
    blocks.push(typed);
    for file in files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        blocks.push(json!({
            "type": "resource_link",
            "uri": crate::lsp::convert::path_to_uri(file),
            "name": name,
        }));
    }
    blocks
}

/// Whether `text` is a slash command (`/compact`, `/review main`), not a
/// message that starts with a path (`/Users/me/a.rs is broken`).
pub fn is_command(text: &str) -> bool {
    text.strip_prefix('/')
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(|name| !name.contains('/'))
}

/// `fs/read_text_file`: the file's text (from its buffer, if it's open, so
/// unsaved changes count), from line `line` (from 1) for `limit` lines.
fn read_text_file(editor: &Editor, params: &Value) -> Result<String, String> {
    let path = PathBuf::from(params["path"].as_str().ok_or("No path")?);
    let text = match editor
        .find_open_path(&path)
        .and_then(|id| editor.document_by_id(id))
    {
        Some(document) => document.buffer().text(),
        None => std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    };
    let first = params["line"]
        .as_u64()
        .map_or(0, |l| (l as usize).saturating_sub(1));
    let limit = params["limit"].as_u64().map(|l| l as usize);
    if first == 0 && limit.is_none() {
        return Ok(text);
    }
    let lines: Vec<&str> = text.split_inclusive('\n').skip(first).collect();
    let lines = match limit {
        Some(limit) => &lines[..limit.min(lines.len())],
        None => &lines[..],
    };
    Ok(lines.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slash_command_is_sent_alone() {
        let files = [PathBuf::from("/p/a.rs")];
        let blocks = prompt_blocks("/review main", Some("In a.rs"), &files);
        assert_eq!(blocks, [json!({"type": "text", "text": "/review main"})]);
        // A message: the note first, then what was typed, then the links.
        let blocks = prompt_blocks("fix a.rs", Some("In a.rs"), &files);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[1]["text"], "fix a.rs");
        // A path at the start isn't a command.
        assert!(is_command("/usage") && is_command("/mcp:server:tool x"));
        assert!(!is_command("/Users/me/a.rs is broken"));
        assert!(!is_command("/") && !is_command("hello /usage"));
    }
}
