//! Application wiring: owns the editor, commands, keymap and UI state, and
//! runs the event loop.

mod background;
mod macros;
mod mouse;
mod normal;
mod repeat;
mod terminal;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use repeat::{ChangeRecorder, EditStamp};

use crossterm::event::{self, Event};
use ratatui::{DefaultTerminal, Frame};

use crate::clipboard::{self, Clipboard};
use crate::commands::{CommandRegistry, Invocation};
use crate::config::StatusElement;
use crate::config::{Config, Theme};
use crate::editor::{DocumentId, Editor, Message, Mode};
use crate::events;
use crate::input::{InputHandler, KeyCode, KeyPress, Keymap, Lookup, Outcome, paste_keys};
use crate::plugins::PluginHost;
use crate::syntax::SyntaxEngine;
use crate::ui::{self, Views};

/// Upper bound on keys handled between redraws, so the screen still updates
/// during a very long paste.
const MAX_KEYS_PER_FRAME: usize = 256;

/// How often the event loop checks for background results while a job is
/// running (it blocks on input when nothing is).
/// The pointer moving with no button held (which only changes its shape).
fn is_pointer_move(event: &Event) -> bool {
    matches!(event, Event::Mouse(m) if m.kind == event::MouseEventKind::Moved)
}

const BACKGROUND_POLL: Duration = Duration::from_millis(30);

/// How often open files are checked for changes made by other programs,
/// with `watch-files`.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

