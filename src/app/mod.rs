//! Application wiring: owns the editor, commands, keymap and UI state, and
//! runs the event loop.

mod macros;
mod repeat;
mod terminal;

use std::collections::HashMap;
use std::time::Duration;

use repeat::ChangeRecorder;

use crossterm::event::{self, Event};
use ratatui::{DefaultTerminal, Frame};

use crate::commands::{CommandRegistry, Invocation};
use crate::config::{Config, Theme};
use crate::editor::{DocumentId, Editor, Message, Mode};
use crate::events;
use crate::input::{InputHandler, KeyPress, Keymap, Outcome};
use crate::plugins::PluginHost;
use crate::syntax::SyntaxEngine;
use crate::ui::{self, Views};

/// Upper bound on keys handled between redraws, so the screen still updates
/// during a very long paste.
const MAX_KEYS_PER_FRAME: usize = 256;

pub struct App {
    editor: Editor,
    commands: CommandRegistry,
    keymap: Keymap,
    theme: Theme,
    input: InputHandler,
    /// Scroll state per window and buffer.
    views: Views,
    changes: ChangeRecorder,
    syntax: SyntaxEngine,
    macros: macros::MacroState,
    /// The Lua runtime, created when the first plugin loads.
    plugins: Option<PluginHost>,
    /// True while `.` feeds the last change back in, so it isn't re-recorded.
    replaying: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

impl App {
    pub fn new(config: Config) -> Self {
        let mut commands = CommandRegistry::with_builtins();
        let mut unknown_hook_commands = Vec::new();
        for (kind, hook_commands) in config.hooks {
            for name in &hook_commands {
                if !commands.contains(name) {
                    unknown_hook_commands.push(format!("{name} ({})", kind.id()));
                }
            }
            commands.on_run_commands(kind, hook_commands);
        }

        let mut app = Self {
            editor: Editor::new(config.editor),
            commands,
            keymap: config.keys,
            theme: config.theme,
            input: InputHandler::default(),
            views: Views::default(),
            changes: ChangeRecorder::default(),
            syntax: SyntaxEngine::default(),
            macros: macros::MacroState::default(),
            plugins: None,
            replaying: false,
        };
        app.validate_keymap();
        if !unknown_hook_commands.is_empty() {
            unknown_hook_commands.sort();
            let text = format!(
                "Hooks reference unknown commands: {}",
                unknown_hook_commands.join(", ")
            );
            app.editor.set_message(Message::error(text));
        }
        app
    }

