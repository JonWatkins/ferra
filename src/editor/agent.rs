//! The coding agent's chat, the editor's side: what's been said and done
//! (the transcript), what the agent is asking, and what the editor wants
//! sent to it. The app's `agent` module talks to the agent (over ACP, the
//! Agent Client Protocol) and turns what it says into `AgentEvent`s here.
//!
//! The chat's shown in the agent sidebar, on the right of the windows
//! (`ui/agent.rs`): the transcript, the agent's question (its options as
//! buttons), and a box to type in, which has focus in `Mode::Agent`.

use std::cell::Cell;
use std::collections::HashSet;

use super::picker::{Picker, PickerAction, PickerItem};
use super::popup::PopupSpan;
use super::text_field::TextField;
use super::{Editor, Message, Mode};

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
    /// Stop the one running (quietly) and start the chat's agent.
    Restart,
    /// A message: what was typed, a note of what's open in the editor,
    /// and the files it mentions (`@src/main.rs`).
    Prompt {
        text: String,
        context: Option<String>,
        files: Vec<std::path::PathBuf>,
    },
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

/// A line of the transcript, before it's wrapped to the sidebar's width.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatLine {
    /// The text, each piece with its theme scope.
    pub spans: Vec<PopupSpan>,
    /// Columns its wrapped rows are indented by (under a bullet...).
    pub indent: u16,
    /// A scope the whole row's painted with (what you asked).
    pub fill: Option<String>,
    /// A click on it opens or closes this tool call.
    pub tool: Option<String>,
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
    /// The agents there are (`[agents]`), by name.
    pub available: Vec<String>,
    /// The sidebar's shown.
    pub open: bool,
    /// What's being typed to it.
    pub input: TextField,
    /// Rows scrolled up from the end (0 follows what comes in); the
    /// sidebar keeps it in range as it draws.
    pub scroll: Cell<usize>,
    /// Tool calls opened to show their output and changes.
    pub expanded: HashSet<String>,
    /// All of them opened (`<C-o>`).
    pub expand_all: bool,
    /// A file's being picked to mention (`@`): it goes in the box.
    pub(super) mentioning: bool,
    /// Buffers opened for the agent's tools, closed when its turn ends.
    pub(super) opened: HashSet<super::DocumentId>,
    requests: Vec<AgentRequest>,
}

impl AgentChat {
    /// The mode's name (as the agent calls it).
    pub fn mode_name(&self) -> Option<&str> {
        let id = self.mode.as_deref()?;
        Some(
            self.modes
                .iter()
                .find(|(m, _)| m == id)
                .map_or(id, |(_, name)| name.as_str()),
        )
    }

    /// The name shown for the agent: its own, else its `[agents]` name.
    pub fn title(&self) -> String {
        match &self.agent_info {
            Some(info) => info.clone(),
            None if self.agent.is_empty() => "Agent".into(),
            None => self.agent.clone(),
        }
    }

    fn running(&self) -> bool {
        !matches!(self.status, AgentStatus::Off | AgentStatus::Stopped(_))
    }

    fn is_expanded(&self, id: &str) -> bool {
        self.expand_all || self.expanded.contains(id)
    }
}

/// At most this many lines of a tool's output are shown, opened.
const TOOL_LINES: usize = 20;
/// And of each of its changes.
const DIFF_LINES: usize = 40;

impl Editor {
    pub fn agent_chat(&self) -> &AgentChat {
        &self.agent
    }

    /// What's to be sent to the agent, for the app.
    pub fn take_agent_requests(&mut self) -> Vec<AgentRequest> {
        std::mem::take(&mut self.agent.requests)
    }

    /// The agents there are to choose from (`[agents]`).
    pub fn set_agent_names(&mut self, mut names: Vec<String>) {
        names.sort();
        self.agent.available = names;
    }

    /// Whether the agent sidebar's shown.
    pub fn agent_sidebar_shown(&self) -> bool {
        self.agent.open
    }

    /// Starts the agent, if it isn't running.
    fn agent_start(&mut self) {
        if !self.agent.running() {
            if self.agent.agent.is_empty() {
                self.agent.agent = self.options.agent.clone();
            }
            self.agent.status = AgentStatus::Starting;
            self.agent.requests.push(AgentRequest::Start);
        }
    }

