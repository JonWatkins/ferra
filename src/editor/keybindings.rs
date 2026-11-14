//! The keybinding editor (`:keybindings`, File → Keyboard Shortcuts): every
//! command with its keys, the mode they're for and where they come from,
//! searchable by name or by pressing the keys. A binding is changed by
//! pressing the new keys, as in VS Code; what else uses them shows as they're
//! pressed.
//!
//! The app owns the keymap, so it makes the rows (`keybindings_show`) and
//! makes the changes asked for here (`take_key_edits`): writing them to
//! `config.toml` (`[keys.<mode>]`, keeping its comments) and building the
//! keys again.

use std::cell::Cell;

use super::text_field::TextField;
use super::{Editor, Message, Mode};
use crate::input::{KeyCode, KeyPress};

/// Where a binding comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSource {
    /// The keymap's own (Vim's or VS Code's, and the keys they share).
    Keymap,
    /// The user's `[keys]`.
    User,
    /// A plugin's.
    Plugin,
    /// One of the keymap's that the user took away (`"<C-s>" = ""`).
    Removed,
    /// A command with no keys.
    Unbound,
}

impl BindingSource {
    pub fn label(self) -> &'static str {
        match self {
            BindingSource::Keymap => "keymap",
            BindingSource::User => "user",
            BindingSource::Plugin => "plugin",
            BindingSource::Removed => "removed",
            BindingSource::Unbound => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    pub command: String,
    pub description: String,
    /// The keys (none for an unbound command).
    pub keys: Vec<KeyPress>,
    /// The mode they're for (none for an unbound command).
    pub mode: Option<Mode>,
    pub source: BindingSource,
}

/// Why keys are being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    /// `r`: to find the bindings that use them.
    Search,
    /// `<CR>`: new keys for the selected binding, in place of its own.
    Change,
    /// `a`: more keys for the selected command, besides its others.
    Add,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeysFocus {
    #[default]
    List,
    /// Typing in the search box.
    Search,
    /// Pressing keys (`recording` says why).
    Record,
}

/// A change to the user's `[keys]`, for the app to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEdit {
    pub mode: Mode,
    pub keys: Vec<KeyPress>,
    /// The command to bind them to, `""` to take away the keymap's
    /// binding, or `None` to take the user's line out (the keymap's comes
    /// back).
    pub command: Option<String>,
}

#[derive(Debug, Default)]
pub struct KeybindingsView {
    rows: Vec<BindingRow>,
    pub query: TextField,
    /// Keys pressed to search with (`r`): rows whose keys start with them.
    pub key_filter: Vec<KeyPress>,
    pub focus: KeysFocus,
    pub recording: Option<Recording>,
    /// The keys pressed so far while recording.
    pub recorded: Vec<KeyPress>,
    /// The selected row, in `shown()`.
    pub selected: usize,
    /// The first row drawn (kept by the UI).
    pub scroll: Cell<usize>,
    /// The mode new keys for an unbound command go in.
    pub home: Mode,
    /// The binding to select when the rows next change (one just made).
    reselect: Option<(String, Option<Mode>, Vec<KeyPress>)>,
}

/// A row's keys as notation (`<C-s>`, `gg`).
pub fn notation(keys: &[KeyPress]) -> String {
    keys.iter().map(ToString::to_string).collect()
}

impl KeybindingsView {
    /// The rows listed: those matching the search (by command, its
    /// description, its keys or its mode; those whose command matches
    /// first) and the keys pressed for it.
    pub fn shown(&self) -> Vec<&BindingRow> {
        let query = self.query.text().trim().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut rows: Vec<&BindingRow> = self
            .rows
            .iter()
            .filter(|row| {
                words.iter().all(|word| {
                    row.command.to_lowercase().contains(word)
                        || row.description.to_lowercase().contains(word)
                        || notation(&row.keys).to_lowercase().contains(word)
                        || row.mode.is_some_and(|m| m.id() == *word)
                })
            })
            .filter(|row| {
                self.key_filter.is_empty()
                    || (row.keys.len() >= self.key_filter.len()
                        && row.keys[..self.key_filter.len()] == self.key_filter[..])
            })
            .collect();
        let command_matches = |row: &BindingRow| {
            let name = row.command.replace('_', " ");
            words.iter().all(|word| name.contains(word))
        };
        rows.sort_by_key(|row| !command_matches(row));
        rows
    }

    /// How many rows there are, shown or not.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    pub fn current(&self) -> Option<&BindingRow> {
        self.shown().get(self.selected).copied()
    }

    /// While recording new keys: the other bindings in that mode they'd
    /// clash with (the same keys, or ones that start with them or that
    /// they start with).
    pub fn conflicts(&self) -> Vec<&BindingRow> {
        if self.recording == Some(Recording::Search) || self.recorded.is_empty() {
            return Vec::new();
        }
        let Some(row) = self.current() else {
            return Vec::new();
        };
        self.conflicts_with(row.mode.unwrap_or(self.home), &self.recorded, row)
    }

    pub fn searching(&self) -> bool {
        !self.query.text().trim().is_empty() || !self.key_filter.is_empty()
    }
}

