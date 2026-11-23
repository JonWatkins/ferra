//! The coding agent's chat, the editor's side: what's been said and done
//! (the transcript), what the agent is asking, and what the editor wants
//! sent to it. The app's `agent` module talks to the agent (over ACP, the
//! Agent Client Protocol) and turns what it says into `AgentEvent`s here.
//!
//! The chat's shown in the agent sidebar, on the right of the windows
//! (`ui/agent.rs`): the transcript, the agent's question (its options as
//! buttons), and a box to type in, which has focus in `Mode::Agent`.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use super::agent_session::{AgentCommand, ConfigOption, Usage};
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
    pub commands: Vec<AgentCommand>,
    /// The session's settings: model, effort...
    pub config: Vec<ConfigOption>,
    /// How much of its context it's used, once it's said.
    pub usage: Option<Usage>,
    /// The slash menu's selected row.
    pub(super) slash_selected: usize,
    /// What was in the box when the slash menu was put away (`<Esc>`).
    pub(super) slash_closed: Option<String>,
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
    /// Agent changes still open, by tool call: a code action's command
    /// whose server sends its edits as it runs. Each is what was unsaved
    /// before its first edit (see `Editor::agent_change`).
    pub(super) open_changes: HashMap<u64, HashMap<super::DocumentId, (u64, bool)>>,
    /// Tool calls given up waiting on a language server: their answers,
    /// and edits, are dropped if they come after all.
    pub(super) given_up: HashSet<u64>,
    /// The last few lists of code actions it was given, to make one.
    pub(super) code_actions: std::collections::VecDeque<super::lsp::ActionList>,
    /// Whether the agent takes images (it says as it starts).
    pub takes_images: Option<bool>,
    /// Images put in the box, each shown there as `[Image #n]`.
    pub images: Vec<Image>,
    pub(super) requests: Vec<AgentRequest>,
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

    /// Focus back to the text (the sidebar stays); with the slash menu
    /// shown, it's put away instead.
    pub fn agent_leave(&mut self) {
        if self.agent_slash_close() {
            return;
        }
        if self.mode() == Mode::Agent {
            self.set_mode(Mode::Normal);
        }
    }

    /// `<CR>` in the box: sends what's typed.
    pub fn agent_send(&mut self) -> Result<(), String> {
        if self.agent_slash_enter()? {
            return Ok(());
        }
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
        if self.agent_local_command(text)? {
            return Ok(());
        }
        if self.agent.status == AgentStatus::Busy {
            return Err("The agent is still working (Ctrl+C stops it)".into());
        }
        self.agent_start();
        self.agent.open = true;
        self.agent.entries.push(ChatEntry::User(text.to_string()));
        // The images whose `[Image #n]` is still in what's sent.
        let images = std::mem::take(&mut self.agent.images)
            .into_iter()
            .filter(|image| text.contains(&image.label()))
            .map(|image| (image.mime, image.data))
            .collect();
        let request = AgentRequest::Prompt {
            text: text.to_string(),
            context: self.agent_context(),
            files: mentioned_files(text),
            images,
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
        self.agent.slash_selected = 0;
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
        self.agent.slash_selected = 0;
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

    /// Text pasted into the box, newlines and all. Files dropped on the
    /// terminal (which pastes their paths) go in as images, if they are
    /// some, or mentions (`@path`).
    pub fn agent_paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if let Some(paths) = dropped_files(&text) {
            for path in paths {
                if let Some(mime) = image_type(&path) {
                    match std::fs::read(&path) {
                        Ok(data) => {
                            if let Err(error) = self.agent_attach(mime, data) {
                                self.set_message(Message::error(error));
                            }
                        }
                        Err(e) => {
                            self.set_message(Message::error(format!("{}: {e}", path.display())))
                        }
                    }
                } else {
                    let shown = short_path(&path.to_string_lossy());
                    format!("@{shown} ")
                        .chars()
                        .for_each(|c| self.agent.input.push(c));
                }
            }
            return;
        }
        text.chars().for_each(|c| self.agent.input.push(c));
    }

    /// `<C-v>` in the box: an image on the clipboard goes in as one; text,
    /// as text.
    pub fn agent_paste_clipboard(&mut self) -> Result<(), String> {
        if let Some(png) = self.registers.clipboard_image()? {
            return self.agent_attach("image/png", png);
        }
        let text = self
            .registers
            .read(Some('+'))?
            .map(|r| r.text)
            .unwrap_or_default();
        if text.is_empty() {
            return Err("Nothing on the clipboard".into());
        }
        self.agent_paste(&text);
        Ok(())
    }

    /// An image for the next message: `[Image #n]` goes in the box.
    pub fn agent_attach(&mut self, mime: &str, data: Vec<u8>) -> Result<(), String> {
        /// The most an image can be (the agents' own limit is about this).
        const MOST: usize = 5 * 1024 * 1024;
        if self.agent.takes_images == Some(false) {
            return Err(format!("{} doesn't take images", self.agent.title()));
        }
        if data.len() > MOST {
            return Err(format!(
                "That image is {:.1} MB: the most is 5 MB",
                data.len() as f64 / (1024.0 * 1024.0)
            ));
        }
        let number = self
            .agent
            .images
            .iter()
            .map(|i| i.number)
            .max()
            .unwrap_or(0)
            + 1;
        let image = Image {
            number,
            mime: mime.to_string(),
            data,
        };
        format!("{} ", image.label())
            .chars()
            .for_each(|c| self.agent.input.push(c));
        self.agent.images.push(image);
        Ok(())
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
        self.agent.config.clear();
        self.agent.usage = None;
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
            AgentEvent::Config(options) => chat.config = options,
            AgentEvent::Usage(usage) => chat.usage = Some(usage),
            AgentEvent::TakesImages(takes) => chat.takes_images = Some(takes),
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

    /// The transcript as lines, for the sidebar `width` columns wide (which
    /// wraps them; tables are fitted to it here).
    pub fn agent_transcript(&self, width: u16) -> Vec<ChatLine> {
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
                    markdown_lines(text, usize::from(width), &mut lines);
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

/// What the agent said, as lines: Markdown's headings, lists, code blocks,
/// tables (fitted to `width`), `code` and **bold** styled; the rest as it
/// is.
fn markdown_lines(text: &str, width: usize, lines: &mut Vec<ChatLine>) {
    let mut in_code = false;
    let source: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < source.len() {
        let line = source[i];
        i += 1;
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
        // A table: a row of cells, then a row of dashes under them.
        if trimmed.starts_with('|')
            && let Some(aligns) = source.get(i).and_then(|l| table_aligns(l))
        {
            let mut rows = vec![table_cells(trimmed)];
            i += 1;
            while let Some(row) = source
                .get(i)
                .map(|l| l.trim())
                .filter(|l| l.starts_with('|'))
            {
                rows.push(table_cells(row));
                i += 1;
            }
            table_lines(&rows, &aligns, width, lines);
            continue;
        }
        // A rule: `---`, `***` or `___`.
        let bare: String = trimmed.chars().filter(|c| *c != ' ').collect();
        if bare.len() >= 3
            && ['-', '*', '_']
                .iter()
                .any(|&mark| bare.chars().all(|c| c == mark))
        {
            lines.push(ChatLine {
                spans: vec![span(&"─".repeat(width.max(1)), "ui.sidebar.agent.hint")],
                ..ChatLine::default()
            });
            continue;
        }
        // A quote: a bar in front, the text dimmed.
        if let Some(quote) = trimmed.strip_prefix('>') {
            let quote = quote.strip_prefix(' ').unwrap_or(quote);
            let mut spans = vec![span("▎ ", "ui.sidebar.agent.hint")];
            spans.extend(inline_spans(quote).into_iter().map(|mut s| {
                if s.style.as_deref() == Some("ui.sidebar.agent.text") {
                    s.style = Some("ui.sidebar.agent.thought".into());
                }
                s
            }));
            lines.push(ChatLine {
                spans,
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

/// How a table's column is lined up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

/// A table row's cells: split at its `|`s (not `\|`), trimmed.
fn table_cells(row: &str) -> Vec<String> {
    let row = row.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let row = match row.strip_suffix('|') {
        Some(inner) if !inner.ends_with('\\') => inner,
        _ => row,
    };
    let mut cells = vec![String::new()];
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let cell = cells.last_mut().expect("one");
                cell.push(c);
                cell.extend(chars.next());
            }
            '|' => cells.push(String::new()),
            _ => cells.last_mut().expect("one").push(c),
        }
    }
    cells.iter().map(|c| c.trim().to_string()).collect()
}

/// The columns' alignments, if `row` is a table's row of dashes
/// (`|:--|--:|:-:|`).
fn table_aligns(row: &str) -> Option<Vec<Align>> {
    let row = row.trim();
    if !row.starts_with('|') {
        return None;
    }
    table_cells(row)
        .iter()
        .map(|cell| {
            let dashes = cell.trim_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
                return None;
            }
            Some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// Between a table's columns.
const COLUMN_GAP: &str = " │ ";

/// A table's lines: its columns lined up, a rule under the header, each
/// column as wide as its widest cell; if they don't fit `width`, the widest
/// columns are narrowed and their cells wrapped.
fn table_lines(rows: &[Vec<String>], aligns: &[Align], width: usize, lines: &mut Vec<ChatLine>) {
    let columns = rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(aligns.len());
    if columns == 0 {
        return;
    }
    // Each cell as styled chars; the header's bold.
    let cells: Vec<Vec<Vec<(char, String)>>> = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            (0..columns)
                .map(|c| {
                    let text = row.get(c).map_or("", String::as_str);
                    inline_spans(text)
                        .into_iter()
                        .flat_map(|s| {
                            let style = match s.style.as_deref() {
                                Some("ui.sidebar.agent.text") if r == 0 => "ui.sidebar.agent.bold",
                                other => other.unwrap_or("ui.sidebar.agent.text"),
                            }
                            .to_string();
                            s.text
                                .chars()
                                .map(move |ch| (ch, style.clone()))
                                .collect::<Vec<_>>()
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let char_w = |c: char| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    let mut widths: Vec<usize> = (0..columns)
        .map(|c| {
            cells
                .iter()
                .map(|row| row[c].iter().map(|(ch, _)| char_w(*ch)).sum::<usize>())
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    let gaps = (columns - 1) * COLUMN_GAP.chars().count();
    while widths.iter().sum::<usize>() + gaps > width {
        let (widest, &w) = widths
            .iter()
            .enumerate()
            .max_by_key(|(_, w)| **w)
            .expect("a column");
        if w <= 4 {
            break;
        }
        widths[widest] -= 1;
    }
    let rule_style = "ui.sidebar.agent.hint";
    for (r, row) in cells.iter().enumerate() {
        let wrapped: Vec<Vec<Vec<(char, String)>>> = row
            .iter()
            .zip(&widths)
            .map(|(cell, &w)| wrap_cell(cell, w))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for k in 0..height {
            let mut spans = Vec::new();
            for (c, cell) in wrapped.iter().enumerate() {
                if c > 0 {
                    spans.push(span(COLUMN_GAP, rule_style));
                }
                let piece = cell.get(k).map_or(&[][..], Vec::as_slice);
                let used: usize = piece.iter().map(|(ch, _)| char_w(*ch)).sum();
                let room = widths[c].saturating_sub(used);
                let (before, after) = match aligns.get(c).copied().unwrap_or(Align::Left) {
                    Align::Left => (0, room),
                    Align::Right => (room, 0),
                    Align::Center => (room / 2, room - room / 2),
                };
                if before > 0 {
                    spans.push(span(&" ".repeat(before), "ui.sidebar.agent.text"));
                }
                for (ch, style) in piece {
                    match spans.last_mut() {
                        Some(last) if last.style.as_deref() == Some(style.as_str()) => {
                            last.text.push(*ch);
                        }
                        _ => spans.push(span(&ch.to_string(), style)),
                    }
                }
                // The last column needs no padding after it.
                if after > 0 && c + 1 < columns {
                    spans.push(span(&" ".repeat(after), "ui.sidebar.agent.text"));
                }
            }
            lines.push(ChatLine {
                spans,
                ..ChatLine::default()
            });
        }
        if r == 0 {
            let rule: Vec<String> = widths.iter().map(|&w| "─".repeat(w)).collect();
            lines.push(ChatLine {
                spans: vec![span(&rule.join("─┼─"), rule_style)],
                ..ChatLine::default()
            });
        }
    }
}

/// A cell's styled chars as lines `width` wide, broken at spaces (or
/// anywhere, for a word too long).
fn wrap_cell(cell: &[(char, String)], width: usize) -> Vec<Vec<(char, String)>> {
    let char_w = |c: char| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    let mut out = Vec::new();
    let mut line: Vec<(char, String)> = Vec::new();
    let mut used = 0;
    for (ch, style) in cell {
        let w = char_w(*ch);
        if used + w > width && !line.is_empty() {
            let carry = match line.iter().rposition(|(c, _)| *c == ' ') {
                Some(at) if *ch != ' ' => line.split_off(at + 1),
                _ => Vec::new(),
            };
            while line.last().is_some_and(|(c, _)| *c == ' ') {
                line.pop();
            }
            out.push(std::mem::take(&mut line));
            line = carry;
            used = line.iter().map(|(c, _)| char_w(*c)).sum();
            if *ch == ' ' {
                continue;
            }
        }
        line.push((*ch, style.clone()));
        used += w;
    }
    out.push(line);
    out
}

/// Markdown's backslash escapes (`\_`, `\*`, `\|`) as the chars they stand for.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
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
            unescape(piece)
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
                    files: Vec::new(),
                    images: Vec::new(),
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
        let text = text_of(&e.agent_transcript(80));
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
        let closed = text_of(&e.agent_transcript(80));
        assert_eq!(closed, ["✓ Edit a.rs ▸", "  a.rs +1 -1"]);
        e.agent_toggle_tool("t1");
        let open = text_of(&e.agent_transcript(80));
        assert!(open.contains(&"  - two".to_string()), "{open:?}");
        assert!(open.contains(&"  + TWO".to_string()), "{open:?}");
        assert_eq!(open[0], "✓ Edit a.rs ▾");
        e.agent_toggle_tool("t1");
        assert_eq!(text_of(&e.agent_transcript(80)), closed);
    }

    #[test]
    fn markdown_is_styled() {
        let mut lines = Vec::new();
        markdown_lines(
            "# Title\nUse `cargo` **now**\n- one\n```\nlet x;\n```",
            80,
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
    fn tables_are_lined_up_and_escapes_dropped() {
        let mut lines = Vec::new();
        markdown_lines(
            "Usage:\n| MCP server | Usage |\n|:--|--:|\n| ccd\\_turn | 8% |\n| claude-in-chrome | 12% |\nafter",
            80,
            &mut lines,
        );
        assert_eq!(
            text_of(&lines),
            [
                "Usage:",
                "MCP server       │ Usage",
                "─────────────────┼──────",
                "ccd_turn         │    8%",
                "claude-in-chrome │   12%",
                "after",
            ]
        );
        // The header's bold, the rule's dim.
        assert_eq!(
            lines[1].spans[0].style.as_deref(),
            Some("ui.sidebar.agent.bold")
        );
        assert_eq!(
            lines[2].spans[0].style.as_deref(),
            Some("ui.sidebar.agent.hint")
        );

        // Too wide: the wide column's cells wrap inside it.
        let mut lines = Vec::new();
        markdown_lines(
            "| Name | What it does |\n|---|---|\n| a | one two three four five six |",
            20,
            &mut lines,
        );
        let text = text_of(&lines);
        assert!(text.iter().all(|l| l.chars().count() <= 20), "{text:?}");
        assert_eq!(text[0], "Name │ What it does");
        assert_eq!(text[2], "a    │ one two three");
        assert_eq!(text[3], "     │ four five six");

        // Escapes go outside code, stay inside it; | in a cell.
        let mut lines = Vec::new();
        markdown_lines("a\\_b `c\\_d`\n| x \\| y |\n|:-:|", 80, &mut lines);
        assert_eq!(text_of(&lines), ["a_b c\\_d", "x | y", "─────"]);
    }

    #[test]
    fn rules_and_quotes_are_drawn() {
        let mut lines = Vec::new();
        markdown_lines(
            "a\n---\n> Approximate, `only` here\n* * *\n- item",
            6,
            &mut lines,
        );
        assert_eq!(
            text_of(&lines),
            [
                "a",
                "──────",
                "▎ Approximate, only here",
                "──────",
                "• item"
            ]
        );
        assert_eq!(lines[2].indent, 2);
        assert_eq!(
            lines[2].spans[1].style.as_deref(),
            Some("ui.sidebar.agent.thought")
        );
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
        let mut e = Editor::default();
        e.agent_attach("image/png", vec![1]).unwrap();
        e.agent_attach("image/png", vec![2]).unwrap();
        assert_eq!(e.agent_chat().input.text(), "[Image #1] [Image #2] ");
        e.agent_input_edit(|f| f.delete_to_start());
        "see [Image #2]".chars().for_each(|c| e.agent_type(c));
        e.agent_send().unwrap();
        let sent = e.take_agent_requests().into_iter().find_map(|r| match r {
            AgentRequest::Prompt { images, .. } => Some(images),
            _ => None,
        });
        assert_eq!(sent, Some(vec![("image/png".to_string(), vec![2])]));
        e.agent_event(AgentEvent::TakesImages(false));
        assert!(e.agent_attach("image/png", vec![3]).is_err());
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