    /// `:agent`: shows the sidebar with its box focused, starting the
    /// agent if it isn't running.
    pub fn agent_open(&mut self) {
        self.agent_start();
        self.agent.open = true;
        self.set_mode(Mode::Agent);
    }

    /// `:agent_sidebar` (Ctrl+Alt+B): shows the sidebar (focused), or
    /// hides it.
    pub fn agent_toggle(&mut self) {
        if self.agent.open {
            self.agent_close();
        } else {
            self.agent_open();
        }
    }

    /// Hides the sidebar (the agent keeps running).
    pub fn agent_close(&mut self) {
        self.agent.open = false;
        if self.mode() == Mode::Agent {
            self.set_mode(Mode::Normal);
        }
    }

    /// Focus back to the text (the sidebar stays).
    pub fn agent_leave(&mut self) {
        if self.mode() == Mode::Agent {
            self.set_mode(Mode::Normal);
        }
    }

    /// `<CR>` in the box: sends what's typed.
    pub fn agent_send(&mut self) -> Result<(), String> {
        let text = self.agent.input.text().to_string();
        self.agent_prompt(&text)?;
        self.agent.input = TextField::default();
        Ok(())
    }

    /// Sends `text` to the agent (starting it first if need be).
    pub fn agent_prompt(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Nothing to ask".into());
        }
        if self.agent.status == AgentStatus::Busy {
            return Err("The agent is still working (Ctrl+C stops it)".into());
        }
        self.agent_start();
        self.agent.open = true;
        self.agent.entries.push(ChatEntry::User(text.to_string()));
        let request = AgentRequest::Prompt {
            text: text.to_string(),
            context: self.agent_context(),
            files: mentioned_files(text),
        };
        self.agent.requests.push(request);
        if self.agent.status == AgentStatus::Ready {
            self.agent.status = AgentStatus::Busy;
        }
        // What comes back, followed.
        self.agent.scroll.set(0);
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
    }

    /// `:agent_stop`: stops the agent.
    pub fn agent_stop(&mut self) {
        self.agent_cancel();
        if self.agent.running() {
            self.agent.requests.push(AgentRequest::Stop);
        }
    }

    /// The `n`th (from 1) of the options the agent offered, chosen.
    pub fn agent_choose(&mut self, n: usize) -> Result<(), String> {
        let ask = self
            .agent
            .permission
            .take()
            .ok_or("The agent isn't asking anything")?;
        let Some(option) = n.checked_sub(1).and_then(|i| ask.options.get(i)) else {
            let count = ask.options.len();
            self.agent.permission = Some(ask);
            return Err(format!("Choose 1 to {count}"));
        };
        self.agent
            .entries
            .push(ChatEntry::Notice(format!("{}: {}", ask.title, option.name)));
        self.agent.requests.push(AgentRequest::Permission {
            request: ask.request.clone(),
            option: Some(option.id.clone()),
        });
        Ok(())
    }

    /// Edits the box (`edit` is one of `TextField`'s).
    pub fn agent_input_edit(&mut self, edit: impl FnOnce(&mut TextField)) {
        edit(&mut self.agent.input);
    }

    /// A char typed in the box. With a question asked and nothing typed,
    /// a digit answers it.
    pub fn agent_type(&mut self, c: char) {
        if self.agent.permission.is_some()
            && self.agent.input.text().is_empty()
            && let Some(n) = c.to_digit(10).filter(|&n| n > 0)
        {
            if let Err(error) = self.agent_choose(n as usize) {
                self.set_message(Message::error(error));
            }
            return;
        }
        self.agent.input.push(c);
    }

    /// `@` in the box: the files to pick one to mention (its path goes in
    /// after the `@`, and the agent's sent the file with the message).
    pub fn agent_mention(&mut self) {
        let job = self.new_job_id();
        self.agent.input.push('@');
        self.agent.mentioning = true;
        self.open_picker(
            Picker::new("Mention a file", Vec::new())
                .loading_from(job)
                .with_placeholder("Search files by name"),
        );
        self.request_file_list(super::FileListRequest {
            job,
            root: std::path::PathBuf::from("."),
            base: None,
            options: self.options.file_picker.into(),
        });
    }

    /// The picker the mention was picked in, closed: the file chosen (if
    /// one was) goes in the box, which has focus again.
    pub(super) fn agent_mentioned(&mut self, action: Option<&PickerAction>) {
        self.agent.mentioning = false;
        if let Some(PickerAction::OpenFile(path)) = action {
            let path = path.to_string_lossy();
            let path = path.strip_prefix("./").unwrap_or(&path);
            path.chars().for_each(|c| self.agent.input.push(c));
            self.agent.input.push(' ');
        }
        self.agent.open = true;
        self.set_mode(Mode::Agent);
    }

    /// Text pasted into the box, newlines and all.
    pub fn agent_paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        text.chars().for_each(|c| self.agent.input.push(c));
    }

    /// Scrolls the chat: `rows` up (or down, negative).
    pub fn agent_scroll(&mut self, rows: isize) {
        let at = self.agent.scroll.get();
        self.agent.scroll.set(at.saturating_add_signed(rows));
    }

    /// Opens or closes a tool call (a click on it).
    pub fn agent_toggle_tool(&mut self, id: &str) {
        if self.agent.expand_all {
            // All open: this one closes, the others stay open.
            self.agent.expand_all = false;
            self.agent.expanded = self
                .agent
                .entries
                .iter()
                .filter_map(|e| match e {
                    ChatEntry::Tool(t) if t.id != id => Some(t.id.clone()),
                    _ => None,
                })
                .collect();
        } else if !self.agent.expanded.remove(id) {
            self.agent.expanded.insert(id.to_string());
        }
    }

    /// `<C-o>`: opens every tool call, or closes them all.
    pub fn agent_toggle_all_tools(&mut self) {
        self.agent.expand_all = !self.agent.expand_all;
        self.agent.expanded.clear();
    }

    /// `<S-Tab>`: the agent's next mode.
    pub fn agent_next_mode(&mut self) -> Result<(), String> {
        if self.agent.modes.is_empty() {
            return Err("The agent has no modes to switch between".into());
        }
        let at = self
            .agent
            .mode
            .as_ref()
            .and_then(|m| self.agent.modes.iter().position(|(id, _)| id == m));
        let next = at.map_or(0, |i| (i + 1) % self.agent.modes.len());
        let id = self.agent.modes[next].0.clone();
        self.agent_set_mode(&id)
    }

    /// Switches the agent to mode `id` (the agent says when it has).
    pub fn agent_set_mode(&mut self, id: &str) -> Result<(), String> {
        if !self.agent.modes.iter().any(|(m, _)| m == id) {
            return Err(format!("The agent has no mode \"{id}\""));
        }
        // Shown at once; the agent confirms.
        self.agent.mode = Some(id.to_string());
        self.agent
            .requests
            .push(AgentRequest::SetMode(id.to_string()));
        Ok(())
    }

    /// `:agent_mode`: a picker of the agent's modes.
    pub fn agent_mode_picker(&mut self) -> Result<(), String> {
        if self.agent.modes.is_empty() {
            return Err("The agent has no modes (or hasn't started)".into());
        }
        let items = self
            .agent
            .modes
            .iter()
            .map(|(id, name)| PickerItem {
                label: name.clone(),
                detail: if self.agent.mode.as_deref() == Some(id) {
                    "current".into()
                } else {
                    String::new()
                },
                action: PickerAction::RunWith("agent_set_mode".into(), vec![id.clone()]),
            })
            .collect();
        self.open_picker(Picker::new("Agent mode", items).keep_label_start());
        Ok(())
    }

    /// `:agent_choose`: a picker of the agents in `[agents]`.
    pub fn agent_picker(&mut self) -> Result<(), String> {
        if self.agent.available.is_empty() {
            return Err("No agents in [agents]".into());
        }
        let current = if self.agent.agent.is_empty() {
            self.options.agent.clone()
        } else {
            self.agent.agent.clone()
        };
        let items = self
            .agent
            .available
            .iter()
            .map(|name| PickerItem {
                label: name.clone(),
                detail: if *name == current {
                    "current".into()
                } else {
                    String::new()
                },
                action: PickerAction::RunWith("agent_use".into(), vec![name.clone()]),
            })
            .collect();
        self.open_picker(Picker::new("Agent", items).keep_label_start());
        Ok(())
    }

    /// Talks to agent `name` from now on (the one running stops; the chat
    /// so far stays, for reading).
    pub fn agent_use(&mut self, name: &str) -> Result<(), String> {
        if !self.agent.available.iter().any(|n| n == name) {
            return Err(format!("No agent named \"{name}\" in [agents]"));
        }
        self.options.agent = name.to_string();
        let same = self.agent.agent == name;
        self.agent.agent = name.to_string();
        if same && self.agent.running() {
            return Ok(());
        }
        if self.agent.running() {
            self.agent_cancel();
            self.agent
                .entries
                .push(ChatEntry::Notice(format!("Switched to {name}")));
            self.agent.requests.push(AgentRequest::Restart);
        } else {
            self.agent.requests.push(AgentRequest::Start);
        }
        self.agent.status = AgentStatus::Starting;
        self.agent.agent_info = None;
        self.agent.modes.clear();
        self.agent.mode = None;
        self.agent.commands.clear();
        self.agent.open = true;
        Ok(())
    }

    /// What the agent said or did: into the chat.
    pub fn agent_event(&mut self, event: AgentEvent) {
        let chat = &mut self.agent;
        match event {
            AgentEvent::Status(status) => {
                if let AgentStatus::Stopped(why) = &status {
                    chat.entries
                        .push(ChatEntry::Notice(format!("The agent stopped: {why}")));
                    chat.permission = None;
                }
                chat.status = status;
            }
            AgentEvent::Started(info) => {
                chat.entries
                    .push(ChatEntry::Notice(format!("{info} is ready")));
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
                match chat
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, ChatEntry::Plan(_)))
                {
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
                // A question wants seeing (focus stays where it is).
                let title = ask.title.clone();
                chat.permission = Some(ask);
                chat.open = true;
                chat.scroll.set(0);
                if self.mode() != Mode::Agent {
                    self.set_message(Message::info(format!(
                        "The agent asks: {title} (answer in its sidebar)"
                    )));
                }
            }
            AgentEvent::TurnEnded(reason) => {
                // What it opened to read or ask about goes.
                self.agent_close_its_buffers();
                let chat = &mut self.agent;
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
    }

    /// The transcript as lines, for the sidebar (which wraps them).
    pub fn agent_transcript(&self) -> Vec<ChatLine> {
        let chat = &self.agent;
        let mut lines: Vec<ChatLine> = Vec::new();
        let gap = |lines: &mut Vec<ChatLine>| {
            if lines.last().is_some_and(|l| !l.spans.is_empty()) {
                lines.push(ChatLine::default());
            }
        };
        for entry in &chat.entries {
            match entry {
                ChatEntry::User(text) => {
                    gap(&mut lines);
                    let fill = Some("ui.sidebar.agent.user".to_string());
                    for (i, line) in text.lines().enumerate() {
                        let mark = if i == 0 { "› " } else { "  " };
                        lines.push(ChatLine {
                            spans: vec![
                                span(mark, "ui.sidebar.agent.user.mark"),
                                span(line, "ui.sidebar.agent.user"),
                            ],
                            indent: 2,
                            fill: fill.clone(),
                            tool: None,
                        });
                    }
                }
                ChatEntry::Agent(text) => {
                    gap(&mut lines);
                    markdown_lines(text, &mut lines);
                }
                ChatEntry::Thought(text) => {
                    gap(&mut lines);
                    for line in text.lines().filter(|l| !l.trim().is_empty()) {
                        lines.push(ChatLine {
                            spans: vec![span(line, "ui.sidebar.agent.thought")],
                            ..ChatLine::default()
                        });
                    }
                }
                ChatEntry::Tool(tool) => {
                    if !matches!(lines.last(), Some(l) if l.tool.is_some()) {
                        gap(&mut lines);
                    }
                    tool_lines(tool, chat.is_expanded(&tool.id), &mut lines);
                }
                ChatEntry::Plan(steps) => {
                    gap(&mut lines);
                    lines.push(ChatLine {
                        spans: vec![span("Plan", "ui.sidebar.agent.heading")],
                        ..ChatLine::default()
                    });
                    for step in steps {
                        let (mark, style) = match step.status.as_str() {
                            "completed" => ("☑ ", "ui.sidebar.agent.plan.done"),
                            "in_progress" => ("▸ ", "ui.sidebar.agent.plan.current"),
                            _ => ("☐ ", "ui.sidebar.agent.plan"),
                        };
                        lines.push(ChatLine {
                            spans: vec![span(mark, style), span(&step.text, style)],
                            indent: 2,
                            ..ChatLine::default()
                        });
                    }
                }
                ChatEntry::Notice(text) => {
                    gap(&mut lines);
                    lines.push(ChatLine {
                        spans: vec![span(text, "ui.sidebar.agent.notice")],
                        ..ChatLine::default()
                    });
                }
            }
        }
        lines
    }

    /// Whether the agent has things for the app to do (or is running).
    pub fn agent_wants_attention(&self) -> bool {
        !self.agent.requests.is_empty()
            || matches!(
                self.agent.status,
                AgentStatus::Starting | AgentStatus::Ready | AgentStatus::Busy
            )
    }

    /// The agent couldn't be started: why.
    pub fn agent_failed(&mut self, why: String) {
        self.set_message(Message::error(why.clone()));
        self.agent_event(AgentEvent::Status(AgentStatus::Stopped(why)));
    }
}

