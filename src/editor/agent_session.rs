//! The agent's session, beyond the chat: its slash commands (a menu
//! above the box as `/` is typed), its settings (model, effort... as ACP
//! "config options", chosen from chips under the box) and how much of its
//! context it's used.
//!
//! A few commands are ferra's own, run here and never sent: `/clear`
//! (a new session), `/model` and `/effort` (their pickers).

use serde_json::Value;

use super::picker::{Picker, PickerAction, PickerItem};
use super::text_field::TextField;
use super::{Editor, Message};
use crate::editor::agent::{AgentRequest, AgentStatus, ChatEntry};

/// One of the agent's slash commands (`available_commands_update`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub name: String,
    pub description: String,
    /// What it takes after its name (`[instructions]`), if anything.
    pub hint: Option<String>,
}

/// One of the session's settings (ACP's `SessionConfigOption`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    /// `mode`, `model`, `thought_level`... (ACP's categories), if given.
    pub category: Option<String>,
    /// Its value now (`true`/`false` for a switch).
    pub current: String,
    /// What it can be: value and name.
    pub choices: Vec<(String, String)>,
    /// An on/off switch (its value's sent as a JSON boolean).
    pub boolean: bool,
}

impl ConfigOption {
    /// The name of what it's set to.
    pub fn current_name(&self) -> &str {
        self.choices
            .iter()
            .find(|(value, _)| *value == self.current)
            .map_or(&self.current, |(_, name)| name)
    }

    /// An on/off switch, as a switch or as a select of on and off.
    fn switch(&self) -> Option<bool> {
        if self.boolean {
            return Some(self.current == "true");
        }
        let values: Vec<&str> = self.choices.iter().map(|(v, _)| v.as_str()).collect();
        match values[..] {
            ["on", "off"] | ["off", "on"] => Some(self.current == "on"),
            _ => None,
        }
    }

    /// Its chip under the agent's box: `Opus 5.5`, `High effort`,
    /// `Fast mode` (a switch, shown while it's on); `None` to show none.
    pub fn chip(&self) -> Option<String> {
        if self.category.as_deref() == Some("mode") {
            // In the header.
            return None;
        }
        if let Some(on) = self.switch() {
            return on.then(|| self.name.clone());
        }
        // `Default (recommended)` → `Default`.
        let value = self.current_name();
        let value = match value.split_once(" (") {
            Some((short, _)) if value.ends_with(')') => short,
            _ => value,
        };
        Some(match self.category.as_deref() {
            Some("model") if value.eq_ignore_ascii_case("default") => "Default model".into(),
            Some("model") => value.to_string(),
            Some("thought_level") => format!("{value} {}", self.name.to_lowercase()),
            _ => format!("{}: {value}", self.name),
        })
    }

    /// `value` as it's sent: a boolean for a switch.
    pub fn json_value(&self, value: &str) -> Value {
        if self.boolean {
            Value::Bool(value == "true")
        } else {
            Value::String(value.to_string())
        }
    }

    /// From the agent's JSON (`None` if it isn't one ferra knows).
    pub fn parse(option: &Value) -> Option<Self> {
        let id = option["id"].as_str()?.to_string();
        let name = option["name"].as_str().unwrap_or(&id).to_string();
        let category = option["category"].as_str().map(str::to_string);
        match option["type"].as_str()? {
            "boolean" => Some(ConfigOption {
                id,
                name,
                category,
                current: option["currentValue"]
                    .as_bool()
                    .unwrap_or(false)
                    .to_string(),
                choices: vec![("true".into(), "On".into()), ("false".into(), "Off".into())],
                boolean: true,
            }),
            "select" => {
                let mut choices = Vec::new();
                // A list of options, or of groups of them.
                for entry in option["options"].as_array().into_iter().flatten() {
                    match entry["options"].as_array() {
                        Some(group) => choices.extend(group.iter().filter_map(choice)),
                        None => choices.extend(choice(entry)),
                    }
                }
                Some(ConfigOption {
                    id,
                    name,
                    category,
                    current: option["currentValue"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    choices,
                    boolean: false,
                })
            }
            _ => None,
        }
    }
}

fn choice(option: &Value) -> Option<(String, String)> {
    let value = option["value"].as_str()?;
    Some((
        value.to_string(),
        option["name"].as_str().unwrap_or(value).to_string(),
    ))
}

/// All of a session's options, from the agent's JSON.
pub fn config_options(options: &Value) -> Vec<ConfigOption> {
    options
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(ConfigOption::parse)
        .collect()
}

/// How much of its context the session's used (`usage_update`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    /// Tokens in the context now.
    pub used: u64,
    /// The most it holds.
    pub size: u64,
    /// What the session's cost so far: amount and currency.
    pub cost: Option<(f64, String)>,
}

