//! The chat with the coding agent: what's been said and done (the
//! transcript's entries), what the agent is asking, the box being typed
//! in, and what's to be sent to it (`AgentRequest`s, taken by the app's
//! `agent` module, which turns what the agent says into `AgentEvent`s
//! here). Only the chat: what it means for the rest of the editor (focus,
//! pickers, the message line, the buffers its tools opened) is the glue in
//! `mod.rs`.

use std::cell::Cell;
use std::collections::HashSet;

use super::session::{AgentCommand, ConfigOption, Usage};
use crate::editor::picker::{Picker, PickerAction, PickerItem};
use crate::editor::text_field::TextField;

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
        /// Images put in the box: their type (`image/png`) and data.
        images: Vec<(String, Vec<u8>)>,
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
    /// Set one of the session's settings (`session/set_config_option`).
    SetConfig {
        id: String,
        value: serde_json::Value,
    },
    /// A new session, in place of the one there is (`/clear`).
    NewSession,
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
    /// Its slash commands.
    Commands(Vec<AgentCommand>),
    /// The session's settings (all of them, each time).
    Config(Vec<ConfigOption>),
    /// How much of its context it's used.
    Usage(Usage),
    /// Whether it takes images in a message.
    TakesImages(bool),
    Permission(PermissionAsk),
    /// The turn ended: why (`end_turn`, `cancelled`, `refusal`...).
    TurnEnded(String),
    Notice(String),
}

/// An image put in the agent's box (pasted or dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Its number, shown in the box as `[Image #n]`.
    pub number: usize,
    /// `image/png`, `image/jpeg`...
    pub mime: String,
    pub data: Vec<u8>,
}

impl Image {
    pub fn label(&self) -> String {
        format!("[Image #{}]", self.number)
    }
}

/// What an event means beyond the chat, for the editor to see to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Followup {
    /// The agent asks something (its title): said on the message line,
    /// unless the box has focus.
    Asked(String),
    /// Its turn ended: what it opened for its tools goes.
    TurnEnded,
}

/// The chat with the agent.
#[derive(Debug, Clone, Default)]
pub struct AgentChat {
    /// Which agent (its name in `[agents]`).
    pub(super) agent: String,
    /// Its own name and version, once it's said.
    pub(super) agent_info: Option<String>,
    pub(super) status: AgentStatus,
    pub(super) entries: Vec<ChatEntry>,
    pub(super) permission: Option<PermissionAsk>,
    pub(super) modes: Vec<(String, String)>,
    pub(super) mode: Option<String>,
    /// The agent's own slash commands.
    pub(super) commands: Vec<AgentCommand>,
    /// The session's settings: model, effort...
    pub(super) config: Vec<ConfigOption>,
    /// How much of its context it's used, once it's said.
    pub(super) usage: Option<Usage>,
    /// The slash menu's selected row.
    pub(super) slash_selected: usize,
    /// What was in the box when the slash menu was put away (`<Esc>`).
    pub(super) slash_closed: Option<String>,
    /// The agents there are (`[agents]`), by name.
    pub(super) available: Vec<String>,
    /// The sidebar's shown.
    pub(super) open: bool,
    /// What's being typed to it.
    pub(super) input: TextField,
    /// Rows scrolled up from the end (0 follows what comes in); the
    /// sidebar keeps it in range as it draws.
    pub(super) scroll: Cell<usize>,
    /// Tool calls opened to show their output and changes.
    pub(super) expanded: HashSet<String>,
    /// All of them opened (`<C-o>`).
    pub(super) expand_all: bool,
    /// A file's being picked to mention (`@`): it goes in the box.
    pub(super) mentioning: bool,
    /// Whether the agent takes images (it says as it starts).
    pub(super) takes_images: Option<bool>,
    /// Images put in the box, each shown there as `[Image #n]`.
    pub(super) images: Vec<Image>,
    pub(super) requests: Vec<AgentRequest>,
}

/// What it holds, for the sidebar (and tests).
impl AgentChat {
    /// Which agent (its name in `[agents]`), once one's started.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Its own name and version, once it's said.
    pub fn agent_info(&self) -> Option<&str> {
        self.agent_info.as_deref()
    }

    pub fn status(&self) -> &AgentStatus {
        &self.status
    }

    pub fn entries(&self) -> &[ChatEntry] {
        &self.entries
    }