fn span(text: &str, style: &str) -> PopupSpan {
    PopupSpan {
        text: text.to_string(),
        style: Some(style.to_string()),
    }
}

/// A tool call: its title (a click opens it), what it changed, and opened,
/// its output and the changes line by line.
fn tool_lines(tool: &ToolCall, expanded: bool, lines: &mut Vec<ChatLine>) {
    let (mark, style) = match tool.status {
        ToolStatus::Pending => ("○ ", "ui.sidebar.agent.tool.pending"),
        ToolStatus::Running => ("◐ ", "ui.sidebar.agent.tool.running"),
        ToolStatus::Done => ("✓ ", "ui.sidebar.agent.tool.done"),
        ToolStatus::Failed => ("✗ ", "ui.sidebar.agent.tool.failed"),
    };
    let more = !tool.text.trim().is_empty() || !tool.diffs.is_empty();
    let mut spans = vec![
        span(mark, style),
        span(&tool.title, "ui.sidebar.agent.tool"),
    ];
    if more {
        let arrow = if expanded { " ▾" } else { " ▸" };
        spans.push(span(arrow, "ui.sidebar.agent.hint"));
    }
    let id = Some(tool.id.clone());
    lines.push(ChatLine {
        spans,
        indent: 2,
        fill: None,
        tool: id.clone(),
    });
    for diff in &tool.diffs {
        let (added, removed) = diff_counts(diff);
        lines.push(ChatLine {
            spans: vec![
                span(
                    &format!("  {} ", short_path(&diff.path)),
                    "ui.sidebar.agent.tool",
                ),
                span(&format!("+{added}"), "ui.sidebar.agent.added"),
                span(" ", "ui.sidebar.agent.tool"),
                span(&format!("-{removed}"), "ui.sidebar.agent.removed"),
            ],
            indent: 4,
            fill: None,
            tool: id.clone(),
        });
    }
    if !expanded {
        return;
    }
    let output: Vec<&str> = tool.text.lines().collect();
    for line in output.iter().take(TOOL_LINES) {
        lines.push(ChatLine {
            spans: vec![span(&format!("  {line}"), "ui.sidebar.agent.output")],
            indent: 2,
            ..ChatLine::default()
        });
    }
    if output.len() > TOOL_LINES {
        lines.push(ChatLine {
            spans: vec![span(
                &format!("  … {} more lines", output.len() - TOOL_LINES),
                "ui.sidebar.agent.hint",
            )],
            ..ChatLine::default()
        });
    }
    for diff in &tool.diffs {
        diff_lines(diff, lines);
    }
}

