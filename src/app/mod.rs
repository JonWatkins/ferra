//! Application wiring: owns the editor, commands, keymap and UI state, and
//! runs the event loop.

mod repeat;
mod terminal;

use repeat::ChangeRecorder;

use crossterm::event::{self, Event};
use ratatui::{DefaultTerminal, Frame};

use crate::commands::{CommandRegistry, Invocation};
use crate::config::{Config, Theme};
use crate::editor::{Editor, Message, Mode};
use crate::input::{InputHandler, KeyPress, Keymap, Outcome};
use crate::ui::{self, ViewState};

pub struct App {
    editor: Editor,
    commands: CommandRegistry,
    keymap: Keymap,
    theme: Theme,
    input: InputHandler,
    view: ViewState,
    changes: ChangeRecorder,
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
        let mut app = Self {
            editor: Editor::new(config.editor),
            commands: CommandRegistry::with_builtins(),
            keymap: config.keys,
            theme: config.theme,
            input: InputHandler::default(),
            view: ViewState::default(),
            changes: ChangeRecorder::default(),
            replaying: false,
        };
        app.validate_keymap();
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

    /// Opens a file at startup, reporting any failure on the message line
    /// (the editor still starts, with an empty scratch document).
    pub fn open(&mut self, path: &std::path::Path) {
        if let Err(error) = self.editor.open(path) {
            self.editor.set_message(Message::error(error.to_string()));
        }
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

            if let Event::Key(event) = event::read()?
                && let Some(key) = KeyPress::from_event(event)
            {
                self.handle_key(key);
            }
        }
        terminal::restore_cursor_shape()
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let pending = self.input.pending_display();
        ui::draw(frame, &self.editor, &self.theme, &mut self.view, &pending);
    }

    /// Handles one key press. Public so tests can drive the app directly.
    pub fn handle_key(&mut self, key: KeyPress) {
        if self.replaying {
            self.process_key(key);
            return;
        }

        // Record each sequence from idle to idle, for `.`.
        if self.is_idle() {
            self.changes.begin(self.editor.document.edit_count());
        }
        self.changes.push(key);
        self.process_key(key);
        if self.editor.mode() == Mode::Command {
            self.changes.note_command_line();
        }
        if self.is_idle() {
            self.changes.finish(self.editor.document.edit_count());
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
        }
        if self.editor.mode() == Mode::OperatorPending {
            self.input.hold_last_sequence();
        }
        self.editor.finish_command();

        if let Some(request) = self.editor.take_repeat_request() {
            self.repeat_last_change(request.count);
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