    /// What it's asking, if anything.
    pub fn permission(&self) -> Option<&PermissionAsk> {
        self.permission.as_ref()
    }

    /// Its modes: id and name.
    pub fn modes(&self) -> &[(String, String)] {
        &self.modes
    }

    /// Its mode's id.
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

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

    /// The agent's own slash commands (`slash_commands` adds ferra's).
    pub fn commands(&self) -> &[AgentCommand] {
        &self.commands
    }

    /// The session's settings: model, effort...
    pub fn config(&self) -> &[ConfigOption] {
        &self.config
    }

    /// How much of its context it's used, once it's said.
    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }

    /// The box.
    pub fn input(&self) -> &TextField {
        &self.input
    }

    /// Rows scrolled up from the end; the sidebar keeps it in range.
    pub fn scrolled(&self) -> &Cell<usize> {
        &self.scroll
    }

    /// The images in the box.
    pub fn images(&self) -> &[Image] {
        &self.images
    }

    /// Whether the sidebar's shown.
    pub fn shown(&self) -> bool {
        self.open
    }

    /// Whether a file's being picked to mention.
    pub fn mentioning(&self) -> bool {
        self.mentioning
    }

    /// Whether it has things for the app to do (or is running).
    pub fn wants_attention(&self) -> bool {
        !self.requests.is_empty()
            || matches!(
                self.status,
                AgentStatus::Starting | AgentStatus::Ready | AgentStatus::Busy
            )
    }

    pub(super) fn running(&self) -> bool {
        !matches!(self.status, AgentStatus::Off | AgentStatus::Stopped(_))
    }

    pub(super) fn is_expanded(&self, id: &str) -> bool {
        self.expand_all || self.expanded.contains(id)
    }
}

impl AgentChat {
    /// What's to be sent to the agent, for the app.
    pub fn take_requests(&mut self) -> Vec<AgentRequest> {
        std::mem::take(&mut self.requests)
    }

    /// The agents there are to choose from (`[agents]`).
    pub fn set_names(&mut self, mut names: Vec<String>) {
        names.sort();
        self.available = names;
    }

    /// Shows or hides the sidebar.
    pub(super) fn show(&mut self, shown: bool) {
        self.open = shown;
    }

    /// Starts the agent, if it isn't running: the one chosen, else
    /// `default` (the editor option `agent`).
    pub(super) fn start(&mut self, default: &str) {
        if !self.running() {
            if self.agent.is_empty() {
                self.agent = default.to_string();
            }
            self.status = AgentStatus::Starting;
            self.requests.push(AgentRequest::Start);
        }
    }

    /// Sends `text` (trimmed, not empty) with the editor's `context`,
    /// starting the agent first if need be (`default` as in `start`).
    pub(super) fn prompt(
        &mut self,
        text: &str,
        context: Option<String>,
        default: &str,
    ) -> Result<(), String> {
        if self.status == AgentStatus::Busy {
            return Err("The agent is still working (Ctrl+C stops it)".into());
        }
        self.start(default);
        self.open = true;
        self.entries.push(ChatEntry::User(text.to_string()));
        // The images whose `[Image #n]` is still in what's sent.
        let images = std::mem::take(&mut self.images)
            .into_iter()
            .filter(|image| text.contains(&image.label()))
            .map(|image| (image.mime, image.data))
            .collect();
        self.requests.push(AgentRequest::Prompt {
            text: text.to_string(),
            context,
            files: mentioned_files(text),
            images,
        });
        if self.status == AgentStatus::Ready {
            self.status = AgentStatus::Busy;
        }
        // What comes back, followed.
        self.scroll.set(0);
        Ok(())
    }

    /// Empties the box (what was in it, sent).
    pub(super) fn clear_input(&mut self) {
        self.input = TextField::default();
    }

    /// `:agent_cancel`: stops the turn (any question it asked, unanswered).
    pub fn cancel(&mut self) {
        if let Some(ask) = self.permission.take() {
            self.requests.push(AgentRequest::Permission {
                request: ask.request,
                option: None,
            });
        }
        if self.status == AgentStatus::Busy {
            self.requests.push(AgentRequest::Cancel);
        }
    }

    /// `:agent_stop`: stops the agent.
    pub fn stop(&mut self) {
        self.cancel();
        if self.running() {
            self.requests.push(AgentRequest::Stop);
        }
    }