/// Lines added and removed by a change.
fn diff_counts(diff: &ToolDiff) -> (usize, usize) {
    let old = diff.old.as_deref().unwrap_or_default();
    let mut counts = (0, 0);
    for change in similar::TextDiff::from_lines(old, &diff.new).iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => counts.0 += 1,
            similar::ChangeTag::Delete => counts.1 += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    counts
}

/// A change line by line: what was taken out and put in, with a line
/// around each.
fn diff_lines(diff: &ToolDiff, lines: &mut Vec<ChatLine>) {
    let old = diff.old.as_deref().unwrap_or_default();
    let text = similar::TextDiff::from_lines(old, &diff.new);
    lines.push(ChatLine {
        spans: vec![span(
            &format!("  {}", short_path(&diff.path)),
            "ui.sidebar.agent.heading",
        )],
        ..ChatLine::default()
    });
    let mut shown = 0;
    let mut left = 0;
    for (i, group) in text.grouped_ops(1).iter().enumerate() {
        if i > 0 && shown < DIFF_LINES {
            lines.push(ChatLine {
                spans: vec![span("  ⋯", "ui.sidebar.agent.hint")],
                ..ChatLine::default()
            });
        }
        for op in group {
            for change in text.iter_changes(op) {
                if shown >= DIFF_LINES {
                    left += 1;
                    continue;
                }
                shown += 1;
                let (sign, style) = match change.tag() {
                    similar::ChangeTag::Insert => ("+ ", "ui.sidebar.agent.added"),
                    similar::ChangeTag::Delete => ("- ", "ui.sidebar.agent.removed"),
                    similar::ChangeTag::Equal => ("  ", "ui.sidebar.agent.output"),
                };
                let line = change.value().trim_end_matches(['\n', '\r']);
                lines.push(ChatLine {
                    spans: vec![span(&format!("  {sign}{line}"), style)],
                    indent: 4,
                    ..ChatLine::default()
                });
            }
        }
    }
    if left > 0 {
        lines.push(ChatLine {
            spans: vec![span(
                &format!("  … {left} more lines"),
                "ui.sidebar.agent.hint",
            )],
            ..ChatLine::default()
        });
    }
}

