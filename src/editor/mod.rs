//! Editor state: the document, current mode, registers, command line and
//! messages.
//!
//! The editor is UI-agnostic. It's mutated by commands and read by the UI.

mod buffers;
mod command_line;
mod document;
mod files;
mod marks;
mod message;
mod mode;
mod operator;
pub mod picker;
pub mod registers;
mod search;
mod selection;
mod transform;
pub mod windows;

pub use buffers::BufferInfo;
pub use command_line::{CommandLine, Completion, CompletionContext, Prompt};
pub use document::{Document, DocumentId, OpenInfo, SaveInfo};
pub use files::UNSAVED_CHANGES;
pub use message::{Message, Severity};
pub use mode::Mode;
pub use operator::{Operator, PendingOperator};
pub use registers::{Register, Registers};
pub use selection::{Selection, SelectionKind};
pub use transform::CaseChange;
pub use windows::{Window, WindowId, Windows};

use crate::config::EditorOptions;
use crate::core::motion::CharSearch;
use crate::events::Event;

/// A request from `q`/`@` for the app, which records and replays keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroRequest {
    StartRecording(char),
    StopRecording,
    /// Play register `register` `count` times (`@` means the last played).
    Play {
        register: char,
        count: usize,
    },
}

/// A request from `.` to repeat the last change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepeatRequest {
    pub count: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Editor {
    /// The active buffer.
    pub document: Document,
    /// The other open buffers, least recently used first.
    background: Vec<Document>,
    /// The last id handed out; new documents get the next one.
    last_id: u64,
    pub command_line: CommandLine,
    pub options: EditorOptions,
    pub registers: Registers,
    /// The last `f`/`t`/`F`/`T`, for `;` and `,`.
    pub last_char_search: Option<CharSearch>,
    mode: Mode,
    pending_operator: Option<PendingOperator>,
    selection: Option<Selection>,
    search: search::SearchState,
    windows: Windows,
    /// The open picker popup, in picker mode.
    picker: Option<picker::Picker>,
    /// Set by `.`; the app replays the last change (which only it can do,
    /// since a change is recorded as keys).
    repeat_request: Option<RepeatRequest>,
    /// "After" events waiting for the app to dispatch to hooks.
    events: Vec<Event>,
    /// A command asking for the next key as its argument (see `request_char`).
    char_request: Option<String>,
    macro_request: Option<MacroRequest>,
    /// The register a macro is being recorded into.
    recording_macro: Option<char>,
    /// Which buffer each global mark (`A`-`Z`) is in.
    global_marks: std::collections::HashMap<char, DocumentId>,
    last_selection: Option<selection::LastSelection>,
    message: Option<Message>,
    should_quit: bool,
}