pub struct App {
    editor: Editor,
    commands: CommandRegistry,
    keymap: Keymap,
    /// Where things were last drawn, for the mouse.
    screen_map: ui::ScreenMap,
    /// What a drag that's going on is doing.
    dragging: Option<mouse::Drag>,
    /// The last click: when, where, and how many in a row (for double and
    /// triple clicks).
    last_click: Option<(Instant, ratatui::layout::Position, u8)>,
    /// The mouse pointer's shape wanted over what's under it (a CSS name).
    pointer: &'static str,
    /// The keymap with the open popup's and the shown sidebar view's own
    /// keys, while they have some.
    popup_keymap: Option<Keymap>,
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
    /// The text of a paste being replayed key by key (from a macro or
    /// `.`), between `<PasteStart>` and `<PasteEnd>`.
    paste: Option<String>,
    /// Work running on other threads (the file picker's listing).
    background: background::Background,
    /// The last error from each plugin status function, so it's shown once.
    plugin_status_errors: HashMap<String, String>,
    /// Language servers for the open files.
    /// Shared with the editor's formatter, which waits on it mid-command.
    lsp: std::rc::Rc<std::cell::RefCell<crate::lsp::Lsp>>,
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
            popup_keymap: None,
            screen_map: ui::ScreenMap::default(),
            dragging: None,
            last_click: None,
            pointer: "default",
            theme: config.theme,
            input: InputHandler::default(),
            views: Views::default(),
            changes: ChangeRecorder::default(),
            syntax: SyntaxEngine::default(),
            macros: macros::MacroState::default(),
            plugins: None,
            replaying: false,
            paste: None,
            background: background::Background::default(),
            plugin_status_errors: HashMap::new(),
            lsp: Default::default(),
        };
        let formatter = crate::lsp::LspFormatter::new(std::rc::Rc::clone(&app.lsp));
        app.editor.set_formatter(Box::new(formatter));
        let mode = app.editor.options.clipboard;
        let provider = app.editor.options.clipboard_provider;
        app.editor
            .registers
            .set_clipboard(clipboard::for_provider(provider), mode);
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
        // A `plugin:name` status line element no plugin provides.
        let layout = &self.editor.options.statusline_layout;
        for element in layout
            .left
            .iter()
            .chain(&layout.center)
            .chain(&layout.right)
        {
            if let StatusElement::Plugin(name) = element
                && !self.plugins.as_ref().is_some_and(|p| p.has_status(name))
            {
                errors.push(format!(
                    "statusline-layout: no plugin provides plugin:{name}"
                ));
            }
        }
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
        let Some(host) = &self.plugins else {
            return Vec::new();
        };
        let errors = host.install(&mut self.commands, &mut self.keymap);
        self.editor.add_sidebar_views(host.take_new_views());
        errors
    }

    /// Before drawing: calls the `lines` function of the plugin view the
    /// sidebar shows, if it needs making. An error shows as its one line
    /// (and as a message).
    fn make_plugin_view(&mut self) {
        let (Some(plugins), Some(name)) = (&self.plugins, self.editor.plugin_view_to_make()) else {
            return;
        };
        let mut cx = crate::commands::Context {
            editor: &mut self.editor,
            registry: &self.commands,
            count: None,
            register: None,
            args: &[],
            bang: false,
            char_arg: None,
            range: None,
            arg_text: "",
        };
        let lines = match plugins.view_lines(&name, &mut cx) {
            Ok(lines) => lines,
            Err(error) => {
                let error = format!("sidebar view {name}: {error}");
                self.editor.set_message(Message::error(error.clone()));
                vec![vec![crate::editor::popup::PopupSpan {
                    text: error,
                    style: Some("ui.message.error".into()),
                }]]
            }
        };
        self.editor.set_plugin_view_lines(&name, lines);
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
        self.lsp.borrow_mut().enable();
        // A terminal that doesn't answer is used the old way.
        let enhanced_keys = self.editor.options.kitty_keyboard
            && terminal::enable_keyboard_enhancement().unwrap_or(false);
        let mut shape = None;
        // Mouse reporting follows the `mouse` option (`:set mouse` too).
        let mut mouse = false;
        let mut pointer = "default";
        // When a half-typed key sequence gives up waiting.
        let mut deadline: Option<Instant> = None;
        // When open files are next checked for outside changes.
        let mut next_watch = Instant::now() + WATCH_INTERVAL;
        // Drawn after input, and otherwise only when something changes (a
        // server's message, a background job's results): waking to check
        // for those doesn't redraw.
        let mut redraw = true;
        while !self.should_quit() {
            if redraw {
                terminal.draw(|frame| self.draw(frame))?;
                // The shell's terminal takes the size its panel was drawn at.
                self.editor.apply_terminal_size();
                redraw = false;
            }

            let wanted = self
                .editor
                .options
                .cursor_shape
                .for_mode(self.editor.mode());
            let wanted = if self.editor.replacing() {
                self.editor.options.cursor_shape.replace
            } else {
                wanted
            };
            if shape != Some(wanted) {
                terminal::set_cursor_shape(wanted)?;
                shape = Some(wanted);
            }
            if mouse != self.editor.options.mouse {
                mouse = self.editor.options.mouse;
                terminal::set_mouse_capture(mouse)?;
            }
            if mouse && pointer != self.pointer {
                pointer = self.pointer;
                terminal::set_pointer_shape(pointer)?;
            }

            // Wait for input, for a half-typed key sequence to time out, for
            // the next check of open files, or (while a background job runs)
            // for its results.
            let watch = self.editor.options.watch_files.then_some(next_watch);
            let due = [deadline, watch].into_iter().flatten().min();
            // Language servers' messages arrive on their own threads, so
            // keep checking while any are running.
            let busy = self.background.busy()
                || self.lsp.borrow().active()
                || self.editor.terminal_running()
                || self.editor.git_job_running();
            let wait = match (due, busy) {
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
            redraw |= self.handle_background();
            redraw |= self.editor.pump_terminal();
            redraw |= self.editor.pump_git_job();
            if !input_ready {
                redraw |= self.update_lsp();
                if deadline.is_some_and(|due| Instant::now() >= due) {
                    self.handle_key_timeout();
                    deadline = None;
                    redraw = true;
                }
                if watch.is_some_and(|due| Instant::now() >= due) {
                    self.check_files_while_idle();
                    next_watch = Instant::now() + WATCH_INTERVAL;
                    redraw = true;
                }
                continue;
            }
            // Handle every key that's already waiting before drawing again,
            // so a burst of input (fast typing, key repeat, a paste) costs
            // one redraw and one re-highlight rather than one per key. The
            // pointer moving only changes its shape: no redraw for that.
            let event = event::read()?;
            redraw = !is_pointer_move(&event);
            self.handle_event(event);
            let mut handled = 1;
            while handled < MAX_KEYS_PER_FRAME
                && !self.should_quit()
                && event::poll(Duration::ZERO)?
            {
                let event = event::read()?;
                redraw |= !is_pointer_move(&event);
                self.handle_event(event);
                handled += 1;
            }
            deadline = self.key_deadline();
        }
        self.lsp.borrow_mut().shutdown();
        if enhanced_keys {
            terminal::disable_keyboard_enhancement()?;
        }
        if mouse {
            terminal::set_mouse_capture(false)?;
        }
        if pointer != "default" {
            terminal::set_pointer_shape("default")?;
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
            Event::Mouse(event) => self.handle_mouse(event),
            _ => {}
        }
    }

    /// The terminal came back into focus: another program may have changed
    /// open files meanwhile. Public so tests can trigger it.
    pub fn handle_focus_gained(&mut self) {
        self.editor.check_disk_changes();
        // Git may have run meanwhile (a commit, `git add`).
        self.editor.git_refresh_all();
        self.dispatch_events();
    }

    /// With `watch-files`, the event loop's regular check of open files for
    /// changes made by other programs: as on focus, unmodified buffers are
    /// reloaded and others warned about. Only between commands in normal
    /// mode, so a reload never lands mid-insert or mid-command. Public so
    /// tests can trigger it.
    pub fn check_files_while_idle(&mut self) {
        if self.editor.options.watch_files && self.is_idle() {
            self.editor.check_disk_changes();
            self.dispatch_events();
        }
    }

    /// Text pasted into the terminal (bracketed paste). It goes in exactly as
    /// copied: no auto-indent, no key bindings, one undo step (see
    /// `App::paste`). Macros and `.` record it as `<PasteStart>`, its text
    /// and `<PasteEnd>`, and put it in the same way when replayed.
    pub fn handle_paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let keys = paste_keys(&text);
        for &key in &keys {
            self.record_macro_key(key);
        }
        if self.replaying {
            self.paste(&text);
            return;
        }
        self.tracked(&keys, |app| app.paste(&text));
    }

    /// Puts in pasted `text`.
    ///
    /// In insert mode it's typed at the cursor (and joins the insert
    /// session); on a line that's only indentation so far, a paste with its
    /// own indentation replaces it. In normal and visual mode it's put in
    /// before the cursor, like `P`. Prompts and pickers take the first line.
    fn paste(&mut self, text: &str) {
        self.input.reset();
        self.editor.note_key_for_completion();
        self.editor.end_completion_unless_used();
        self.editor.note_key_for_hover();
        self.editor.end_hover_unless_used();
        match self.editor.mode() {
            Mode::Insert => {
                // On a line that's only (auto-)indentation so far, a paste
                // that brings its own indentation replaces it rather than
                // adding to it.
                if self.editor.replacing() {
                    // Typed over the text, as if each char were typed.
                    text.chars().for_each(|c| self.editor.insert_text_char(c));
                    return;
                }
                let doc = &mut self.editor.document;
                let cursor = doc.cursor();
                let line = doc.buffer().line(cursor.row);
                let only_indent_before =
                    cursor.col > 0 && line.chars().take(cursor.col).all(char::is_whitespace);
                if only_indent_before && text.starts_with([' ', '\t']) {
                    doc.delete(cursor.with_col(0), cursor);
                    doc.set_cursor(cursor.with_col(0));
                }
                doc.insert_str(text);
                self.editor.note_typed(text);
            }
            Mode::Command | Mode::Picker | Mode::Find | Mode::Input => {
                let first_line = text.lines().next().unwrap_or_default();
                first_line
                    .chars()
                    .for_each(|c| self.editor.insert_text_char(c));
                self.editor.search_symbols_after_key();
            }
            Mode::Normal | Mode::Visual | Mode::OperatorPending => {
                self.editor.set_mode(Mode::Normal);
                let doc = &mut self.editor.document;
                let end = doc.insert(doc.cursor(), text);
                doc.set_cursor(end.with_col(end.col.saturating_sub(1)));
                self.editor.finish_command();
            }
            Mode::Terminal => self.editor.terminal_paste(text),
            Mode::Popup | Mode::Git => {}
            // Nothing to paste into.
            Mode::Sidebar => {}
        }
    }

    /// Puts in a replayed paste whose `<PasteEnd>` never came (a macro
    /// edited by hand), so it doesn't swallow the keys after it.
    fn finish_open_paste(&mut self) {
        if let Some(text) = self.paste.take() {
            self.paste(&text);
        }
    }

    /// Replaces the clipboard behind `"+` (the system one by default).
    /// Tests use an in-memory clipboard.
    pub fn use_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        let mode = self.editor.options.clipboard;
        self.editor.registers.set_clipboard(clipboard, mode);
    }

    /// Calls the Lua functions behind `plugin:name` status line elements, so
    /// the status line can show what they return. An error is shown once
    /// (until it changes), and the element is left empty.
    fn refresh_plugin_statuses(&mut self) {
        let Some(plugins) = &self.plugins else {
            return;
        };
        let layout = &self.editor.options.statusline_layout;
        let names: Vec<String> = layout
            .left
            .iter()
            .chain(&layout.center)
            .chain(&layout.right)
            .filter_map(|element| match element {
                StatusElement::Plugin(name) if plugins.has_status(name) => Some(name.clone()),
                _ => None,
            })
            .collect();
        for name in names {
            let mut cx = crate::commands::Context {
                editor: &mut self.editor,
                registry: &self.commands,
                count: None,
                register: None,
                args: &[],
                bang: false,
                char_arg: None,
                range: None,
                arg_text: "",
            };
            let text = match plugins.status(&name, &mut cx) {
                Ok(text) => {
                    self.plugin_status_errors.remove(&name);
                    text
                }
                Err(error) => {
                    let error = format!("plugin:{name}: {error}");
                    if self.plugin_status_errors.get(&name) != Some(&error) {
                        self.editor.set_message(Message::error(error.clone()));
                        self.plugin_status_errors.insert(name.clone(), error);
                    }
                    String::new()
                }
            };
            self.editor.set_plugin_status(&name, text);
        }
    }

    /// The language servers, to replace how they're started (tests).
    pub fn lsp_mut(&mut self) -> std::cell::RefMut<'_, crate::lsp::Lsp> {
        self.lsp.borrow_mut()
    }

    /// Handles what language servers sent and sends them the latest
    /// changes. Called before each draw; public so tests can drive it.
    /// Returns whether anything happened that changes what's shown.
    pub fn update_lsp(&mut self) -> bool {
        self.lsp.borrow_mut().update(&mut self.editor)
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        self.update_lsp();
        self.refresh_plugin_statuses();
        self.make_plugin_view();
        self.editor.update_git_changes();
        if !self.editor.options.inlay_hints {
            self.editor.clear_inlay_hints();
        }
        let pending = self.input.pending_display();
        let count = self.editor.buffer_count();
        if self.syntax.cached_count() > count || self.editor.syntax_trees.count() > count {
            let open: Vec<DocumentId> = self.editor.buffers().iter().map(|b| b.id).collect();
            self.syntax.retain(&open);
            self.editor.syntax_trees.retain(&open);
        }

        let splash = if self.editor.splash_visible() {
            self.splash_entries()
        } else {
            Vec::new()
        };
        // The find box's key hints, from the keymap (so rebinding shows).
        let find_keys = if self.editor.mode() == Mode::Find {
            ui::FindKeys::from_keymap(&self.keymap)
        } else {
            ui::FindKeys::default()
        };
        let input_keys = if self.editor.input_box().is_some() {
            let label = |command| {
                self.keymap
                    .hint_keys(Mode::Input, command)
                    .map(|keys| ui::key_label(&keys, cfg!(target_os = "macos")))
            };
            ui::InputKeys {
                submit: label("input_submit"),
                cancel: label("input_cancel"),
            }
        } else {
            ui::InputKeys::default()
        };
        let terminal_keys = if self.editor.terminal_panel().is_some() {
            let label = |mode, command| {
                self.keymap
                    .hint_keys(mode, command)
                    .map(|keys| ui::key_label(&keys, cfg!(target_os = "macos")))
            };
            ui::TerminalKeys {
                leave: label(Mode::Terminal, "terminal_leave"),
                hide: label(Mode::Terminal, "terminal_toggle"),
                focus: label(Mode::Normal, "terminal_toggle"),
            }
        } else {
            ui::TerminalKeys::default()
        };
        self.screen_map = ui::draw(
            frame,
            &self.editor,
            &self.theme,
            &mut self.views,
            &pending,
            self.editor
                .options
                .syntax_highlighting
                .then_some(&mut self.syntax),
            &ui::KeyHints {
                splash,
                find: find_keys,
                terminal: terminal_keys,
                input: input_keys,
            },
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

    /// The keymap for the current mode: with a popup's or a sidebar
    /// view's own keys while it has focus.
    fn mode_keymap(&self) -> &Keymap {
        match (&self.popup_keymap, self.editor.mode()) {
            (Some(overlay), Mode::Popup | Mode::Sidebar) => overlay,
            _ => &self.keymap,
        }
    }

    /// After a key: a popup it opened, or a sidebar view it showed, gets its
    /// keys bound (in a copy of the keymap); a popup without focus from
    /// before goes.
    fn after_popup_key(&mut self) {
        let popup_opened = self.editor.take_popup_keys_changed();
        let view_changed = self.editor.take_view_keys_changed();
        if !popup_opened {
            self.editor.end_popup_unless_new(true);
        }
        if !popup_opened && !view_changed {
            return;
        }
        let popup_keys = self.editor.popup().map_or(&[][..], |p| p.keys.as_slice());
        let view_keys = self.editor.plugin_view_keys();
        self.popup_keymap = (!popup_keys.is_empty() || !view_keys.is_empty()).then(|| {
            let mut keymap = self.keymap.clone();
            for (mode, keys) in [(Mode::Popup, popup_keys), (Mode::Sidebar, &view_keys)] {
                for (notation, command) in keys {
                    // Checked when the plugin made them.
                    if let Ok(keys) = crate::input::parse_keys(notation) {
                        keymap.bind(mode, &keys, command.clone());
                    }
                }
            }
            keymap
        });
    }

    /// When the key timeout should end the sequence being typed, if any.
    fn key_deadline(&self) -> Option<Instant> {
        let millis = self.editor.options.key_timeout;
        (millis > 0 && self.input.has_pending_keys())
            .then(|| Instant::now() + Duration::from_millis(millis))
    }

    /// Handles one key press. Public so tests can drive the app directly.
    pub fn handle_key(&mut self, key: KeyPress) {
        let key = self.as_legacy(key);
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
        self.tracked(&[key], |app| app.process_key(key));
    }

    /// The key timeout passed with a key sequence half typed (see
    /// [`InputHandler::timeout`]). Public so tests can trigger it.
    pub fn handle_key_timeout(&mut self) {
        self.tracked(&[], |app| {
            let mode = app.editor.mode();
            let keymap = match (&app.popup_keymap, mode) {
                (Some(overlay), Mode::Popup | Mode::Sidebar) => overlay,
                _ => &app.keymap,
            };
            let outcomes = app.input.timeout(mode, keymap);
            app.apply_outcomes(outcomes);
            app.after_popup_key();
        });
    }

    /// With the kitty keyboard protocol, `<C-[>`, `<C-i>` and `<C-m>` arrive
    /// as themselves rather than as `<Esc>`, `<Tab>` and `<CR>`. `<C-[>` is
    /// always `<Esc>` (Vim users press it for that); the others act as Tab
    /// and Enter unless a binding in the current mode uses them, so binding
    /// `<C-i>` is how to get it apart from Tab.
    fn as_legacy(&self, key: KeyPress) -> KeyPress {
        match key.legacy_equivalent() {
            Some(legacy)
                if key.code == KeyCode::Char('[')
                    || !self.mode_keymap().uses(self.editor.mode(), key) =>
            {
                legacy
            }
            _ => key,
        }
    }

    /// Terminals send Alt+x as Esc then x, so Esc quickly followed by a key
    /// (`<Esc>` then `j`, typed fast or pasted) arrives as Alt+j. Like
    /// Neovim, an Alt key with no binding in this mode is taken as Esc
    /// followed by the key.
    fn split_alt(&self, key: KeyPress) -> Option<[KeyPress; 2]> {
        if !key.modifiers.alt
            || self.mode_keymap().lookup(self.editor.mode(), &[key]) != Lookup::NoMatch
        {
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
    fn tracked(&mut self, keys: &[KeyPress], handle: impl FnOnce(&mut Self)) {
        let before = self.edit_stamp();

        // Record each sequence from idle to idle, for `.`.
        if self.is_idle() {
            self.changes.begin(before);
        }
        if !keys.is_empty() && self.input.is_idle() && self.paste.is_none() {
            self.changes.mark_binding_start();
        }
        for &key in keys {
            self.changes.push(key);
        }
        // Counts are marked so `.` with a count can replace them all. Only
        // normal-mode counts (not a visual selection's motion counts).
        let counts_count = matches!(self.editor.mode(), Mode::Normal | Mode::OperatorPending);
        handle(self);
        if !keys.is_empty() && counts_count && self.input.took_count() {
            self.changes.mark_count();
        }
        if let Some(shape) = self.editor.take_visual_shape() {
            self.changes.note_visual(shape);
        }
        // `:` commands and picker choices aren't repeated by `.`; a search
        // is, as an operator's motion (`d/foo<CR>`).
        let at_colon_prompt =
            self.editor.mode() == Mode::Command && !self.editor.command_line.prompt().is_search();
        if at_colon_prompt
            || matches!(
                self.editor.mode(),
                Mode::Picker | Mode::Find | Mode::Terminal | Mode::Popup | Mode::Input | Mode::Git
            )
        {
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
                self.editor.plugin_views_saw(event.kind());
                self.editor.git_saw(&event);
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
        self.editor.mode() == Mode::Normal && self.input.is_idle() && self.paste.is_none()
    }

    fn process_key(&mut self, key: KeyPress) {
        // A replayed paste: collect its text, then put it in.
        if let Some(text) = &mut self.paste {
            if key.code == KeyCode::PasteEnd {
                self.finish_open_paste();
            } else if let Some(c) = key.as_pasted() {
                text.push(c);
            }
            return;
        }
        match key.code {
            KeyCode::PasteStart => {
                self.input.reset();
                self.paste = Some(String::new());
                return;
            }
            KeyCode::PasteEnd => return,
            _ => {}
        }
        self.editor.note_key_for_completion();
        self.editor.note_key_for_hover();
        let mode = self.editor.mode();
        // A popup's own keys are in a copy of the keymap made for it.
        let keymap = match (&self.popup_keymap, mode) {
            (Some(overlay), Mode::Popup | Mode::Sidebar) => overlay,
            _ => &self.keymap,
        };
        let outcomes = self.input.handle(key, mode, keymap);
        self.apply_outcomes(outcomes);
        self.after_popup_key();
        self.editor.end_completion_unless_used();
        self.editor.end_hover_unless_used();
        self.editor.search_symbols_after_key();
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
                Outcome::PassThrough(keys) => self.editor.terminal_keys(&keys),
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
        self.finish_open_paste();
        self.replaying = false;
    }
}
