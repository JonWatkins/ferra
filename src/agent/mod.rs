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

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

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
        let mut changed = false;
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
                self.client = Some(AgentClient::start(&name, streams, &root, sender));
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
            AgentRequest::Prompt(text) => {
                self.start(editor);
                if let Some(client) = &mut self.client {
                    client.prompt(&text);
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
            AgentRequest::Stop => {
                if let Some(mut client) = self.client.take() {
                    client.stop();
                }
                self.receiver = None;
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