    /// The `n`th (from 1) of the options the agent offered, chosen.
    pub fn choose(&mut self, n: usize) -> Result<(), String> {
        let ask = self
            .permission
            .take()
            .ok_or("The agent isn't asking anything")?;
        let Some(option) = n.checked_sub(1).and_then(|i| ask.options.get(i)) else {
            let count = ask.options.len();
            self.permission = Some(ask);
            return Err(format!("Choose 1 to {count}"));
        };
        self.entries
            .push(ChatEntry::Notice(format!("{}: {}", ask.title, option.name)));
        self.requests.push(AgentRequest::Permission {
            request: ask.request.clone(),
            option: Some(option.id.clone()),
        });
        Ok(())
    }

    /// Edits the box (`edit` is one of `TextField`'s).
    pub fn edit_input(&mut self, edit: impl FnOnce(&mut TextField)) {
        edit(&mut self.input);
        self.slash_selected = 0;
    }

    /// A char typed in the box. With a question asked and nothing typed,
    /// a digit answers it.
    pub fn type_char(&mut self, c: char) -> Result<(), String> {
        if self.permission.is_some()
            && self.input.text().is_empty()
            && let Some(n) = c.to_digit(10).filter(|&n| n > 0)
        {
            return self.choose(n as usize);
        }
        self.input.push(c);
        self.slash_selected = 0;
        Ok(())
    }

    /// Types `text` into the box, as it is.
    fn type_text(&mut self, text: &str) {
        text.chars().for_each(|c| self.input.push(c));
    }

    /// `@` typed: a file's to be picked to mention.
    pub(super) fn begin_mention(&mut self) {
        self.input.push('@');
        self.mentioning = true;
    }

    /// The file to mention picked (`path`), or not: its path goes in
    /// after the `@`, and the sidebar's shown.
    pub(super) fn end_mention(&mut self, path: Option<&str>) {
        self.mentioning = false;
        if let Some(path) = path {
            self.type_text(path.strip_prefix("./").unwrap_or(path));
            self.input.push(' ');
        }
        self.open = true;
    }