/// The files `text` mentions, as `@path` (that are files, from the
/// project's folder).
fn mentioned_files(text: &str) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for word in text.split_whitespace() {
        let Some(path) = word.strip_prefix('@') else {
            continue;
        };
        let path = path.trim_end_matches([',', '.', ';', ':', ')', '?', '!']);
        let full = std::env::current_dir().unwrap_or_default().join(path);
        if full.is_file() && !files.contains(&full) {
            files.push(full);
        }
    }
    files
}

/// A path as it's shown: relative to the project's folder, if it's in it.
pub(crate) fn short_path(path: &str) -> String {
    let path = std::path::Path::new(path);
    std::env::current_dir()
        .ok()
        .and_then(|here| std::fs::canonicalize(&here).ok().or(Some(here)))
        .and_then(|here| {
            path.strip_prefix(here)
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_else(|| path.display().to_string())
}

/// What the agent said, as lines: Markdown's headings, lists, code blocks,
/// `code` and **bold** styled; the rest as it is.
fn markdown_lines(text: &str, lines: &mut Vec<ChatLine>) {
    let mut in_code = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(ChatLine {
                spans: vec![span(&format!("  {line}"), "ui.sidebar.agent.code")],
                indent: 2,
                ..ChatLine::default()
            });
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            lines.push(ChatLine {
                spans: vec![span(heading, "ui.sidebar.agent.heading")],
                ..ChatLine::default()
            });
            continue;
        }
        let lead = line.len() - trimmed.len();
        let bullet = ["- ", "* ", "+ "]
            .iter()
            .find_map(|b| trimmed.strip_prefix(b));
        let (mark, rest, indent) = match bullet {
            Some(rest) => (format!("{}• ", " ".repeat(lead)), rest, lead + 2),
            None => {
                // `1. `: kept, with what follows indented under it.
                let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
                if digits > 0 && trimmed[digits..].starts_with(". ") {
                    let mark = &line[..lead + digits + 2];
                    (
                        mark.to_string(),
                        &trimmed[digits + 2..],
                        mark.chars().count(),
                    )
                } else {
                    (String::new(), line, 0)
                }
            }
        };
        let mut spans = Vec::new();
        if !mark.is_empty() {
            spans.push(span(&mark, "ui.sidebar.agent.bullet"));
        }
        spans.extend(inline_spans(rest));
        lines.push(ChatLine {
            spans,
            indent: indent as u16,
            ..ChatLine::default()
        });
    }
}

