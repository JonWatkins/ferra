//! The coding agent's chat, the editor's side: what's been said and done
//! (the transcript), what the agent is asking, and what the editor wants
//! sent to it. The app's `agent` module talks to the agent (over ACP, the
//! Agent Client Protocol) and turns what it says into `AgentEvent`s here.
//!
//! Until the chat has its own panel, it's shown in a popup (`:agent`).

use super::popup::{Popup, PopupLine, PopupSpan};
use super::{Editor, Message};

/// Where the agent's at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentStatus {
    /// Not started.
    #[default]
    Off,
    /// Starting up, or making its session.
    Starting,
    /// Waiting for a prompt.
    Ready,
    /// Working on one.
    Busy,
    /// It stopped (or couldn't start): why.
    Stopped(String),
}

/// A step of a tool call's progress.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ToolStatus {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
}

/// A file change a tool call made (or proposes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiff {
    pub path: String,
    /// `None` for a new file.
    pub old: Option<String>,
    pub new: String,
}

/// One tool call: reading a file, an edit, a command...
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub title: String,
    /// `read`, `edit`, `execute`, `search`... (ACP's kinds).
    pub kind: String,
    pub status: ToolStatus,
    /// What it produced, as text.
    pub text: String,
    pub diffs: Vec<ToolDiff>,
    /// The files it's about (and the line, if it said).
    pub locations: Vec<(String, Option<usize>)>,
}

/// A step of the agent's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub text: String,
    /// `pending`, `in_progress` or `completed`.
    pub status: String,
}

/// A part of the chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEntry {
    /// What you asked.
    User(String),
    /// What the agent said (Markdown), as it streams in.
    Agent(String),
    /// What it thought, when it shows that.
    Thought(String),
    Tool(ToolCall),
    Plan(Vec<PlanStep>),
    /// From ferra: the agent started, stopped, a turn ended oddly.
    Notice(String),
}

/// A choice the agent offers when asking permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once`, `reject_always`.
    pub kind: String,
}

/// The agent asking permission (to edit, to run a command...).
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionAsk {
    /// The request's id, for the answer.
    pub request: serde_json::Value,
    pub title: String,
    pub options: Vec<PermissionOption>,
}

/// What the editor wants sent to the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRequest {
    /// Start it (if it isn't running).
    Start,
    Prompt(String),
    /// Stop the turn it's working on.
    Cancel,
    /// The answer to a permission request: the option chosen, or `None`
    /// (cancelled).
    Permission {
        request: serde_json::Value,
        option: Option<String>,
    },
    SetMode(String),
    /// Stop the agent.
    Stop,
}

/// What the agent said or did, for the chat.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Status(AgentStatus),
    /// Its name and version, once started.
    Started(String),
    Text(String),
    Thought(String),
    ToolStarted(ToolCall),
    /// A tool call's changes: what's `Some` is new.
    ToolUpdated {
        id: String,
        title: Option<String>,
        status: Option<ToolStatus>,
        text: Option<String>,
        diffs: Option<Vec<ToolDiff>>,
    },
    Plan(Vec<PlanStep>),
    /// The modes it has (id and name) and the one it's in.
    Modes(Vec<(String, String)>, Option<String>),
    Mode(String),
    /// Its slash commands: name and description.
    Commands(Vec<(String, String)>),
    Permission(PermissionAsk),
    /// The turn ended: why (`end_turn`, `cancelled`, `refusal`...).
    TurnEnded(String),
    Notice(String),
}

/// The chat with the agent.
#[derive(Debug, Clone, Default)]
pub struct AgentChat {
    /// Which agent (its name in `[agents]`).
    pub agent: String,
    /// Its own name and version, once it's said.
    pub agent_info: Option<String>,
    pub status: AgentStatus,
    pub entries: Vec<ChatEntry>,
    pub permission: Option<PermissionAsk>,
    pub modes: Vec<(String, String)>,
    pub mode: Option<String>,
    pub commands: Vec<(String, String)>,
    requests: Vec<AgentRequest>,
    /// The chat popup was asked for (shown again as things change).
    shown: bool,
}

/// The chat popup's name.
const POPUP: &str = "agent";

impl Editor {
    pub fn agent_chat(&self) -> &AgentChat {
        &self.agent
    }

    /// What's to be sent to the agent, for the app.
    pub fn take_agent_requests(&mut self) -> Vec<AgentRequest> {
        std::mem::take(&mut self.agent.requests)
    }