impl Editor {
    pub fn new(options: EditorOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    /// An editor with default options editing `document`.
    pub fn with_document(document: Document) -> Self {
        Self {
            document,
            ..Self::default()
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Switches mode, applying Vim's transition rules.
    pub fn set_mode(&mut self, mode: Mode) {
        let previous = std::mem::replace(&mut self.mode, mode);
        if previous != mode {
            self.emit(Event::ModeChanged {
                from: previous,
                to: mode,
            });
        }

        if previous == Mode::Insert && mode == Mode::Normal {
            // Like Vim, leaving insert mode steps back onto the last typed char.
            let cursor = self.document.cursor();
            self.document
                .set_cursor(cursor.with_col(cursor.col.saturating_sub(1)));
        }
        if previous == Mode::Command && mode != Mode::Command {
            self.end_search_prompt();
        }
        if previous == Mode::Command || mode == Mode::Command {
            self.command_line.clear();
        }
        if mode != Mode::OperatorPending {
            self.pending_operator = None;
        }
        if mode != Mode::Visual {
            self.end_selection();
        }
        if mode != Mode::Picker {
            self.picker = None;
        }
        self.enforce_cursor_rules();
    }

    /// The operator awaiting a motion, if in operator-pending mode.
    pub fn pending_operator(&self) -> Option<PendingOperator> {
        self.pending_operator
    }

    /// Normal mode may not rest on the end-of-line slot.
    pub fn enforce_cursor_rules(&mut self) {
        if matches!(
            self.mode,
            Mode::Normal | Mode::OperatorPending | Mode::Visual
        ) {
            self.document.clamp_to_last_char();
        }
    }

    /// Housekeeping after every command: enforces cursor rules and closes the
    /// undo step. An insert session stays open until insert mode is left, so
    /// that it (and the command that started it, like `o` or `cw`) undoes as
    /// one step.
    pub fn finish_command(&mut self) {
        self.enforce_cursor_rules();
        if self.mode != Mode::Insert {
            self.document.commit();
        }
    }

    /// Inserts typed text into whatever has focus in the current mode.
    pub fn insert_text_char(&mut self, c: char) {
        match self.mode {
            Mode::Insert => {
                self.dedent_for_closing_bracket(c);
                self.document.insert_char(c);
            }
            Mode::Command => {
                self.command_line.push(c);
                self.update_search_preview();
            }
            Mode::Picker => {
                if let Some(picker) = &mut self.picker {
                    picker.push(c);
                }
            }
            Mode::Normal | Mode::OperatorPending | Mode::Visual => {}
        }
    }

    pub fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    pub fn set_message(&mut self, message: Message) {
        self.message = Some(message);
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    /// Opens `picker` as a popup, in picker mode.
    pub fn open_picker(&mut self, picker: picker::Picker) {
        self.set_mode(Mode::Picker);
        self.picker = Some(picker);
    }

    pub fn picker(&self) -> Option<&picker::Picker> {
        self.picker.as_ref()
    }

    pub fn picker_mut(&mut self) -> Option<&mut picker::Picker> {
        self.picker.as_mut()
    }

    /// Closes the picker, returning to normal mode, and returns the action
    /// of the selected item, if any.
    pub fn accept_picker(&mut self) -> Option<picker::PickerAction> {
        let action = self
            .picker
            .as_ref()
            .and_then(|p| p.selected_item())
            .map(|item| item.action.clone());
        self.set_mode(Mode::Normal);
        action
    }

    /// Asks for the next key to be passed to `command` as its character
    /// argument. For commands that only sometimes need one (`q` takes a
    /// register to start recording, but not to stop); others use
    /// `Command::takes_char`.
    pub fn request_char(&mut self, command: &str) {
        self.char_request = Some(command.to_string());
    }

    pub fn take_char_request(&mut self) -> Option<String> {
        self.char_request.take()
    }

    pub fn request_macro(&mut self, request: MacroRequest) {
        self.macro_request = Some(request);
    }

    pub fn take_macro_request(&mut self) -> Option<MacroRequest> {
        self.macro_request.take()
    }

    pub fn recording_macro(&self) -> Option<char> {
        self.recording_macro
    }

    pub fn set_recording_macro(&mut self, register: Option<char>) {
        self.recording_macro = register;
    }

    /// Queues an event for hooks. The app dispatches queued events after each
    /// key, once the editor is in a consistent state.
    pub fn emit(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Asks the app to repeat the last change, with `count` replacing the
    /// original count if given.
    pub fn request_repeat(&mut self, count: Option<usize>) {
        self.repeat_request = Some(RepeatRequest { count });
    }

    pub fn take_repeat_request(&mut self) -> Option<RepeatRequest> {
        self.repeat_request.take()
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position;

    #[test]
    fn leaving_insert_mode_steps_cursor_back() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.insert_text_char('a');
        editor.insert_text_char('b');
        editor.set_mode(Mode::Normal);
        assert_eq!(editor.document.cursor(), Position::new(0, 1));
    }

    #[test]
    fn entering_and_leaving_command_mode_clears_input() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Command);
        editor.insert_text_char('q');
        assert_eq!(editor.command_line.input(), "q");
        editor.set_mode(Mode::Normal);
        assert!(editor.command_line.is_empty());
    }

    #[test]
    fn insert_session_is_one_undo_step() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.insert_text_char('a');
        editor.finish_command();
        editor.insert_text_char('b');
        editor.set_mode(Mode::Normal);
        editor.finish_command();
        editor.document.undo();
        assert_eq!(editor.document.buffer().text(), "");
    }

    #[test]
    fn mode_changes_are_emitted() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Normal);
        assert!(editor.take_events().is_empty(), "no change, no event");
        editor.set_mode(Mode::Insert);
        assert_eq!(
            editor.take_events(),
            vec![Event::ModeChanged {
                from: Mode::Normal,
                to: Mode::Insert
            }]
        );
        assert!(editor.take_events().is_empty());
    }

    #[test]
    fn typing_in_normal_mode_does_nothing() {
        let mut editor = Editor::default();
        editor.insert_text_char('x');
        assert_eq!(editor.document.buffer().text(), "");
    }
}