    /// Text pasted into the box, newlines and all. Files dropped on the
    /// terminal (which pastes their paths) go in as images, if they are
    /// some, or mentions (`@path`). An image that can't be read (or
    /// taken) is said; the rest still go in.
    pub fn paste(&mut self, text: &str) -> Result<(), String> {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let Some(paths) = dropped_files(&text) else {
            self.type_text(&text);
            return Ok(());
        };
        let mut failed = Ok(());
        for path in paths {
            if let Some(mime) = image_type(&path) {
                let attached = std::fs::read(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))
                    .and_then(|data| self.attach(mime, data));
                if attached.is_err() {
                    failed = attached;
                }
            } else {
                let shown = short_path(&path.to_string_lossy());
                self.type_text(&format!("@{shown} "));
            }
        }
        failed
    }

    /// An image for the next message: `[Image #n]` goes in the box.
    pub fn attach(&mut self, mime: &str, data: Vec<u8>) -> Result<(), String> {
        /// The most an image can be (the agents' own limit is about this).
        const MOST: usize = 5 * 1024 * 1024;
        if self.takes_images == Some(false) {
            return Err(format!("{} doesn't take images", self.title()));
        }
        if data.len() > MOST {
            return Err(format!(
                "That image is {:.1} MB: the most is 5 MB",
                data.len() as f64 / (1024.0 * 1024.0)
            ));
        }
        let number = self.images.iter().map(|i| i.number).max().unwrap_or(0) + 1;
        let image = Image {
            number,
            mime: mime.to_string(),
            data,
        };
        self.type_text(&format!("{} ", image.label()));
        self.images.push(image);
        Ok(())
    }

    /// Scrolls the chat: `rows` up (or down, negative).
    pub fn scroll_by(&mut self, rows: isize) {
        let at = self.scroll.get();
        self.scroll.set(at.saturating_add_signed(rows));
    }

    /// Opens or closes a tool call (a click on it).
    pub fn toggle_tool(&mut self, id: &str) {
        if self.expand_all {
            // All open: this one closes, the others stay open.
            self.expand_all = false;
            self.expanded = self
                .entries
                .iter()
                .filter_map(|e| match e {
                    ChatEntry::Tool(t) if t.id != id => Some(t.id.clone()),
                    _ => None,
                })
                .collect();
        } else if !self.expanded.remove(id) {
            self.expanded.insert(id.to_string());
        }
    }

    /// `<C-o>`: opens every tool call, or closes them all.
    pub fn toggle_all_tools(&mut self) {
        self.expand_all = !self.expand_all;
        self.expanded.clear();
    }

    /// `<S-Tab>`: the agent's next mode.
    pub fn next_mode(&mut self) -> Result<(), String> {
        if self.modes.is_empty() {
            return Err("The agent has no modes to switch between".into());
        }
        let at = self
            .mode
            .as_ref()
            .and_then(|m| self.modes.iter().position(|(id, _)| id == m));
        let next = at.map_or(0, |i| (i + 1) % self.modes.len());
        let id = self.modes[next].0.clone();
        self.set_mode(&id)
    }

    /// Switches the agent to mode `id` (the agent says when it has).
    pub fn set_mode(&mut self, id: &str) -> Result<(), String> {
        if !self.modes.iter().any(|(m, _)| m == id) {
            return Err(format!("The agent has no mode \"{id}\""));
        }
        // Shown at once; the agent confirms.
        self.mode = Some(id.to_string());
        self.requests.push(AgentRequest::SetMode(id.to_string()));
        Ok(())
    }

    /// `:agent_mode`: a picker of the agent's modes.
    pub(super) fn mode_picker(&self) -> Result<Picker, String> {
        if self.modes.is_empty() {
            return Err("The agent has no modes (or hasn't started)".into());
        }
        let items = self
            .modes
            .iter()
            .map(|(id, name)| PickerItem {
                label: name.clone(),
                detail: if self.mode.as_deref() == Some(id) {
                    "current".into()
                } else {
                    String::new()
                },
                action: PickerAction::RunWith("agent_set_mode".into(), vec![id.clone()]),
            })
            .collect();
        Ok(Picker::new("Agent mode", items).keep_label_start())
    }

    /// `:agent_choose`: a picker of the agents in `[agents]` (`default`
    /// is current if none's been started).
    pub(super) fn agent_picker(&self, default: &str) -> Result<Picker, String> {
        if self.available.is_empty() {
            return Err("No agents in [agents]".into());
        }
        let current = if self.agent.is_empty() {
            default
        } else {
            &self.agent
        };
        let items = self
            .available
            .iter()
            .map(|name| PickerItem {
                label: name.clone(),
                detail: if name == current {
                    "current".into()
                } else {
                    String::new()
                },
                action: PickerAction::RunWith("agent_use".into(), vec![name.clone()]),
            })
            .collect();
        Ok(Picker::new("Agent", items).keep_label_start())
    }

    /// Talks to agent `name` from now on (the one running stops; the chat
    /// so far stays, for reading).
    pub(super) fn use_agent(&mut self, name: &str) -> Result<(), String> {
        if !self.available.iter().any(|n| n == name) {
            return Err(format!("No agent named \"{name}\" in [agents]"));
        }
        let same = self.agent == name;
        self.agent = name.to_string();
        if same && self.running() {
            return Ok(());
        }
        if self.running() {
            self.cancel();
            self.entries
                .push(ChatEntry::Notice(format!("Switched to {name}")));
            self.requests.push(AgentRequest::Restart);
        } else {
            self.requests.push(AgentRequest::Start);
        }
        self.status = AgentStatus::Starting;
        self.agent_info = None;
        self.modes.clear();
        self.mode = None;
        self.commands.clear();
        self.config.clear();
        self.usage = None;
        self.open = true;
        Ok(())
    }

    /// What the agent said or did: into the chat. Returns what it means
    /// beyond it, if anything.
    pub(super) fn event(&mut self, event: AgentEvent) -> Option<Followup> {
        match event {
            AgentEvent::Status(status) => {
                if let AgentStatus::Stopped(why) = &status {
                    self.entries
                        .push(ChatEntry::Notice(format!("The agent stopped: {why}")));
                    self.permission = None;
                }
                self.status = status;
            }
            AgentEvent::Started(info) => {
                self.entries
                    .push(ChatEntry::Notice(format!("{info} is ready")));
                self.agent_info = Some(info);
            }
            AgentEvent::Text(text) => match self.entries.last_mut() {
                Some(ChatEntry::Agent(said)) => said.push_str(&text),
                _ => self.entries.push(ChatEntry::Agent(text)),
            },
            AgentEvent::Thought(text) => match self.entries.last_mut() {
                Some(ChatEntry::Thought(said)) => said.push_str(&text),
                _ => self.entries.push(ChatEntry::Thought(text)),
            },
            AgentEvent::ToolStarted(tool) => self.entries.push(ChatEntry::Tool(tool)),
            AgentEvent::ToolUpdated {
                id,
                title,
                status,
                text,
                diffs,
            } => {
                let found = self.entries.iter_mut().rev().find_map(|e| match e {
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
                match self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, ChatEntry::Plan(_)))
                {
                    Some(ChatEntry::Plan(old)) => *old = steps,
                    _ => self.entries.push(ChatEntry::Plan(steps)),
                }
            }
            AgentEvent::Modes(modes, current) => {
                self.modes = modes;
                self.mode = current;
            }
            AgentEvent::Mode(mode) => self.mode = Some(mode),
            AgentEvent::Commands(commands) => self.commands = commands,
            AgentEvent::Config(options) => self.config = options,
            AgentEvent::Usage(usage) => self.usage = Some(usage),
            AgentEvent::TakesImages(takes) => self.takes_images = Some(takes),
            AgentEvent::Permission(ask) => {
                // A question wants seeing (focus stays where it is).
                let title = ask.title.clone();
                self.permission = Some(ask);
                self.open = true;
                self.scroll.set(0);
                return Some(Followup::Asked(title));
            }
            AgentEvent::TurnEnded(reason) => {
                if self.status == AgentStatus::Busy {
                    self.status = AgentStatus::Ready;
                }
                self.permission = None;
                let note = match reason.as_str() {
                    "end_turn" => None,
                    "cancelled" => Some("Stopped".to_string()),
                    "max_tokens" => Some("Stopped: it reached its output limit".into()),
                    "max_turn_requests" => Some("Stopped: it reached its limit of steps".into()),
                    "refusal" => Some("The agent declined to go on".into()),
                    other => Some(format!("Stopped ({other})")),
                };
                if let Some(note) = note {
                    self.entries.push(ChatEntry::Notice(note));
                }
                return Some(Followup::TurnEnded);
            }
            AgentEvent::Notice(text) => self.entries.push(ChatEntry::Notice(text)),
        }
        None
    }
}