impl Editor {
    pub fn keybindings(&self) -> Option<&KeybindingsView> {
        self.keybindings.as_ref()
    }

    /// `:keybindings`: asks the app to open the keybinding editor (it has
    /// the keys), searching for `query` if given.
    pub fn keybindings_request(&mut self, query: &str) {
        self.keybindings_wanted = Some(query.to_string());
    }

    /// The open request, for the app.
    pub fn take_keybindings_request(&mut self) -> Option<String> {
        self.keybindings_wanted.take()
    }

    /// Opens the keybinding editor with `rows` (or, open, puts them in
    /// place, the same binding staying selected).
    pub fn keybindings_show(&mut self, rows: Vec<BindingRow>, query: Option<&str>) {
        let home = if self.modeless() {
            Mode::Insert
        } else {
            Mode::Normal
        };
        let mut view = self.keybindings.take().unwrap_or_default();
        let was = view.reselect.take().or_else(|| {
            view.current()
                .map(|row| (row.command.clone(), row.mode, row.keys.clone()))
        });
        view.rows = rows;
        view.home = home;
        if let Some(query) = query {
            view.query.set_text(query);
            view.key_filter.clear();
            view.selected = 0;
        } else if let Some((command, mode, keys)) = was {
            let shown = view.shown();
            let at = shown
                .iter()
                .position(|r| r.command == command && r.mode == mode && r.keys == keys)
                .or_else(|| shown.iter().position(|r| r.command == command));
            view.selected = at.unwrap_or(view.selected.min(shown.len().saturating_sub(1)));
        }
        self.keybindings = Some(view);
        if self.mode() != Mode::Keybindings {
            self.settings = None;
            self.set_mode(Mode::Keybindings);
        }
    }

    /// `q`, `<Esc>`: back to the windows.
    pub fn keybindings_close(&mut self) {
        if self.keybindings.take().is_some() && self.mode() == Mode::Keybindings {
            self.set_mode(Mode::Normal);
        }
    }

    /// The changes asked for, for the app to make.
    pub fn take_key_edits(&mut self) -> Vec<KeyEdit> {
        std::mem::take(&mut self.key_edits)
    }

    /// `j`/`k` and the like: `delta` rows on.
    pub fn keybindings_move(&mut self, delta: isize) {
        let Some(view) = self.keybindings.as_mut() else {
            return;
        };
        let n = view.shown().len() as isize;
        if n > 0 {
            view.selected = (view.selected as isize + delta).clamp(0, n - 1) as usize;
        }
    }

    /// `gg` / `G`.
    pub fn keybindings_jump(&mut self, last: bool) {
        let Some(view) = self.keybindings.as_mut() else {
            return;
        };
        view.selected = if last {
            view.shown().len().saturating_sub(1)
        } else {
            0
        };
    }

    /// Selects shown row `index` (a click).
    pub fn keybindings_select(&mut self, index: usize) {
        let Some(view) = self.keybindings.as_mut() else {
            return;
        };
        if index < view.shown().len() {
            view.selected = index;
            view.focus = KeysFocus::List;
        }
    }

    /// `/`: types in the search box (replacing what was there).
    pub fn keybindings_search(&mut self) {
        let Some(view) = self.keybindings.as_mut() else {
            return;
        };
        view.focus = KeysFocus::Search;
        view.key_filter.clear();
        view.query.select_all();
    }

    /// `r`, `<CR>`, `a`: starts recording keys, to search with them or for
    /// the selected binding.
    pub fn keybindings_record(&mut self, why: Recording) -> Result<(), String> {
        let Some(view) = self.keybindings.as_mut() else {
            return Ok(());
        };
        if why != Recording::Search {
            let row = view.current().ok_or("No binding selected")?;
            if row.source == BindingSource::Plugin {
                return Err(format!(
                    "{} is a plugin's binding: change it in the plugin",
                    row.command
                ));
            }
        } else {
            view.key_filter.clear();
            view.query.set_text("");
            view.selected = 0;
        }
        view.recording = Some(why);
        view.recorded.clear();
        view.focus = KeysFocus::Record;
        Ok(())
    }

    /// Whether keys are being recorded (they all go to the editor, bound
    /// or not).
    pub fn keybindings_recording(&self) -> bool {
        self.keybindings
            .as_ref()
            .is_some_and(|v| v.focus == KeysFocus::Record)
    }

    /// `d`: takes the selected binding away.
    pub fn keybindings_remove(&mut self) -> Result<(), String> {
        let Some(row) = self.keybindings.as_ref().and_then(|v| v.current()).cloned() else {
            return Ok(());
        };
        let Some(mode) = row.mode else {
            return Err(format!("{} has no keys", row.command));
        };
        let command = match row.source {
            BindingSource::User => None,
            BindingSource::Keymap => Some(String::new()),
            BindingSource::Plugin => {
                return Err(format!(
                    "{} is a plugin's binding: change it in the plugin",
                    row.command
                ));
            }
            BindingSource::Removed | BindingSource::Unbound => return Ok(()),
        };
        self.key_edits.push(KeyEdit {
            mode,
            keys: row.keys,
            command,
        });
        Ok(())
    }