/// A line's `code` and **bold** pieces, styled.
fn inline_spans(text: &str) -> Vec<PopupSpan> {
    let mut spans = Vec::new();
    let ticks = text.matches('`').count();
    for (i, piece) in text.split('`').enumerate() {
        if piece.is_empty() {
            continue;
        }
        // Between backticks (an unclosed one's text is kept as it is).
        if i % 2 == 1 && i < ticks {
            spans.push(span(piece, "ui.sidebar.agent.code.inline"));
            continue;
        }
        let piece = if i % 2 == 1 {
            format!("`{piece}")
        } else {
            piece.to_string()
        };
        for (j, part) in piece.split("**").enumerate() {
            if part.is_empty() {
                continue;
            }
            let style = if j % 2 == 1 {
                "ui.sidebar.agent.bold"
            } else {
                "ui.sidebar.agent.text"
            };
            spans.push(span(part, style));
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(lines: &[ChatLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn a_turn_streams_into_the_chat() {
        let mut e = Editor::default();
        e.options.agent = "claude".into();
        e.agent_prompt("fix it").unwrap();
        assert_eq!(
            e.take_agent_requests(),
            [
                AgentRequest::Start,
                AgentRequest::Prompt {
                    text: "fix it".into(),
                    context: None,
                    files: Vec::new()
                }
            ]
        );
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
        let text = text_of(&e.agent_transcript());
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
        assert!(e.agent_sidebar_shown(), "a question shows the sidebar");
        assert!(e.agent_choose(3).is_err());
        // A digit typed with nothing else typed answers.
        e.agent_type('2');
        assert_eq!(e.agent_chat().input.text(), "");
        assert_eq!(
            e.take_agent_requests(),
            [AgentRequest::Permission {
                request: serde_json::json!(7),
                option: Some("no".into())
            }]
        );
    }

    #[test]
    fn tool_calls_open_to_show_their_changes() {
        let mut e = Editor::default();
        e.agent_event(AgentEvent::ToolStarted(ToolCall {
            id: "t1".into(),
            title: "Edit a.rs".into(),
            status: ToolStatus::Done,
            diffs: vec![ToolDiff {
                path: "a.rs".into(),
                old: Some("one\ntwo\nthree\n".into()),
                new: "one\nTWO\nthree\n".into(),
            }],
            ..ToolCall::default()
        }));
        let closed = text_of(&e.agent_transcript());
        assert_eq!(closed, ["✓ Edit a.rs ▸", "  a.rs +1 -1"]);
        e.agent_toggle_tool("t1");
        let open = text_of(&e.agent_transcript());
        assert!(open.contains(&"  - two".to_string()), "{open:?}");
        assert!(open.contains(&"  + TWO".to_string()), "{open:?}");
        assert_eq!(open[0], "✓ Edit a.rs ▾");
        e.agent_toggle_tool("t1");
        assert_eq!(text_of(&e.agent_transcript()), closed);
    }

    #[test]
    fn markdown_is_styled() {
        let mut lines = Vec::new();
        markdown_lines(
            "# Title\nUse `cargo` **now**\n- one\n```\nlet x;\n```",
            &mut lines,
        );
        let text = text_of(&lines);
        assert_eq!(text, ["Title", "Use cargo now", "• one", "  let x;"]);
        assert_eq!(
            lines[1].spans[1].style.as_deref(),
            Some("ui.sidebar.agent.code.inline")
        );
        assert_eq!(
            lines[1].spans[3].style.as_deref(),
            Some("ui.sidebar.agent.bold")
        );
        assert_eq!(lines[2].indent, 2);
    }

    #[test]
    fn modes_cycle_and_agents_switch() {
        let mut e = Editor::default();
        e.set_agent_names(vec!["gemini".into(), "claude".into()]);
        e.agent_event(AgentEvent::Modes(
            vec![
                ("default".into(), "Default".into()),
                ("plan".into(), "Plan".into()),
            ],
            Some("default".into()),
        ));
        e.agent_next_mode().unwrap();
        assert_eq!(e.agent_chat().mode_name(), Some("Plan"));
        e.agent_next_mode().unwrap();
        assert_eq!(e.agent_chat().mode.as_deref(), Some("default"));
        assert!(e.agent_use("nope").is_err());
        e.agent_use("gemini").unwrap();
        assert_eq!(e.options.agent, "gemini");
        let requests = e.take_agent_requests();
        assert_eq!(requests.last(), Some(&AgentRequest::Start));
    }
}