/// The files a paste names, if it's only that: a file dropped on the
/// terminal pastes its path (shell-quoted: `a\ b.png`, `'a b.png'`), and
/// several, theirs with spaces between.
fn dropped_files(text: &str) -> Option<Vec<std::path::PathBuf>> {
    let text = text.trim();
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    let mut paths = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    let push = |word: &mut String, paths: &mut Vec<String>| {
        if !word.is_empty() {
            paths.push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\\') => word.extend(chars.next()),
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ' ') => push(&mut word, &mut paths),
            (_, c) => word.push(c),
        }
    }
    push(&mut word, &mut paths);
    let here = std::env::current_dir().unwrap_or_default();
    let files: Vec<std::path::PathBuf> = paths
        .iter()
        .map(|p| {
            let p = p.strip_prefix("file://").unwrap_or(p);
            match p.strip_prefix("~/") {
                Some(rest) => std::env::var_os("HOME").map_or_else(
                    || here.join(p),
                    |home| std::path::PathBuf::from(home).join(rest),
                ),
                None => here.join(p),
            }
        })
        .collect();
    // Absolute, and all files: a drop (not a word that happens to be one).
    let dropped = paths
        .iter()
        .all(|p| p.starts_with('/') || p.starts_with("file://") || p.starts_with('~'))
        && files.iter().all(|f| f.is_file());
    dropped.then_some(files)
}

/// Whether a paste is only image files dropped on the terminal.
pub fn only_images(text: &str) -> bool {
    dropped_files(text).is_some_and(|files| files.iter().all(|f| image_type(f).is_some()))
}

