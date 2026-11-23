//! The coding agent, the editor's side. The app's `agent` module talks to
//! the agent (over ACP, the Agent Client Protocol); here's what it talks
//! about:
//!
//! - `chat.rs`: the chat ([`AgentChat`]): what's been said and done, what
//!   the agent's asking, the box, and the requests for the app to send.
//!   It changes only itself; what an event means beyond it comes back as a
//!   [`Followup`].
//! - `session.rs`: its slash commands, settings and context used.
//! - `transcript.rs`: the chat as lines for the sidebar, Markdown laid out.
//! - `tools.rs`, `lsp.rs`: what the agent's tools do in the editor (and
//!   agent changes), and its side of language server answers; their state
//!   is [`AgentToolState`], apart from the chat.
//!
//! This file is the glue: what the chat means for the rest of the editor
//! (focus, pickers, the message line, the options, the clipboard and the
//! buffers its tools opened). Anything that's only the chat's is called on
//! it (`Editor::agent_chat_mut`).
//!
//! The chat's shown in the agent sidebar, on the right of the windows
//! (`ui/agent.rs`): the transcript, the agent's question (its options as
//! buttons), and a box to type in, which has focus in `Mode::Agent`.

mod chat;
mod lsp;
mod session;
mod tools;
mod transcript;

pub(crate) use chat::short_path;
pub use chat::{
    AgentChat, AgentEvent, AgentRequest, AgentStatus, ChatEntry, Followup, Image, PermissionAsk,
    PermissionOption, PlanStep, ToolCall, ToolDiff, ToolStatus, only_images,
};
use session::SlashEnter;
pub use session::{AgentCommand, ConfigOption, SLASH_ROWS, Usage, config_options, tokens};
pub use tools::{AgentAnswer, AgentToolState};
pub use transcript::ChatLine;

use super::picker::{Picker, PickerAction};
use super::{Editor, FileListRequest, Message, Mode};

impl Editor {
    pub fn agent_chat(&self) -> &AgentChat {
        &self.agent
    }

    /// The chat, for what's only its own (scrolling, its modes, the
    /// requests for the app...).
    pub fn agent_chat_mut(&mut self) -> &mut AgentChat {
        &mut self.agent
    }

    /// `:agent`: shows the sidebar with its box focused, starting the
    /// agent if it isn't running.
    pub fn agent_open(&mut self) {
        self.agent.start(&self.options.agent);
        self.agent.show(true);
        self.set_mode(Mode::Agent);
    }

    /// `:agent_sidebar` (Ctrl+Alt+B): shows the sidebar (focused), or
    /// hides it.
    pub fn agent_toggle(&mut self) {
        if self.agent.shown() {
            self.agent_close();
        } else {
            self.agent_open();
        }
    }

    /// Hides the sidebar (the agent keeps running).
    pub fn agent_close(&mut self) {
        self.agent.show(false);
        if self.mode() == Mode::Agent {
            self.set_mode(Mode::Normal);
        }
    }

    /// Focus back to the text (the sidebar stays); with the slash menu
    /// shown, it's put away instead.
    pub fn agent_leave(&mut self) {
        if self.agent.slash_close() {
            return;
        }
        if self.mode() == Mode::Agent {
            self.set_mode(Mode::Normal);
        }
    }

    /// `<CR>` in the box: sends what's typed (or, with the slash menu
    /// shown, its command).
    pub fn agent_send(&mut self) -> Result<(), String> {
        if self.agent_slash_enter(None)? {
            return Ok(());
        }
        let text = self.agent.input().text().to_string();
        self.agent_prompt(&text)?;
        self.agent.clear_input();
        Ok(())
    }