impl Usage {
    /// Its labels, longest first, for the room there is: `42k / 200k ·
    /// $0.31`, `42k / 200k`, `21%`.
    pub fn labels(&self) -> [String; 3] {
        let percent = (self.used * 100).checked_div(self.size).unwrap_or(0);
        [
            self.label(),
            format!("{} / {}", tokens(self.used), tokens(self.size)),
            format!("{percent}%"),
        ]
    }

    /// `42k / 200k · $0.31`.
    pub fn label(&self) -> String {
        let mut label = format!("{} / {}", tokens(self.used), tokens(self.size));
        if let Some((amount, currency)) = &self.cost {
            if currency == "USD" {
                label.push_str(&format!(" · ${amount:.2}"));
            } else {
                label.push_str(&format!(" · {amount:.2} {currency}"));
            }
        }
        label
    }
}

/// `950`, `42k`, `1.2M`.
pub fn tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{}k", (n + 500) / 1000)
    } else {
        let m = format!("{:.1}", n as f64 / 1_000_000.0);
        format!("{}M", m.trim_end_matches(".0"))
    }
}

/// ferra's own commands: name, description, and whether it's offered
/// (by what the session has).
const LOCAL: [(&str, &str); 3] = [
    ("clear", "Start a new conversation (ferra)"),
    ("model", "Choose the model (ferra)"),
    ("effort", "Choose how hard it thinks (ferra)"),
];

/// The most rows the slash menu shows.
pub const SLASH_ROWS: usize = 8;

impl Editor {
    /// The option of `category` (`model`, `thought_level`), if the agent
    /// has one.
    fn agent_option_of(&self, category: &str) -> Option<&ConfigOption> {
        self.agent
            .config
            .iter()
            .find(|o| o.category.as_deref() == Some(category))
    }

    /// The commands `/` offers: the agent's, then ferra's own (those it
    /// can do now, and not shadowing one of the agent's).
    pub fn agent_commands(&self) -> Vec<AgentCommand> {
        let mut commands: Vec<AgentCommand> = self
            .agent
            .commands
            .iter()
            .filter(|c| !LOCAL.iter().any(|(name, _)| *name == c.name))
            .cloned()
            .collect();
        for (name, description) in LOCAL {
            let offered = match name {
                "model" => self.agent_option_of("model").is_some(),
                "effort" => self.agent_option_of("thought_level").is_some(),
                _ => true,
            };
            if offered {
                commands.push(AgentCommand {
                    name: name.into(),
                    description: description.into(),
                    hint: None,
                });
            }
        }
        commands
    }

    /// The slash menu's commands: while the box holds just `/` and a
    /// name being typed (and it wasn't put away). Names starting with
    /// what's typed come first, then those containing it.
    pub fn agent_slash_matches(&self) -> Vec<AgentCommand> {
        let text = self.agent.input.text();
        let Some(typed) = text.strip_prefix('/') else {
            return Vec::new();
        };
        if typed.contains(char::is_whitespace) || self.agent.slash_closed.as_deref() == Some(text) {
            return Vec::new();
        }
        let typed = typed.to_lowercase();
        let (mut first, mut then): (Vec<_>, Vec<_>) = self
            .agent_commands()
            .into_iter()
            .filter(|c| c.name.to_lowercase().contains(&typed))
            .partition(|c| c.name.to_lowercase().starts_with(&typed));
        first.sort_by(|a, b| a.name.cmp(&b.name));
        then.sort_by(|a, b| a.name.cmp(&b.name));
        first.extend(then);
        first
    }

    /// The slash menu's selected row (kept in range).
    pub fn agent_slash_selected(&self) -> usize {
        let count = self.agent_slash_matches().len();
        self.agent.slash_selected.min(count.saturating_sub(1))
    }

