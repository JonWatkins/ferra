//! The settings editor (`:settings`, the activity bar's gear, File →
//! Settings): every `[editor]` option by category, searchable, changed
//! with a key or a click. A change applies at once, as `:set` does, and is
//! written to `config.toml` straight away (a setting put back to its
//! default is taken out of it), keeping the file's comments.
//!
//! It takes the windows' place while it's open, in its own mode
//! (`Mode::Settings`, keys in `[keys.settings]`). Typing in the search box
//! or a value's field goes to that field (`settings_field_key`).

use std::cell::Cell;
use std::path::PathBuf;

use toml::Value;

use super::text_field::TextField;
use super::{Editor, Message, Mode};
use crate::config::schema::{self, CATEGORIES, Kind, Setting};
use crate::input::{KeyCode, KeyPress};

/// Which part of the settings editor keys go to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsFocus {
    /// The list of categories, down the left.
    Categories,
    /// The settings.
    #[default]
    List,
    /// Typing in the search box.
    Search,
    /// Typing a number's or a text's new value.
    Edit,
}

#[derive(Debug, Default)]
pub struct SettingsView {
    pub category: usize,
    /// The selected setting, in `shown()`.
    pub selected: usize,
    pub focus: SettingsFocus,
    pub query: TextField,
    /// The value being typed, with `focus` on `Edit`.
    pub edit: TextField,
    /// The first setting drawn (kept by the UI so the selection shows).
    pub scroll: Cell<usize>,
}

impl SettingsView {
    /// The settings listed: the category's, or with a search, every one
    /// that matches it (by title, name, description or category), those
    /// whose title matches first.
    pub fn shown(&self) -> Vec<&'static Setting> {
        let query = self.query.text().trim().to_lowercase();
        if query.is_empty() {
            return schema::settings()
                .iter()
                .filter(|s| s.category == CATEGORIES[self.category])
                .collect();
        }
        let words: Vec<&str> = query.split_whitespace().collect();
        let all_in = |text: &str| {
            let text = text.to_lowercase();
            words.iter().all(|word| text.contains(word))
        };
        let mut found: Vec<(u8, &'static Setting)> = schema::settings()
            .iter()
            .filter_map(|s| {
                let rank = if all_in(&s.title) {
                    0
                } else if all_in(&s.path) {
                    1
                } else if words.iter().all(|word| {
                    [
                        s.title.as_str(),
                        s.path.as_str(),
                        s.category,
                        &s.description,
                    ]
                    .iter()
                    .any(|field| field.to_lowercase().contains(word))
                }) {
                    2
                } else {
                    return None;
                };
                Some((rank, s))
            })
            .collect();
        found.sort_by_key(|(rank, _)| *rank);
        found.into_iter().map(|(_, s)| s).collect()
    }

    pub fn searching(&self) -> bool {
        !self.query.text().trim().is_empty()
    }

    /// The selected setting, if any are listed.
    pub fn current(&self) -> Option<&'static Setting> {
        self.shown().get(self.selected).copied()
    }
}

impl Editor {
    pub fn settings(&self) -> Option<&SettingsView> {
        self.settings.as_ref()
    }

    /// Where the settings editor saves changes: the user's `config.toml`
    /// (set at startup; tests set their own, or none).
    pub fn set_config_file(&mut self, path: Option<PathBuf>) {
        self.config_file = path;
    }

    pub fn config_file(&self) -> Option<&std::path::Path> {
        self.config_file.as_deref()
    }

    /// `:settings`: opens the settings editor, at `query`'s matches if
    /// given (`:settings tab`).
    pub fn settings_open(&mut self, query: Option<&str>) {
        let mut view = self.settings.take().unwrap_or_default();
        if let Some(query) = query.filter(|q| !q.is_empty()) {
            view.query.set_text(query);
            view.selected = 0;
        }
        view.focus = SettingsFocus::List;
        self.keybindings = None;
        self.settings = Some(view);
        self.set_mode(Mode::Settings);
    }

    /// `q`, `<Esc>`: back to the windows.
    pub fn settings_close(&mut self) {
        if self.settings.take().is_some() && self.mode() == Mode::Settings {
            self.set_mode(Mode::Normal);
        }
    }

    /// `e`: closes it and opens `config.toml` to edit by hand.
    pub fn settings_edit_toml(&mut self) -> Result<(), String> {
        self.settings_close();
        self.open_config()
    }