    /// Warns about bindings to commands that don't exist (usually typos).
    fn validate_keymap(&mut self) {
        let mut unknown: Vec<String> = self
            .keymap
            .bound_commands()
            .into_iter()
            .filter(|(_, command)| !self.commands.contains(command))
            .map(|(mode, command)| format!("{command} ({})", mode.id()))
            .collect();
        if !unknown.is_empty() {
            unknown.sort();
            let text = format!("Keymap references unknown commands: {}", unknown.join(", "));
            self.editor.set_message(Message::error(text));
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        &mut self.commands
    }

    /// Loads Lua plugins from `config_dir`: `init.lua`, then `plugins/*.lua`.
    /// Problems are shown on the message line (and returned, so they can be
    /// shown again after startup messages); a broken plugin doesn't stop the
    /// others, or the editor, from loading.
    pub fn load_plugins(&mut self, config_dir: &std::path::Path) -> Option<String> {
        let has_plugins =
            config_dir.join("init.lua").is_file() || config_dir.join("plugins").is_dir();
        if !has_plugins {
            return None;
        }
        let mut errors = match self.plugin_host() {
            Ok(host) => host.load_dir(config_dir),
            Err(error) => vec![error],
        };
        errors.extend(self.install_plugins());
        if errors.is_empty() {
            return None;
        }
        let text = format!("Plugin errors: {}", errors.join("; "));
        self.editor.set_message(Message::error(text.clone()));
        Some(text)
    }

    /// Loads one plugin from source (`name` is used in error messages).
    pub fn load_plugin(&mut self, name: &str, source: &str) -> Result<(), String> {
        self.plugin_host()?.load(name, source)?;
        let errors = self.install_plugins();
        match errors.is_empty() {
            true => Ok(()),
            false => Err(errors.join("; ")),
        }
    }

    fn plugin_host(&mut self) -> Result<&PluginHost, String> {
        if self.plugins.is_none() {
            self.plugins = Some(PluginHost::new()?);
        }
        Ok(self.plugins.as_ref().expect("just created"))
    }

    fn install_plugins(&mut self) -> Vec<String> {
        match &self.plugins {
            Some(host) => host.install(&mut self.commands, &mut self.keymap),
            None => Vec::new(),
        }
    }

    /// Opens files at startup, one buffer each, showing the first.
    pub fn open_all(&mut self, paths: &[std::path::PathBuf]) {
        let mut first = None;
        for path in paths {
            self.open(path);
            first.get_or_insert(self.editor.document.id());
        }
        if let Some(id) = first {
            self.editor.switch_to(id);
        }
    }

    /// Opens a file, reporting any failure on the message line (the editor
    /// carries on with whatever buffer it had).
    pub fn open(&mut self, path: &std::path::Path) {
        if let Err(error) = self.editor.open(path) {
            self.editor.set_message(Message::error(error.to_string()));
        }
        self.dispatch_events();
    }

    pub fn should_quit(&self) -> bool {
        self.editor.should_quit()
    }

    /// Runs the event loop until the user quits.
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let mut shape = None;
        while !self.should_quit() {
            terminal.draw(|frame| self.draw(frame))?;

            let wanted = self
                .editor
                .options
                .cursor_shape
                .for_mode(self.editor.mode());
            if shape != Some(wanted) {
                terminal::set_cursor_shape(wanted)?;
                shape = Some(wanted);
            }

            // Handle every key that's already waiting before drawing again,
            // so a burst of input (fast typing, key repeat, a paste) costs
            // one redraw and one re-highlight rather than one per key.
            self.handle_event(event::read()?);
            let mut handled = 1;
            while handled < MAX_KEYS_PER_FRAME
                && !self.should_quit()
                && event::poll(Duration::ZERO)?
            {
                self.handle_event(event::read()?);
                handled += 1;
            }
        }
        terminal::restore_cursor_shape()
    }

