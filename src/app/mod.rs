//! Application wiring: owns the editor, commands, keymap and UI state, and
//! runs the event loop.

mod background;
mod macros;
mod normal;
mod repeat;
mod terminal;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use repeat::{ChangeRecorder, EditStamp};

use crossterm::event::{self, Event};
use ratatui::{DefaultTerminal, Frame};

use crate::clipboard::{Clipboard, LazySystemClipboard};
use crate::commands::{CommandRegistry, Invocation};
use crate::config::{Config, Theme};
use crate::editor::{DocumentId, Editor, Message, Mode};
use crate::events;
use crate::input::{InputHandler, KeyCode, KeyPress, Keymap, Lookup, Outcome};
use crate::plugins::PluginHost;
use crate::syntax::SyntaxEngine;
use crate::ui::{self, Views};

/// Upper bound on keys handled between redraws, so the screen still updates
/// during a very long paste.
const MAX_KEYS_PER_FRAME: usize = 256;

/// How often the event loop checks for background results while a job is
/// running (it blocks on input when nothing is).
const BACKGROUND_POLL: Duration = Duration::from_millis(30);

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
    /// Work running on other threads (the file picker's listing).
    background: background::Background,
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

        let mut editor = Editor::new(config.editor);
        editor.languages = config.languages;
        let mut app = Self {
            editor,
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
            background: background::Background::default(),
        };
        let mode = app.editor.options.clipboard;
        app.editor
            .registers
            .set_clipboard(Box::new(LazySystemClipboard::default()), mode);
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
    /// Opens the files named on the command line, or with none, shows the
    /// splash screen.
    pub fn open_all(&mut self, paths: &[std::path::PathBuf]) {
        if paths.is_empty() {
            self.editor.show_splash();
            return;
        }
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
        terminal::enable_bracketed_paste()?;
        terminal::enable_focus_reporting()?;
        let mut shape = None;
        // When a half-typed key sequence gives up waiting.
        let mut deadline: Option<Instant> = None;
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

            // Wait for input, for a half-typed key sequence to time out, or
            // (while a background job runs) for its results.
            let wait = match (deadline, self.background.busy()) {
                (Some(due), busy) => {
                    let left = due.saturating_duration_since(Instant::now());
                    Some(if busy {
                        left.min(BACKGROUND_POLL)
                    } else {
                        left
                    })
                }
                (None, true) => Some(BACKGROUND_POLL),
                (None, false) => None,
            };
            let input_ready = match wait {
                Some(wait) => event::poll(wait)?,
                None => true,
            };
            self.handle_background();
            if !input_ready {
                if deadline.is_some_and(|due| Instant::now() >= due) {
                    self.handle_key_timeout();
                    deadline = None;
                }
                continue;
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
            deadline = self.key_deadline();
        }
        terminal::disable_focus_reporting()?;
        terminal::disable_bracketed_paste()?;
        terminal::restore_cursor_shape()
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(event) => {
                if let Some(key) = KeyPress::from_event(event) {
                    self.handle_key(key);
                }
            }
            Event::Paste(text) => self.handle_paste(&text),
            Event::FocusGained => self.handle_focus_gained(),
            _ => {}
        }
    }

    /// The terminal came back into focus: another program may have changed
    /// open files meanwhile. Public so tests can trigger it.
    pub fn handle_focus_gained(&mut self) {
        self.editor.check_disk_changes();
        self.dispatch_events();
    }

    /// Text pasted into the terminal (bracketed paste). It goes in exactly as
    /// copied: no auto-indent, no key bindings, one undo step.
    ///
    /// In insert mode it's typed at the cursor (and joins the insert
    /// session); on a line that's only indentation so far, a paste with its
    /// own indentation replaces it. In normal and visual mode it's put in before the cursor,
    /// like `P`. Prompts and pickers take the first line.
    pub fn handle_paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.input.reset();
        let before = self.edit_stamp();
        match self.editor.mode() {
            Mode::Insert => {
                // On a line that's only (auto-)indentation so far, a paste
                // that brings its own indentation replaces it rather than
                // adding to it.
                let doc = &mut self.editor.document;
                let cursor = doc.cursor();
                let line = doc.buffer().line(cursor.row);
                let only_indent_before =
                    cursor.col > 0 && line.chars().take(cursor.col).all(char::is_whitespace);
                if only_indent_before && text.starts_with([' ', '\t']) {
                    doc.delete(cursor.with_col(0), cursor);
                    doc.set_cursor(cursor.with_col(0));
                }
                doc.insert_str(&text);
                self.editor.note_typed(&text);
            }
            Mode::Command | Mode::Picker => {
                let first_line = text.lines().next().unwrap_or_default();
                first_line
                    .chars()
                    .for_each(|c| self.editor.insert_text_char(c));
            }
            Mode::Normal | Mode::Visual | Mode::OperatorPending => {
                self.editor.set_mode(Mode::Normal);
                let doc = &mut self.editor.document;
                let end = doc.insert(doc.cursor(), &text);
                doc.set_cursor(end.with_col(end.col.saturating_sub(1)));
                self.editor.finish_command();
            }
        }
        if self.edit_stamp().edited_since(before) {
            self.editor.emit(events::Event::BufferChanged);
        }
        self.editor.update_splash();
        self.dispatch_events();
    }

    /// Replaces the clipboard behind `"+` (the system one by default).
    /// Tests use an in-memory clipboard.
    pub fn use_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        let mode = self.editor.options.clipboard;
        self.editor.registers.set_clipboard(clipboard, mode);
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
                    self.syntax.refresh(document, &self.editor.languages);
                }
            }
            for &id in &shown {
                if let Some(highlights) = self.syntax.get(id) {
                    syntax.insert(id, highlights);
                }
            }
        }

        let splash = if self.editor.splash_visible() {
            self.splash_entries()
        } else {
            Vec::new()
        };
        ui::draw(
            frame,
            &self.editor,
            &self.theme,
            &mut self.views,
            &pending,
            &syntax,
            &splash,
        );
    }

    /// The splash screen's menu, with the keys actually bound (so a
    /// rebinding shows up), leaving out commands that aren't bound.
    fn splash_entries(&self) -> Vec<ui::SplashEntry> {
        let bound = |command: &str| {
            self.keymap
                .keys_for(Mode::Normal, command)
                .map(|keys| keys.iter().map(ToString::to_string).collect::<String>())
        };
        let entry = |keys: Option<String>, description| {
            keys.map(|keys| ui::SplashEntry { keys, description })
        };
        [
            entry(Some(":e <file>".into()), "open a file"),
            entry(bound("file_picker"), "find a file"),
            entry(bound("command_palette"), "browse commands"),
            entry(bound("insert_mode"), "start writing"),
            entry(Some(":q".into()), "quit"),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// When the key timeout should end the sequence being typed, if any.
    fn key_deadline(&self) -> Option<Instant> {
        let millis = self.editor.options.key_timeout;
        (millis > 0 && self.input.has_pending_keys())
            .then(|| Instant::now() + Duration::from_millis(millis))
    }

    /// Handles one key press. Public so tests can drive the app directly.
    pub fn handle_key(&mut self, key: KeyPress) {
        if let Some([esc, key]) = self.split_alt(key) {
            self.handle_key(esc);
            self.handle_key(key);
            return;
        }
        self.record_macro_key(key);
        if self.replaying {
            self.process_key(key);
            return;
        }
        self.tracked(Some(key), |app| app.process_key(key));
    }

    /// The key timeout passed with a key sequence half typed (see
    /// [`InputHandler::timeout`]). Public so tests can trigger it.
    pub fn handle_key_timeout(&mut self) {
        self.tracked(None, |app| {
            let outcomes = app.input.timeout(app.editor.mode(), &app.keymap);
            app.apply_outcomes(outcomes);
        });
    }

    /// Terminals send Alt+x as Esc then x, so Esc quickly followed by a key
    /// (`<Esc>` then `j`, typed fast or pasted) arrives as Alt+j. Like
    /// Neovim, an Alt key with no binding in this mode is taken as Esc
    /// followed by the key.
    fn split_alt(&self, key: KeyPress) -> Option<[KeyPress; 2]> {
        if !key.modifiers.alt || self.keymap.lookup(self.editor.mode(), &[key]) != Lookup::NoMatch {
            return None;
        }
        let mut modifiers = key.modifiers;
        modifiers.alt = false;
        Some([
            KeyPress::plain(KeyCode::Esc),
            KeyPress::new(key.code, modifiers),
        ])
    }

    fn edit_stamp(&self) -> EditStamp {
        EditStamp {
            document: self.editor.document.id(),
            edits: self.editor.document.edit_count(),
        }
    }

    /// Runs `handle`, recording keys for `.` and firing change events.
    fn tracked(&mut self, key: Option<KeyPress>, handle: impl FnOnce(&mut Self)) {
        let before = self.edit_stamp();

        // Record each sequence from idle to idle, for `.`.
        if self.is_idle() {
            self.changes.begin(before);
        }
        if let Some(key) = key {
            if self.input.is_idle() {
                self.changes.mark_binding_start();
            }
            self.changes.push(key);
        }
        // Counts are marked so `.` with a count can replace them all. Only
        // normal-mode counts (not a visual selection's motion counts).
        let counts_count = matches!(self.editor.mode(), Mode::Normal | Mode::OperatorPending);
        handle(self);
        if key.is_some() && counts_count && self.input.took_count() {
            self.changes.mark_count();
        }
        if let Some(shape) = self.editor.take_visual_shape() {
            self.changes.note_visual(shape);
        }
        // `:` commands and picker choices aren't repeated by `.`; a search
        // is, as an operator's motion (`d/foo<CR>`).
        let at_colon_prompt =
            self.editor.mode() == Mode::Command && !self.editor.command_line.prompt().is_search();
        if at_colon_prompt || self.editor.mode() == Mode::Picker {
            self.changes.note_command_line();
        }
        if self.is_idle() {
            self.changes.finish(self.edit_stamp());
        }

        if self.edit_stamp().edited_since(before) {
            self.editor.emit(events::Event::BufferChanged);
        }
        self.editor.update_splash();
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
        let outcomes = self.input.handle(key, self.editor.mode(), &self.keymap);
        self.apply_outcomes(outcomes);
    }

    fn apply_outcomes(&mut self, outcomes: Vec<Outcome>) {
        for outcome in outcomes {
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

        // A `:s///c` question answered with a key that isn't a character
        // (`<Esc>`) ends the session.
        if self.editor.is_confirming() && !self.input.is_awaiting_char() {
            self.editor.cancel_confirm();
            self.editor.finish_command();
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
        if let Some(error) = self.editor.registers.take_clipboard_error() {
            self.editor.set_message(Message::error(error));
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
        self.start_background_requests();
        let runs = self.editor.take_normal_queue();
        if !runs.is_empty() {
            self.run_normal(runs);
        }
    }

    /// `.`: feeds the last change's keys back in.
    fn repeat_last_change(&mut self, count: Option<usize>) {
        let Some(change) = self.changes.last_change_mut() else {
            return;
        };
        let pastes = matches!(
            self.keymap.lookup(Mode::Normal, &change.body()),
            Lookup::Command("paste_after" | "paste_before")
        );
        if pastes {
            change.advance_numbered_register();
        }
        // As in Vim, a count given to `.` becomes the change's count, so a
        // later `.` without one uses it too. A visual change has no count
        // of its own (its size is the selection's), so it ignores one.
        let visual = change.visual();
        if let Some(count) = count
            && visual.is_none()
        {
            change.set_count(count);
        }
        let keys = change.keys(None);
        if let Some(shape) = visual {
            self.editor.select_shape(shape);
        }
        self.changes.note_replay();
        self.replaying = true;
        for key in keys {
            self.process_key(key);
        }
        self.replaying = false;
    }
}