    /// `j`/`k`: the next or previous category or setting.
    pub fn settings_move(&mut self, delta: isize) {
        let Some(view) = self.settings.as_mut() else {
            return;
        };
        match view.focus {
            SettingsFocus::Categories => {
                let n = CATEGORIES.len() as isize;
                view.category = (view.category as isize + delta).clamp(0, n - 1) as usize;
                view.selected = 0;
                view.scroll.set(0);
            }
            _ => {
                let n = view.shown().len() as isize;
                if n > 0 {
                    view.selected = (view.selected as isize + delta).clamp(0, n - 1) as usize;
                }
                view.focus = SettingsFocus::List;
            }
        }
    }

    /// `gg` / `G`: the first or last setting.
    pub fn settings_jump(&mut self, last: bool) {
        let Some(view) = self.settings.as_mut() else {
            return;
        };
        view.selected = if last {
            view.shown().len().saturating_sub(1)
        } else {
            0
        };
    }

    /// `<Tab>`: between the categories and the settings.
    pub fn settings_switch_pane(&mut self) {
        let Some(view) = self.settings.as_mut() else {
            return;
        };
        view.focus = match view.focus {
            SettingsFocus::Categories => SettingsFocus::List,
            _ if view.searching() => SettingsFocus::List,
            _ => SettingsFocus::Categories,
        };
    }

    /// Shows category `index` (a click on it).
    pub fn settings_category(&mut self, index: usize) {
        let Some(view) = self.settings.as_mut() else {
            return;
        };
        if index < CATEGORIES.len() {
            view.category = index;
            view.query.set_text("");
            view.selected = 0;
            view.scroll.set(0);
            view.focus = SettingsFocus::Categories;
        }
    }

    /// Selects listed setting `index` (a click on it).
    pub fn settings_select(&mut self, index: usize) {
        let Some(view) = self.settings.as_mut() else {
            return;
        };
        if index < view.shown().len() {
            view.selected = index;
            view.focus = SettingsFocus::List;
        }
    }

    /// `/`: types in the search box.
    pub fn settings_search(&mut self) {
        if let Some(view) = self.settings.as_mut() {
            view.focus = SettingsFocus::Search;
            // Typing replaces what was searched for, as in VS Code.
            view.query.select_all();
        }
    }