    /// Sends `text` to the agent (starting it first if need be), with a
    /// note of what's open. One of ferra's own commands (`/clear`...) is
    /// run here instead.
    pub fn agent_prompt(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Nothing to ask".into());
        }
        match self.agent.local_command(text) {
            Some("clear") => return self.agent.new_session(),
            Some("model") => return self.agent_config_picker(Some("model")),
            Some("effort") => return self.agent_config_picker(Some("thought_level")),
            _ => {}
        }
        let context = self.agent_context();
        self.agent.prompt(text, context, &self.options.agent)
    }

    /// `<CR>` on the slash menu's row `at` (else the selected one): its
    /// command filled in, or sent. Returns whether the menu was shown.
    fn agent_slash_enter(&mut self, at: Option<usize>) -> Result<bool, String> {
        match self.agent.slash_enter(at)? {
            SlashEnter::Hidden => Ok(false),
            SlashEnter::Filled => Ok(true),
            SlashEnter::Run(text) => self.agent_prompt(&text).map(|()| true),
        }
    }

    /// A click on the slash menu's row `i`: as `<CR>` on it.
    pub fn agent_slash_pick(&mut self, i: usize) -> Result<(), String> {
        self.agent_slash_enter(Some(i))?;
        if self.mode() != Mode::Agent {
            self.set_mode(Mode::Agent);
        }
        Ok(())
    }

    /// `@` in the box: the files to pick one to mention (its path goes in
    /// after the `@`, and the agent's sent the file with the message).
    pub fn agent_mention(&mut self) {
        let job = self.new_job_id();
        self.agent.begin_mention();
        self.open_picker(
            Picker::new("Mention a file", Vec::new())
                .loading_from(job)
                .with_placeholder("Search files by name"),
        );
        self.request_file_list(FileListRequest {
            job,
            root: std::path::PathBuf::from("."),
            base: None,
            options: self.options.file_picker.into(),
        });
    }

    /// The picker the mention was picked in, closed: the file chosen (if
    /// one was) goes in the box, which has focus again.
    pub(in crate::editor) fn agent_mentioned(&mut self, action: Option<&PickerAction>) {
        let path = match action {
            Some(PickerAction::OpenFile(path)) => Some(path.to_string_lossy()),
            _ => None,
        };
        self.agent.end_mention(path.as_deref());
        self.set_mode(Mode::Agent);
    }

    /// Text pasted into the box (see `AgentChat::paste`); an image that
    /// couldn't go in is said.
    pub fn agent_paste(&mut self, text: &str) {
        if let Err(error) = self.agent.paste(text) {
            self.set_message(Message::error(error));
        }
    }

    /// `<C-v>` in the box: an image on the clipboard goes in as one; text,
    /// as text.
    pub fn agent_paste_clipboard(&mut self) -> Result<(), String> {
        if let Some(png) = self.registers.clipboard_image()? {
            return self.agent.attach("image/png", png);
        }
        let text = self
            .registers
            .read(Some('+'))?
            .map(|r| r.text)
            .unwrap_or_default();
        if text.is_empty() {
            return Err("Nothing on the clipboard".into());
        }
        self.agent.paste(&text)
    }

    /// `:agent_mode`: a picker of the agent's modes.
    pub fn agent_mode_picker(&mut self) -> Result<(), String> {
        let picker = self.agent.mode_picker()?;
        self.open_picker(picker);
        Ok(())
    }

    /// `:agent_choose`: a picker of the agents in `[agents]`.
    pub fn agent_picker(&mut self) -> Result<(), String> {
        let picker = self.agent.agent_picker(&self.options.agent)?;
        self.open_picker(picker);
        Ok(())
    }

    /// Talks to agent `name` from now on (it's the editor option `agent`
    /// too): the one running stops; the chat so far stays, for reading.
    pub fn agent_use(&mut self, name: &str) -> Result<(), String> {
        self.agent.use_agent(name)?;
        self.options.agent = name.to_string();
        Ok(())
    }

    /// `:agent_config`: a picker of a setting's values: the setting of
    /// `category`, or the one with that id; with none given, a picker of
    /// the settings first.
    pub fn agent_config_picker(&mut self, which: Option<&str>) -> Result<(), String> {
        let picker = self.agent.config_picker(which)?;
        self.open_picker(picker);
        Ok(())
    }

    /// Sets the agent's setting `id` to `value`, and says so.
    pub fn agent_set_config(&mut self, id: &str, value: &str) -> Result<(), String> {
        let note = self.agent.set_config(id, value)?;
        self.set_message(Message::info(note));
        Ok(())
    }

    /// What the agent said or did: into the chat. A question's said on the
    /// message line if the box hasn't focus; at the end of a turn, what it
    /// opened for its tools is closed.
    pub fn agent_event(&mut self, event: AgentEvent) {
        match self.agent.event(event) {
            Some(Followup::Asked(title)) => {
                if self.mode() != Mode::Agent {
                    self.set_message(Message::info(format!(
                        "The agent asks: {title} (answer in its sidebar)"
                    )));
                }
            }
            Some(Followup::TurnEnded) => self.agent_close_its_buffers(),
            None => {}
        }
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
    fn a_prompt_starts_the_agent_the_options_name() {
        let mut e = Editor::default();
        e.options.agent = "claude".into();
        e.agent_prompt("  fix it ").unwrap();
        assert_eq!(e.agent_chat().agent(), "claude");
        assert!(e.agent_chat().shown());
        assert!(e.agent_prompt(" ").is_err());
        let requests = e.agent_chat_mut().take_requests();
        assert!(matches!(&requests[1], AgentRequest::Prompt { text, .. } if text == "fix it"));
    }

    #[test]
    fn a_question_is_said_unless_the_box_has_focus() {
        let ask = || {
            AgentEvent::Permission(PermissionAsk {
                request: serde_json::json!(1),
                title: "Run it".into(),
                options: Vec::new(),
            })
        };
        let mut e = Editor::default();
        e.agent_event(ask());
        assert!(e.message().is_some_and(|m| m.text.contains("Run it")));
        let mut e = Editor::default();
        e.agent_open();
        e.agent_event(ask());
        assert!(e.message().is_none());
    }

    #[test]
    fn agents_switch_and_are_the_option_from_then() {
        let mut e = Editor::default();
        e.agent_chat_mut()
            .set_names(vec!["gemini".into(), "claude".into()]);
        assert!(e.agent_use("nope").is_err());
        e.agent_use("gemini").unwrap();
        assert_eq!(e.options.agent, "gemini");
        let requests = e.agent_chat_mut().take_requests();
        assert_eq!(requests.last(), Some(&AgentRequest::Start));
    }

    #[test]
    fn a_mention_picked_or_not_goes_back_to_the_box() {
        let mut e = Editor::default();
        e.agent_open();
        e.agent_mention();
        assert!(e.agent_chat().mentioning());
        e.agent_mentioned(Some(&PickerAction::OpenFile("./src/a.rs".into())));
        assert!(!e.agent_chat().mentioning());
        assert_eq!(e.agent_chat().input().text(), "@src/a.rs ");
        assert_eq!(e.mode(), Mode::Agent);
    }
}
