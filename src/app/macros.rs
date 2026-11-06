//! Recording and playing macros.
//!
//! While recording, every key the user presses is kept. Stopping stores the
//! keys in the register as key notation (`ihello<Esc>j`), the same notation
//! config uses, so a macro can be pasted, edited and yanked back like any
//! text. Playing parses the register and feeds the keys through the normal
//! input path, exactly as if typed.

use super::App;
use crate::editor::{MacroRequest, Message, Register};
use crate::input::{KeyPress, parse_keys};

/// How deeply macros may play other macros (including themselves), so a
/// macro that calls itself stops with an error instead of overflowing.
const MAX_DEPTH: usize = 100;

#[derive(Debug, Default)]
pub(super) struct MacroState {
    /// Keys pressed since recording started.
    keys: Vec<KeyPress>,
    /// How many macros are playing (nested `@` inside a macro).
    depth: usize,
    /// A command failed during playback: stop the rest, as Vim does.
    failed: bool,
    /// For `@@`.
    last_played: Option<char>,
}

impl MacroState {
    /// Starts playing keys that aren't typed (a macro, `:normal`): they
    /// aren't recorded, and a failing command stops them.
    pub(super) fn begin_playback(&mut self) {
        self.depth += 1;
    }

    pub(super) fn end_playback(&mut self) {
        self.depth -= 1;
        if self.depth == 0 {
            self.failed = false;
        }
    }

    /// A command failed during playback.
    pub(super) fn failed(&self) -> bool {
        self.failed
    }
}

impl App {
    /// Called for every key the user presses.
    pub(super) fn record_macro_key(&mut self, key: KeyPress) {
        if self.editor.recording_macro().is_some() && self.macros.depth == 0 {
            self.macros.keys.push(key);
        }
    }

    /// A command failed: during playback, the macro stops.
    pub(super) fn note_command_failure(&mut self) {
        if self.macros.depth > 0 {
            self.macros.failed = true;
        }
    }

    pub(super) fn handle_macro_request(&mut self, request: MacroRequest) {
        match request {
            MacroRequest::StartRecording(register) => {
                self.macros.keys.clear();
                self.editor.set_recording_macro(Some(register));
            }
            MacroRequest::StopRecording => self.stop_recording(),
            MacroRequest::Play { register, count } => self.play_macro(register, count),
        }
    }

    fn stop_recording(&mut self) {
        let Some(register) = self.editor.recording_macro() else {
            return;
        };
        self.editor.set_recording_macro(None);
        let mut keys = std::mem::take(&mut self.macros.keys);
        // The `q` that stopped recording isn't part of the macro.
        keys.pop();
        let text: String = keys.iter().map(ToString::to_string).collect();
        if let Err(error) = self
            .editor
            .registers
            .set(register, Register::charwise(text))
        {
            self.editor.set_message(Message::error(error));
        }
    }

    fn play_macro(&mut self, register: char, count: usize) {
        let register = match register {
            '@' => match self.macros.last_played {
                Some(last) => last,
                None => return self.fail("No previous macro"),
            },
            other => other,
        };
        let text = if register == ':' {
            match self.editor.command_line.last_command() {
                Some(command) => format!(":{}<CR>", command.replace('<', "<lt>")),
                None => return self.fail("No previous command line"),
            }
        } else {
            match self.editor.registers.read(Some(register)) {
                Ok(Some(contents)) => contents.text,
                Ok(None) => return self.fail(&format!("Register {register} is empty")),
                Err(error) => return self.fail(&error),
            }
        };
        let keys = match parse_keys(&text) {
            Ok(keys) => keys,
            Err(error) => return self.fail(&format!("Register {register}: {error}")),
        };
        self.macros.last_played = Some(register);
        if self.macros.depth >= MAX_DEPTH {
            return self.fail("Macro recursion too deep");
        }

        self.macros.depth += 1;
        'replay: for _ in 0..count.max(1) {
            for &key in &keys {
                self.handle_key(key);
                if self.macros.failed {
                    break 'replay;
                }
            }
        }
        self.macros.depth -= 1;
        if self.macros.depth == 0 {
            self.macros.failed = false;
        }
    }

    fn fail(&mut self, message: &str) {
        self.editor.set_message(Message::error(message));
        self.note_command_failure();
    }
}