    /// `u`: puts back the keymap's binding (taking the user's line out).
    pub fn keybindings_reset(&mut self) -> Result<(), String> {
        let Some(row) = self.keybindings.as_ref().and_then(|v| v.current()).cloned() else {
            return Ok(());
        };
        match (row.source, row.mode) {
            (BindingSource::User | BindingSource::Removed, Some(mode)) => {
                self.key_edits.push(KeyEdit {
                    mode,
                    keys: row.keys,
                    command: None,
                });
                Ok(())
            }
            _ => Err(format!("{} is as the keymap has it", row.command)),
        }
    }

    /// While the search box has focus or keys are being recorded, `key`
    /// goes there. Returns whether it took the key.
    pub fn keybindings_field_key(&mut self, key: KeyPress) -> bool {
        let Some(view) = self.keybindings.as_mut() else {
            return false;
        };
        match view.focus {
            KeysFocus::List => false,
            KeysFocus::Search => {
                let field = &mut view.query;
                let ctrl = key.modifiers.ctrl;
                match key.code {
                    KeyCode::Enter | KeyCode::Down | KeyCode::Tab => view.focus = KeysFocus::List,
                    KeyCode::Esc => {
                        view.query.set_text("");
                        view.focus = KeysFocus::List;
                    }
                    KeyCode::Backspace => field.backspace(),
                    KeyCode::Delete => field.delete(),
                    KeyCode::Left => field.left(),
                    KeyCode::Right => field.right(),
                    KeyCode::Home => field.home(),
                    KeyCode::End => field.end(),
                    KeyCode::Char('u') if ctrl => field.delete_to_start(),
                    KeyCode::Char('w') if ctrl => field.delete_word(),
                    _ => {
                        if let Some(c) = key.as_text() {
                            field.push(c);
                        }
                    }
                }
                view.selected = 0;
                view.scroll.set(0);
                true
            }
            KeysFocus::Record => {
                let plain = key.modifiers == crate::input::Modifiers::default();
                match key.code {
                    // Enter keeps what was pressed (on its own, it's the key).
                    KeyCode::Enter if plain && !view.recorded.is_empty() => {
                        self.keybindings_finish_recording();
                    }
                    KeyCode::Esc if plain => {
                        if view.recording == Some(Recording::Search) {
                            view.key_filter.clear();
                        }
                        view.recording = None;
                        view.recorded.clear();
                        view.focus = KeysFocus::List;
                    }
                    _ if view.recorded.len() < 4 => {
                        view.recorded.push(key);
                        if view.recording == Some(Recording::Search) {
                            view.key_filter = view.recorded.clone();
                            view.selected = 0;
                            view.scroll.set(0);
                        }
                    }
                    _ => {}
                }
                true
            }
        }
    }

    /// `<CR>` while recording: searches with the keys, or binds them.
    fn keybindings_finish_recording(&mut self) {
        let Some(view) = self.keybindings.as_mut() else {
            return;
        };
        let why = view.recording.take();
        let keys = std::mem::take(&mut view.recorded);
        view.focus = KeysFocus::List;
        let Some(why) = why.filter(|w| *w != Recording::Search) else {
            return;
        };
        let Some(row) = view.current().cloned() else {
            return;
        };
        let mode = row.mode.unwrap_or(view.home);
        if keys == row.keys && row.mode.is_some() {
            return;
        }
        let clashes: Vec<String> = view
            .conflicts_with(mode, &keys, &row)
            .iter()
            .map(|r| format!("{} ({})", r.command, notation(&r.keys)))
            .collect();
        if why == Recording::Change && row.mode.is_some() {
            match row.source {
                BindingSource::User => self.key_edits.push(KeyEdit {
                    mode,
                    keys: row.keys.clone(),
                    command: None,
                }),
                BindingSource::Keymap => self.key_edits.push(KeyEdit {
                    mode,
                    keys: row.keys.clone(),
                    command: Some(String::new()),
                }),
                _ => {}
            }
        }
        self.key_edits.push(KeyEdit {
            mode,
            keys: keys.clone(),
            command: Some(row.command.clone()),
        });
        if let Some(view) = self.keybindings.as_mut() {
            view.reselect = Some((row.command.clone(), Some(mode), keys.clone()));
        }
        if !clashes.is_empty() {
            self.set_message(Message::error(format!(
                "{} in {} took them from {}",
                notation(&keys),
                mode.id(),
                clashes.join(", ")
            )));
        }
    }
}

impl KeybindingsView {
    /// The other bindings in `mode` that `keys` would clash with.
    fn conflicts_with(&self, mode: Mode, keys: &[KeyPress], row: &BindingRow) -> Vec<&BindingRow> {
        self.rows
            .iter()
            .filter(|other| {
                other.mode == Some(mode)
                    && other.source != BindingSource::Removed
                    && !(other.command == row.command && other.keys == row.keys)
                    && {
                        let n = other.keys.len().min(keys.len());
                        other.keys[..n] == keys[..n]
                    }
            })
            .collect()
    }
}