    fn handle_event(&mut self, event: Event) {
        if let Event::Key(event) = event
            && let Some(key) = KeyPress::from_event(event)
        {
            self.handle_key(key);
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let pending = self.input.pending_display();
        if self.syntax.cached_count() > self.editor.buffer_count() {
            let open: Vec<DocumentId> = self.editor.buffers().iter().map(|b| b.id).collect();
            self.syntax.retain(&open);
        }

        // Every buffer on screen needs its highlights, so refresh them all
        // first, then borrow them together.
        let shown: Vec<DocumentId> = self
            .editor
            .windows()
            .layout()
            .windows()
            .into_iter()
            .filter_map(|id| self.editor.window_view(id).map(|(doc, _)| doc.id()))
            .collect();
        let mut syntax = HashMap::new();
        if self.editor.options.syntax_highlighting {
            for &id in &shown {
                if let Some(document) = self.editor.document_by_id(id) {
                    self.syntax.refresh(document);
                }
            }
            for &id in &shown {
                if let Some(highlights) = self.syntax.get(id) {
                    syntax.insert(id, highlights);
                }
            }
        }

        ui::draw(
            frame,
            &self.editor,
            &self.theme,
            &mut self.views,
            &pending,
            &syntax,
        );
    }

    /// Handles one key press. Public so tests can drive the app directly.
    pub fn handle_key(&mut self, key: KeyPress) {
        self.record_macro_key(key);
        if self.replaying {
            self.process_key(key);
            return;
        }

        let edits_before = self.editor.document.edit_count();

        // Record each sequence from idle to idle, for `.`.
        if self.is_idle() {
            self.changes.begin(edits_before);
        }
        self.changes.push(key);
        self.process_key(key);
        // `:` commands and picker choices aren't repeated by `.`.
        if matches!(self.editor.mode(), Mode::Command | Mode::Picker) {
            self.changes.note_command_line();
        }
        if self.is_idle() {
            self.changes.finish(self.editor.document.edit_count());
        }

        if self.editor.document.edit_count() != edits_before {
            self.editor.emit(events::Event::BufferChanged);
        }
        self.dispatch_events();
    }

    /// Runs hooks for the events queued while handling a key. Hooks can
    /// queue more events, so this repeats, up to a limit that stops two
    /// hooks from triggering each other forever.
    fn dispatch_events(&mut self) {
        const MAX_ROUNDS: usize = 8;
        for _ in 0..MAX_ROUNDS {
            let events = self.editor.take_events();
            if events.is_empty() {
                return;
            }
            for event in events {
                if let Err(error) = self.commands.fire(&event, &mut self.editor) {
                    self.editor.set_message(Message::error(error));
                }
            }
            // A hook's edits form their own undo step.
            self.editor.finish_command();
        }
        if !self.editor.take_events().is_empty() {
            self.editor.set_message(Message::error(format!(
                "Hooks kept triggering each other; stopped after {MAX_ROUNDS} rounds"
            )));
        }
    }

    /// Normal mode with nothing typed but not yet run: the boundary between
    /// one command sequence and the next.
    fn is_idle(&self) -> bool {
        self.editor.mode() == Mode::Normal && self.input.is_idle()
    }

    fn process_key(&mut self, key: KeyPress) {
        for outcome in self.input.handle(key, self.editor.mode(), &self.keymap) {
            match outcome {
                Outcome::Execute {
                    command,
                    count,
                    register,
                    char_arg: None,
                } if self.commands.takes_char(&command) => {
                    // `f`, `t`, `r`...: the next key is the argument.
                    self.input.hold_last_sequence();
                    self.input.await_char(command, count, register);
                }
                Outcome::Execute {
                    command,
                    count,
                    register,
                    char_arg,
                } => {
                    let invocation = Invocation {
                        count,
                        register,
                        char_arg,
                        ..Invocation::default()
                    };
                    self.execute(&command, invocation);
                }
                Outcome::InsertChar(c) => self.editor.insert_text_char(c),
                // Like Vim, an invalid key after an operator cancels it.
                Outcome::Unbound if self.editor.mode() == Mode::OperatorPending => {
                    self.editor.set_mode(Mode::Normal);
                }
                Outcome::Pending | Outcome::Unbound => {}
            }
        }

        // Keep e.g. `d` or `df` on display while waiting for the rest.
        if self.editor.mode() != Mode::OperatorPending && !self.input.is_awaiting_char() {
            self.input.release_held();
        }
    }

    /// Runs a named command, reporting failures on the message line.
    pub fn execute(&mut self, command: &str, invocation: Invocation) {
        self.editor.clear_message();
        if let Err(error) = self.commands.execute(command, &mut self.editor, invocation) {
            self.editor.set_message(Message::error(error));
            self.note_command_failure();
        }
        if self.editor.mode() == Mode::OperatorPending {
            self.input.hold_last_sequence();
        }
        self.editor.finish_command();

        // A command that wants the next key (`q` before a register).
        if let Some(command) = self.editor.take_char_request() {
            self.input.hold_last_sequence();
            self.input.await_char(command, None, None);
        }
        if let Some(request) = self.editor.take_repeat_request() {
            self.repeat_last_change(request.count);
        }
        if let Some(request) = self.editor.take_macro_request() {
            self.handle_macro_request(request);
        }
    }

    /// `.`: feeds the last change's keys back in.
    fn repeat_last_change(&mut self, count: Option<usize>) {
        let Some(change) = self.changes.last_change() else {
            return;
        };
        let keys = change.keys(count);
        self.changes.note_replay();
        self.replaying = true;
        for key in keys {
            self.process_key(key);
        }
        self.replaying = false;
    }
}