    /// `<Up>`/`<Down>` in the box: through the slash menu, if it's shown;
    /// else the chat scrolls.
    pub fn agent_up_down(&mut self, down: bool) {
        let count = self.agent_slash_matches().len();
        if count == 0 {
            self.agent_scroll(if down { -3 } else { 3 });
            return;
        }
        let at = self.agent_slash_selected();
        self.agent.slash_selected = if down {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
    }

    /// `<Tab>` in the box: the selected command's name filled in.
    pub fn agent_complete(&mut self) -> Result<(), String> {
        let matches = self.agent_slash_matches();
        let command = matches
            .get(self.agent_slash_selected())
            .ok_or("Type / for the agent's commands")?;
        self.agent.input = TextField::default();
        format!("/{} ", command.name)
            .chars()
            .for_each(|c| self.agent.input.push(c));
        self.agent.slash_selected = 0;
        Ok(())
    }

    /// `<CR>` with the slash menu shown: the selected command. One that
    /// takes something after its name is filled in to finish; one that
    /// doesn't is run. Returns whether the menu was shown.
    pub(super) fn agent_slash_enter(&mut self) -> Result<bool, String> {
        let matches = self.agent_slash_matches();
        let Some(command) = matches.get(self.agent_slash_selected()) else {
            return Ok(false);
        };
        if command.hint.is_some() {
            self.agent_complete()?;
            return Ok(true);
        }
        let text = format!("/{}", command.name);
        self.agent.input = TextField::default();
        self.agent.slash_selected = 0;
        self.agent_prompt(&text)?;
        Ok(true)
    }

    /// A click on the slash menu's row `i`: as `<CR>` on it.
    pub fn agent_slash_pick(&mut self, i: usize) -> Result<(), String> {
        self.agent.slash_selected = i;
        self.agent_slash_enter()?;
        if self.mode() != super::Mode::Agent {
            self.set_mode(super::Mode::Agent);
        }
        Ok(())
    }

    /// Puts the slash menu away (`<Esc>`) until the box changes. Returns
    /// whether it was shown.
    pub fn agent_slash_close(&mut self) -> bool {
        if self.agent_slash_matches().is_empty() {
            return false;
        }
        self.agent.slash_closed = Some(self.agent.input.text().to_string());
        true
    }

    /// The hint for what's typed: the command's input hint, while just
    /// `/name ` is in the box.
    pub fn agent_input_hint(&self) -> Option<String> {
        let text = self.agent.input.text();
        let name = text.strip_prefix('/')?.strip_suffix(' ')?;
        self.agent_commands()
            .into_iter()
            .find(|c| c.name == name)?
            .hint
    }

    /// One of ferra's own commands, typed as the whole message: run here.
    /// Returns whether it was one.
    pub(super) fn agent_local_command(&mut self, text: &str) -> Result<bool, String> {
        let Some(name) = text.strip_prefix('/') else {
            return Ok(false);
        };
        let local = self
            .agent_commands()
            .into_iter()
            .any(|c| c.name == name && LOCAL.iter().any(|(n, _)| *n == name));
        if !local {
            return Ok(false);
        }
        match name {
            "clear" => self.agent_new_session()?,
            "model" => self.agent_config_picker(Some("model"))?,
            "effort" => self.agent_config_picker(Some("thought_level"))?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `/clear`, `:agent_new`: a new session (the agent forgets the
    /// conversation; the chat's cleared).
    pub fn agent_new_session(&mut self) -> Result<(), String> {
        match self.agent.status {
            AgentStatus::Busy => return Err("The agent is still working (Ctrl+C stops it)".into()),
            AgentStatus::Ready => {
                self.agent.requests.push(AgentRequest::NewSession);
                self.agent.status = AgentStatus::Starting;
            }
            // It starts with a new session anyway.
            _ => {}
        }
        let chat = &mut self.agent;
        chat.entries.clear();
        chat.expanded.clear();
        chat.permission = None;
        chat.usage = None;
        chat.scroll.set(0);
        chat.entries
            .push(ChatEntry::Notice("A new conversation".into()));
        Ok(())
    }

    /// `:agent_config`: a picker of a setting's values: the setting of
    /// `category`, or the one with that id; with none given, a picker of
    /// the settings first.
    pub fn agent_config_picker(&mut self, which: Option<&str>) -> Result<(), String> {
        if self.agent.config.is_empty() {
            return Err("The agent has no settings (or hasn't started)".into());
        }
        let Some(which) = which else {
            let items = self
                .agent
                .config
                .iter()
                .filter(|o| o.category.as_deref() != Some("mode"))
                .map(|o| PickerItem {
                    label: o.name.clone(),
                    detail: o.current_name().to_string(),
                    action: PickerAction::RunWith("agent_config".into(), vec![o.id.clone()]),
                })
                .collect();
            self.open_picker(Picker::new("Agent settings", items).keep_label_start());
            return Ok(());
        };
        let option = self
            .agent
            .config
            .iter()
            .find(|o| o.id == which)
            .or_else(|| self.agent_option_of(which))
            .ok_or_else(|| format!("The agent has no setting \"{which}\""))?;
        let items = option
            .choices
            .iter()
            .map(|(value, name)| PickerItem {
                label: name.clone(),
                detail: if *value == option.current {
                    "current".into()
                } else {
                    String::new()
                },
                action: PickerAction::RunWith(
                    "agent_set_config".into(),
                    vec![option.id.clone(), value.clone()],
                ),
            })
            .collect();
        let title = option.name.clone();
        self.open_picker(Picker::new(title, items).keep_label_start());
        Ok(())
    }

    /// Sets the agent's setting `id` to `value` (shown at once; the agent
    /// answers with them all).
    pub fn agent_set_config(&mut self, id: &str, value: &str) -> Result<(), String> {
        let option = self
            .agent
            .config
            .iter_mut()
            .find(|o| o.id == id)
            .ok_or_else(|| format!("The agent has no setting \"{id}\""))?;
        if !option.choices.iter().any(|(v, _)| v == value) {
            return Err(format!("{} can't be \"{value}\"", option.name));
        }
        option.current = value.to_string();
        let request = AgentRequest::SetConfig {
            id: id.to_string(),
            value: option.json_value(value),
        };
        let note = format!("{}: {}", option.name, option.current_name());
        self.agent.requests.push(request);
        self.set_message(Message::info(note));
        Ok(())
    }

    /// The settings shown as chips under the box: their ids and labels
    /// (see `ConfigOption::chip`).
    pub fn agent_chips(&self) -> Vec<(String, String)> {
        self.agent
            .config
            .iter()
            .filter_map(|o| Some((o.id.clone(), o.chip()?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tokens_read_short() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(42_300), "42k");
        assert_eq!(tokens(200_000), "200k");
        assert_eq!(tokens(1_000_000), "1M");
        assert_eq!(tokens(1_250_000), "1.2M");
        let usage = Usage {
            used: 42_000,
            size: 200_000,
            cost: Some((0.314, "USD".into())),
        };
        assert_eq!(usage.label(), "42k / 200k · $0.31");
    }

    #[test]
    fn options_are_read_grouped_or_not() {
        let options = config_options(&json!([
            {"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": "opus", "options": [
                {"value": "opus", "name": "Opus"}, {"value": "sonnet", "name": "Sonnet"}]},
            {"id": "effort", "name": "Effort", "type": "select", "currentValue": "high",
             "options": [{"group": "g", "name": "Levels", "options": [
                {"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]}]},
            {"id": "fast", "name": "Fast", "type": "boolean", "currentValue": true},
            {"id": "odd", "name": "Odd", "type": "slider"},
        ]));
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].current_name(), "Opus");
        assert_eq!(options[1].choices.len(), 2);
        assert_eq!(options[1].current_name(), "High");
        assert_eq!(options[2].current_name(), "On");
        assert_eq!(options[2].json_value("false"), json!(false));
        assert_eq!(options[0].json_value("sonnet"), json!("sonnet"));
    }

    #[test]
    fn chips_say_what_they_set() {
        let options = config_options(&json!([
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select",
             "currentValue": "plan", "options": [{"value": "plan", "name": "Plan"}]},
            {"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": "default", "options": [
                {"value": "default", "name": "Default (recommended)"},
                {"value": "opus", "name": "Opus 5.5"}]},
            {"id": "effort", "name": "Effort", "category": "thought_level", "type": "select",
             "currentValue": "high", "options": [{"value": "high", "name": "High"}]},
            {"id": "fast", "name": "Fast mode", "category": "model_config", "type": "select",
             "currentValue": "off", "options": [
                {"value": "on", "name": "On"}, {"value": "off", "name": "Off"}]},
            {"id": "x", "name": "Style", "type": "select", "currentValue": "terse",
             "options": [{"value": "terse", "name": "Terse"}]},
        ]));
        let chips: Vec<Option<String>> = options.iter().map(ConfigOption::chip).collect();
        assert_eq!(
            chips,
            [
                None,
                Some("Default model".into()),
                Some("High effort".into()),
                None,
                Some("Style: Terse".into())
            ]
        );
        let mut fast = options[3].clone();
        fast.current = "on".into();
        assert_eq!(fast.chip().as_deref(), Some("Fast mode"));
        let mut opus = options[1].clone();
        opus.current = "opus".into();
        assert_eq!(opus.chip().as_deref(), Some("Opus 5.5"));
        let usage = Usage {
            used: 50_000,
            size: 200_000,
            cost: None,
        };
        assert_eq!(usage.labels()[2], "25%");
    }
}