/// The image type of a file, by its name.
fn image_type(path: &std::path::Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::agent::transcript::text_of;

    #[test]
    fn a_turn_streams_into_the_chat() {
        let mut chat = AgentChat::default();
        chat.prompt("fix it", None, "claude").unwrap();
        assert_eq!(chat.agent(), "claude");
        assert_eq!(
            chat.take_requests(),
            [
                AgentRequest::Start,
                AgentRequest::Prompt {
                    text: "fix it".into(),
                    context: None,
                    files: Vec::new(),
                    images: Vec::new(),
                }
            ]
        );
        chat.event(AgentEvent::Status(AgentStatus::Busy));
        chat.event(AgentEvent::Text("Looking".into()));
        chat.event(AgentEvent::Text(" now.".into()));
        chat.event(AgentEvent::ToolStarted(ToolCall {
            id: "t1".into(),
            title: "Read main.rs".into(),
            ..ToolCall::default()
        }));
        chat.event(AgentEvent::ToolUpdated {
            id: "t1".into(),
            title: None,
            status: Some(ToolStatus::Done),
            text: None,
            diffs: None,
        });
        assert_eq!(
            chat.event(AgentEvent::TurnEnded("end_turn".into())),
            Some(Followup::TurnEnded)
        );
        let text = text_of(&chat.transcript(80));
        assert!(text.contains(&"› fix it".to_string()), "{text:?}");
        assert!(text.contains(&"Looking now.".to_string()), "{text:?}");
        assert!(text.contains(&"✓ Read main.rs".to_string()), "{text:?}");
        assert_eq!(chat.status(), &AgentStatus::Ready);
    }

    #[test]
    fn permission_is_asked_and_answered() {
        let mut chat = AgentChat::default();
        let followup = chat.event(AgentEvent::Permission(PermissionAsk {
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
        assert_eq!(followup, Some(Followup::Asked("Run cargo test".into())));
        assert!(chat.shown(), "a question shows the sidebar");
        assert!(chat.choose(3).is_err());
        // A digit typed with nothing else typed answers.
        chat.type_char('2').unwrap();
        assert_eq!(chat.input().text(), "");
        assert_eq!(
            chat.take_requests(),
            [AgentRequest::Permission {
                request: serde_json::json!(7),
                option: Some("no".into())
            }]
        );
        assert!(
            chat.type_char('1').is_ok(),
            "asked nothing: a digit's typed"
        );
        assert_eq!(chat.input().text(), "1");
    }

    #[test]
    fn dropped_paths_are_read_as_the_shell_quotes_them() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a b.png");
        let c = dir.path().join("c.txt");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&c, "").unwrap();
        let escaped = a.display().to_string().replace(' ', "\\ ");
        let found = dropped_files(&format!("{escaped} '{}'", c.display())).unwrap();
        assert_eq!(found, [a.clone(), c]);
        assert!(only_images(&escaped));
        // Words, or a path that isn't there: text.
        assert_eq!(dropped_files("hello world"), None);
        assert_eq!(dropped_files("/no/such/file.png"), None);
        assert_eq!(
            dropped_files("src/main.rs"),
            None,
            "not absolute: typed, not dropped"
        );
    }

    #[test]
    fn images_removed_from_the_box_arent_sent() {
        let mut chat = AgentChat::default();
        chat.attach("image/png", vec![1]).unwrap();
        chat.attach("image/png", vec![2]).unwrap();
        assert_eq!(chat.input().text(), "[Image #1] [Image #2] ");
        chat.edit_input(|f| f.delete_to_start());
        for c in "see [Image #2]".chars() {
            chat.type_char(c).unwrap();
        }
        let text = chat.input().text().to_string();
        chat.prompt(&text, None, "claude").unwrap();
        let sent = chat.take_requests().into_iter().find_map(|r| match r {
            AgentRequest::Prompt { images, .. } => Some(images),
            _ => None,
        });
        assert_eq!(sent, Some(vec![("image/png".to_string(), vec![2])]));
        chat.event(AgentEvent::TakesImages(false));
        assert!(chat.attach("image/png", vec![3]).is_err());
    }

    #[test]
    fn modes_cycle() {
        let mut chat = AgentChat::default();
        assert!(chat.next_mode().is_err());
        chat.event(AgentEvent::Modes(
            vec![
                ("default".into(), "Default".into()),
                ("plan".into(), "Plan".into()),
            ],
            Some("default".into()),
        ));
        chat.next_mode().unwrap();
        assert_eq!(chat.mode_name(), Some("Plan"));
        chat.next_mode().unwrap();
        assert_eq!(chat.mode(), Some("default"));
        assert!(chat.set_mode("nope").is_err());
    }
}