    /// `:agent`: shows the chat, starting the agent if it isn't running.
    pub fn agent_open(&mut self) {
        if matches!(self.agent.status, AgentStatus::Off | AgentStatus::Stopped(_)) {
            self.agent.agent = self.options.agent.clone();
            self.agent.status = AgentStatus::Starting;
            self.agent.requests.push(AgentRequest::Start);
        }
        self.agent.shown = true;
        self.show_agent_chat();
    }

    /// Sends `text` to the agent (starting it first if need be).
    pub fn agent_prompt(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Nothing to ask".into());
        }
        if self.agent.status == AgentStatus::Busy {
            return Err("The agent is still working (:agent_cancel stops it)".into());
        }
        self.agent_open();
        self.agent.entries.push(ChatEntry::User(text.to_string()));
        self.agent.requests.push(AgentRequest::Prompt(text.to_string()));
        if self.agent.status == AgentStatus::Ready {
            self.agent.status = AgentStatus::Busy;
        }
        self.show_agent_chat();
        Ok(())
    }

    /// `:agent_cancel`: stops the turn (any question it asked, unanswered).
    pub fn agent_cancel(&mut self) {
        if let Some(ask) = self.agent.permission.take() {
            self.agent.requests.push(AgentRequest::Permission {
                request: ask.request,
                option: None,
            });
        }
        if self.agent.status == AgentStatus::Busy {
            self.agent.requests.push(AgentRequest::Cancel);
        }
        self.show_agent_chat();
    }

    /// `:agent_stop`: stops the agent.
    pub fn agent_stop(&mut self) {
        self.agent_cancel();
        if !matches!(self.agent.status, AgentStatus::Off | AgentStatus::Stopped(_)) {
            self.agent.requests.push(AgentRequest::Stop);
        }
    }

    /// The `n`th (from 1) of the options the agent offered, chosen.
    pub fn agent_choose(&mut self, n: usize) -> Result<(), String> {
        let ask = self.agent.permission.take().ok_or("The agent isn't asking anything")?;
        let Some(option) = n.checked_sub(1).and_then(|i| ask.options.get(i)) else {
            let count = ask.options.len();
            self.agent.permission = Some(ask);
            return Err(format!("Choose 1 to {count}"));
        };
        self.agent.entries.push(ChatEntry::Notice(format!("{}: {}", ask.title, option.name)));
        self.agent.requests.push(AgentRequest::Permission {
            request: ask.request.clone(),
            option: Some(option.id.clone()),
        });
        self.show_agent_chat();
        Ok(())
    }

    /// What the agent said or did: into the chat.
    pub fn agent_event(&mut self, event: AgentEvent) {
        let chat = &mut self.agent;
        match event {
            AgentEvent::Status(status) => {
                if let AgentStatus::Stopped(why) = &status {
                    chat.entries.push(ChatEntry::Notice(format!("The agent stopped: {why}")));
                    chat.permission = None;
                }
                chat.status = status;
            }
            AgentEvent::Started(info) => {
                chat.entries.push(ChatEntry::Notice(format!("{info} is ready")));
                chat.agent_info = Some(info);
            }
            AgentEvent::Text(text) => match chat.entries.last_mut() {
                Some(ChatEntry::Agent(said)) => said.push_str(&text),
                _ => chat.entries.push(ChatEntry::Agent(text)),
            },
            AgentEvent::Thought(text) => match chat.entries.last_mut() {
                Some(ChatEntry::Thought(said)) => said.push_str(&text),
                _ => chat.entries.push(ChatEntry::Thought(text)),
            },
            AgentEvent::ToolStarted(tool) => chat.entries.push(ChatEntry::Tool(tool)),
            AgentEvent::ToolUpdated {
                id,
                title,
                status,
                text,
                diffs,
            } => {
                let found = chat.entries.iter_mut().rev().find_map(|e| match e {
                    ChatEntry::Tool(tool) if tool.id == id => Some(tool),
                    _ => None,
                });
                if let Some(tool) = found {
                    if let Some(title) = title {
                        tool.title = title;
                    }
                    if let Some(status) = status {
                        tool.status = status;
                    }
                    if let Some(text) = text {
                        tool.text = text;
                    }
                    if let Some(diffs) = diffs {
                        tool.diffs = diffs;
                    }
                }
            }
            AgentEvent::Plan(steps) => {
                // One plan, kept up to date where it first showed.
                match chat.entries.iter_mut().rev().find(|e| matches!(e, ChatEntry::Plan(_))) {
                    Some(ChatEntry::Plan(old)) => *old = steps,
                    _ => chat.entries.push(ChatEntry::Plan(steps)),
                }
            }
            AgentEvent::Modes(modes, current) => {
                chat.modes = modes;
                chat.mode = current;
            }
            AgentEvent::Mode(mode) => chat.mode = Some(mode),
            AgentEvent::Commands(commands) => chat.commands = commands,
            AgentEvent::Permission(ask) => {
                chat.permission = Some(ask);
                // A question wants seeing.
                chat.shown = true;
            }
            AgentEvent::TurnEnded(reason) => {
                if chat.status == AgentStatus::Busy {
                    chat.status = AgentStatus::Ready;
                }
                chat.permission = None;
                let note = match reason.as_str() {
                    "end_turn" => None,
                    "cancelled" => Some("Stopped".to_string()),
                    "max_tokens" => Some("Stopped: it reached its output limit".into()),
                    "max_turn_requests" => Some("Stopped: it reached its limit of steps".into()),
                    "refusal" => Some("The agent declined to go on".into()),
                    other => Some(format!("Stopped ({other})")),
                };
                if let Some(note) = note {
                    chat.entries.push(ChatEntry::Notice(note));
                }
            }
            AgentEvent::Notice(text) => chat.entries.push(ChatEntry::Notice(text)),
        }
        self.show_agent_chat();
    }

    /// The chat as lines, for the popup (and, later, the panel).
    pub fn agent_chat_lines(&self) -> Vec<PopupLine> {
        let span = |text: &str, style: Option<&str>| PopupSpan {
            text: text.to_string(),
            style: style.map(str::to_string),
        };
        let mut lines: Vec<PopupLine> = Vec::new();
        let gap = |lines: &mut Vec<PopupLine>| {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(Vec::new());
            }
        };
        for entry in &self.agent.entries {
            match entry {
                ChatEntry::User(text) => {
                    gap(&mut lines);
                    for (i, line) in text.lines().enumerate() {
                        let mark = if i == 0 { "› " } else { "  " };
                        lines.push(vec![span(mark, Some("ui.popup.title")), span(line, Some("ui.popup.title"))]);
                    }
                    lines.push(Vec::new());
                }
                ChatEntry::Agent(text) => {
                    gap(&mut lines);
                    for line in text.lines() {
                        lines.push(vec![span(line, None)]);
                    }
                }
                ChatEntry::Thought(text) => {
                    for line in text.lines() {
                        lines.push(vec![span(line, Some("comment"))]);
                    }
                }
                ChatEntry::Tool(tool) => {
                    let (mark, style) = match tool.status {
                        ToolStatus::Pending => ("○", "ui.popup.detail"),
                        ToolStatus::Running => ("◐", "ui.popup.detail"),
                        ToolStatus::Done => ("✓", "diff.plus"),
                        ToolStatus::Failed => ("✗", "ui.decoration.error"),
                    };
                    gap(&mut lines);
                    lines.push(vec![span(&format!("{mark} "), Some(style)), span(&tool.title, Some("ui.popup.detail"))]);
                    for diff in &tool.diffs {
                        let added = diff.new.lines().count();
                        let removed = diff.old.as_deref().map_or(0, |o| o.lines().count());
                        lines.push(vec![span(&format!("    {}  +{added} -{removed}", diff.path), Some("comment"))]);
                    }
                }
                ChatEntry::Plan(steps) => {
                    gap(&mut lines);
                    for step in steps {
                        let mark = match step.status.as_str() {
                            "completed" => "☑",
                            "in_progress" => "▸",
                            _ => "☐",
                        };
                        lines.push(vec![span(&format!("{mark} {}", step.text), Some("ui.popup.detail"))]);
                    }
                    lines.push(Vec::new());
                }
                ChatEntry::Notice(text) => {
                    gap(&mut lines);
                    lines.push(vec![span(text, Some("comment"))]);
                }
            }
        }
        if let Some(ask) = &self.agent.permission {
            gap(&mut lines);
            lines.push(vec![span(&ask.title, Some("ui.popup.title"))]);
            for (i, option) in ask.options.iter().enumerate() {
                lines.push(vec![
                    span(&format!("  {}", i + 1), Some("ui.popup.title")),
                    span(&format!(" {}", option.name), None),
                ]);
            }
        }
        lines
    }

    /// Shows the chat popup again, if it was asked for (as things change).
    fn show_agent_chat(&mut self) {
        if !self.agent.shown {
            return;
        }
        // Not over another popup (it'll show next time).
        if self.popup().is_some_and(|p| p.name != POPUP) {
            return;
        }
        let name = if self.agent.agent.is_empty() {
            "Agent".to_string()
        } else {
            self.agent.agent.clone()
        };
        let state = match &self.agent.status {
            AgentStatus::Off => "",
            AgentStatus::Starting => " starting…",
            AgentStatus::Ready => "",
            AgentStatus::Busy => " working…",
            AgentStatus::Stopped(_) => " stopped",
        };
        let mode = self.agent.mode.as_ref().map(|m| format!(" ({m})")).unwrap_or_default();
        let mut lines = self.agent_chat_lines();
        if lines.is_empty() {
            lines.push(vec![PopupSpan {
                text: "Ask it something: i".into(),
                style: Some("comment".into()),
            }]);
        }
        let mut keys = vec![
            ("i".into(), "agent_ask".into()),
            ("c".into(), "agent_cancel".into()),
        ];
        let footer = if self.agent.permission.is_some() {
            for n in 1..=4 {
                keys.push((n.to_string(), format!("agent_option_{n}")));
            }
            " 1-4 answer  c cancel  Esc hide ".to_string()
        } else {
            " i ask  c cancel  Esc hide ".to_string()
        };
        let scroll = lines.len();
        self.open_popup(Popup {
            name: POPUP.into(),
            title: format!("{name}{mode}{state}"),
            lines,
            footer,
            focus: true,
            keys,
            on_close: Some("agent_hide".into()),
            ..Popup::default()
        });
        // Its end in sight.
        if let Some(popup) = self.popup.as_ref() {
            popup.scroll.set(scroll);
        }
    }

    /// The chat popup closed: not shown again until asked.
    pub fn agent_hide(&mut self) {
        self.agent.shown = false;
    }

    /// Whether the agent has things for the app to do (or is running).
    pub fn agent_wants_attention(&self) -> bool {
        !self.agent.requests.is_empty()
            || matches!(self.agent.status, AgentStatus::Starting | AgentStatus::Ready | AgentStatus::Busy)
    }

    /// The agent couldn't be started: why.
    pub fn agent_failed(&mut self, why: String) {
        self.set_message(Message::error(why.clone()));
        self.agent_event(AgentEvent::Status(AgentStatus::Stopped(why)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_streams_into_the_chat() {
        let mut e = Editor::default();
        e.options.agent = "claude".into();
        e.agent_prompt("fix it").unwrap();
        assert_eq!(e.take_agent_requests(), [AgentRequest::Start, AgentRequest::Prompt("fix it".into())]);
        e.agent_event(AgentEvent::Status(AgentStatus::Busy));
        e.agent_event(AgentEvent::Text("Looking".into()));
        e.agent_event(AgentEvent::Text(" now.".into()));
        e.agent_event(AgentEvent::ToolStarted(ToolCall {
            id: "t1".into(),
            title: "Read main.rs".into(),
            ..ToolCall::default()
        }));
        e.agent_event(AgentEvent::ToolUpdated {
            id: "t1".into(),
            title: None,
            status: Some(ToolStatus::Done),
            text: None,
            diffs: None,
        });
        e.agent_event(AgentEvent::TurnEnded("end_turn".into()));
        let text: Vec<String> = e
            .agent_chat_lines()
            .iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert!(text.contains(&"› fix it".to_string()), "{text:?}");
        assert!(text.contains(&"Looking now.".to_string()), "{text:?}");
        assert!(text.contains(&"✓ Read main.rs".to_string()), "{text:?}");
        assert_eq!(e.agent_chat().status, AgentStatus::Ready);
    }

    #[test]
    fn permission_is_asked_and_answered() {
        let mut e = Editor::default();
        e.agent_event(AgentEvent::Permission(PermissionAsk {
            request: serde_json::json!(7),
            title: "Run cargo test".into(),
            options: vec![
                PermissionOption {
                    id: "yes".into(),
                    name: "Allow".into(),
                    kind: "allow_once".into(),
                },
                PermissionOption {
                    id: "no".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                },
            ],
        }));
        assert!(e.agent_choose(3).is_err());
        e.agent_choose(2).unwrap();
        assert_eq!(
            e.take_agent_requests(),
            [AgentRequest::Permission {
                request: serde_json::json!(7),
                option: Some("no".into())
            }]
        );
    }
}