    /// `<CR>`, `<Space>`, a click on the value: flips a switch, moves a
    /// choice on, or starts typing a number's or text's new value. In the
    /// categories, goes to their settings.
    pub fn settings_activate(&mut self) -> Result<(), String> {
        let Some(view) = self.settings.as_mut() else {
            return Ok(());
        };
        if view.focus == SettingsFocus::Categories {
            view.focus = SettingsFocus::List;
            return Ok(());
        }
        let Some(setting) = view.current() else {
            return Ok(());
        };
        match &setting.kind {
            Kind::Bool | Kind::Choice(_) => self.settings_step(1),
            Kind::Number | Kind::Text => {
                let current = self.setting_value(setting);
                let text = match current {
                    Some(Value::String(s)) => s,
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                let view = self.settings.as_mut().expect("open");
                view.edit.set_selected(&text);
                view.focus = SettingsFocus::Edit;
                Ok(())
            }
            Kind::List => Err(format!(
                "{} is a list: edit it in config.toml (e)",
                setting.title
            )),
        }
    }

    /// `l`/`h` (`→`/`←`): the next or previous choice, one more or less,
    /// or a switch flipped.
    pub fn settings_step(&mut self, delta: i64) -> Result<(), String> {
        let Some(view) = self.settings.as_ref() else {
            return Ok(());
        };
        if view.focus == SettingsFocus::Categories {
            return Ok(());
        }
        let Some(setting) = view.current() else {
            return Ok(());
        };
        let current = self.setting_value(setting);
        let value = match (&setting.kind, current) {
            (Kind::Bool, Some(Value::Boolean(on))) => Value::Boolean(!on),
            (Kind::Choice(names), Some(Value::String(name))) => {
                let at = names.iter().position(|n| *n == name).unwrap_or(0) as i64;
                let n = names.len() as i64;
                Value::String(names[(at + delta).rem_euclid(n) as usize].clone())
            }
            (Kind::Number, Some(Value::Integer(n))) => Value::Integer((n + delta).max(0)),
            // Text and lists have no next value.
            _ => return Ok(()),
        };
        self.settings_apply(setting, value)
    }

    /// `u`: puts the selected setting back to its default.
    pub fn settings_reset(&mut self) -> Result<(), String> {
        let Some(setting) = self.settings.as_ref().and_then(SettingsView::current) else {
            return Ok(());
        };
        self.settings_apply(setting, setting.default.clone())
    }

    /// The value `setting` has (as set in config and by `:set`).
    pub fn setting_value(&self, setting: &Setting) -> Option<Value> {
        crate::config::set::get_value(&self.global_options, &setting.path)
    }

    /// Whether `setting` isn't at its default.
    pub fn setting_modified(&self, setting: &Setting) -> bool {
        self.setting_value(setting)
            .is_some_and(|value| value != setting.default)
    }

    /// Sets `setting` to `value` (as `:set` would) and saves it.
    fn settings_apply(&mut self, setting: &Setting, value: Value) -> Result<(), String> {
        self.set_option(&setting.path, value.clone(), false)?;
        let shown = match &value {
            Value::String(s) => format!("{s:?}"),
            other => other.to_string(),
        };
        let Some(file) = self.config_file.clone() else {
            self.set_message(Message::info(format!(
                "{} = {shown} (not saved: there's no config file)",
                setting.path
            )));
            return Ok(());
        };
        let saved = (value != setting.default).then_some(&value);
        crate::config::edit::write_option(&file, &setting.path, saved)?;
        let text = if saved.is_some() {
            format!("{} = {shown}", setting.path)
        } else {
            format!("{} = {shown} (the default)", setting.path)
        };
        self.set_message(Message::info(text));
        Ok(())
    }

    /// While the search box or a value's field has focus, `key` goes to it.
    /// Returns whether it took the key.
    pub fn settings_field_key(&mut self, key: KeyPress) -> bool {
        let Some(view) = self.settings.as_mut() else {
            return false;
        };
        let field = match view.focus {
            SettingsFocus::Search => &mut view.query,
            SettingsFocus::Edit => &mut view.edit,
            _ => return false,
        };
        let ctrl = key.modifiers.ctrl;
        match key.code {
            KeyCode::Enter => {
                if view.focus == SettingsFocus::Edit {
                    let result = self.settings_commit_edit();
                    if let Err(error) = result {
                        self.set_message(Message::error(error));
                    }
                } else {
                    view.focus = SettingsFocus::List;
                }
            }
            KeyCode::Esc => {
                if view.focus == SettingsFocus::Search {
                    view.query.set_text("");
                    view.selected = 0;
                }
                view.focus = SettingsFocus::List;
            }
            KeyCode::Down | KeyCode::Tab if view.focus == SettingsFocus::Search => {
                view.focus = SettingsFocus::List;
            }
            KeyCode::Backspace => field.backspace(),
            KeyCode::Delete => field.delete(),
            KeyCode::Left => field.left(),
            KeyCode::Right => field.right(),
            KeyCode::Home => field.home(),
            KeyCode::End => field.end(),
            KeyCode::Char('u') if ctrl => field.delete_to_start(),
            KeyCode::Char('w') if ctrl => field.delete_word(),
            KeyCode::Char('a') if ctrl => field.home(),
            KeyCode::Char('e') if ctrl => field.end(),
            _ => match key.as_text() {
                Some(c) => field.push(c),
                // Other keys do nothing while typing.
                None => return true,
            },
        }
        if let Some(view) = self.settings.as_mut()
            && view.focus == SettingsFocus::Search
        {
            view.selected = 0;
            view.scroll.set(0);
        }
        true
    }

    /// Pasted text, into the field being typed in.
    pub fn settings_paste(&mut self, text: &str) {
        let Some(view) = self.settings.as_mut() else {
            return;
        };
        let field = match view.focus {
            SettingsFocus::Search => &mut view.query,
            SettingsFocus::Edit => &mut view.edit,
            _ => return,
        };
        for c in text.chars().filter(|c| !c.is_control()) {
            field.push(c);
        }
    }

    /// `<CR>` in a value's field: sets it.
    fn settings_commit_edit(&mut self) -> Result<(), String> {
        let Some(view) = self.settings.as_mut() else {
            return Ok(());
        };
        let Some(setting) = view.current() else {
            return Ok(());
        };
        let text = view.edit.text().to_string();
        let value = match setting.kind {
            Kind::Number => Value::Integer(
                text.trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| format!("{}: {text:?} isn't a number", setting.title))?,
            ),
            _ => Value::String(text),
        };
        self.settings_apply(setting, value)?;
        if let Some(view) = self.settings.as_mut() {
            view.focus = SettingsFocus::List;
        }
        Ok(())
    }
}
