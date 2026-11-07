//! Editor state: the document, current mode, registers, command line and
//! messages.
//!
//! The editor is UI-agnostic. It's mutated by commands and read by the UI.

mod buffers;
mod command_line;
mod document;
mod files;
mod jumps;
mod marks;
mod message;
mod mode;
mod operator;
pub mod picker;
pub mod registers;
mod search;
pub use search::{SearchDone, SearchHit};
mod block;
mod comment;
mod confirm;
mod selection;
mod transform;
pub mod windows;

pub use buffers::BufferInfo;
pub use command_line::{CommandLine, Completion, CompletionContext, Prompt};
pub use document::{DiskState, Document, DocumentId, OpenInfo, SaveInfo};
pub use files::UNSAVED_CHANGES;
pub use jumps::{Jump, JumpList};
pub use message::{Message, Severity};
pub use mode::Mode;
pub use operator::{Operator, PendingOperator};
pub use registers::{Register, Registers};
pub use selection::{Selection, SelectionKind};
pub use transform::CaseChange;
pub use windows::{Window, WindowId, Windows};

use crate::config::EditorOptions;
use crate::core::Position;
use crate::core::motion::CharSearch;
use crate::events::Event;

/// Keys for `:normal` to run, with the cursor at `at`. The app runs them
/// after the command (or after `:g`, which queues one run per line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalRun {
    pub keys: String,
    pub at: Position,
}

/// The file picker asking the app to list files in the background and add
/// them to the picker loading job `job`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListRequest {
    pub job: u64,
    pub root: std::path::PathBuf,
    /// The directory the picker was given, which opened files are under;
    /// `None` for the current directory (files open by their relative path).
    pub base: Option<std::path::PathBuf>,
    pub options: crate::files::WalkOptions,
}

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
    /// Which files are in which language (`[languages]` config).
    pub languages: crate::config::Languages,
    pub registers: Registers,
    /// The last `f`/`t`/`F`/`T`, for `;` and `,`.
    pub last_char_search: Option<CharSearch>,
    /// The arguments of the last `:s` (`/a/b/g`), for `&` and `:&&`.
    pub last_substitute: Option<String>,
    /// The last `:s` replacement, in `regex` syntax with any `~` already
    /// expanded: what `~` means next time, and what `&` repeats.
    pub last_replacement: Option<String>,
    /// A `:s///c` waiting for y/n/a/q/l answers.
    confirm: Option<confirm::ConfirmSession>,
    /// `:normal` runs waiting for the app.
    normal_queue: Vec<NormalRun>,
    /// A file listing waiting for the app to start.
    file_list_request: Option<FileListRequest>,
    /// The last background job number handed out.
    last_job: u64,
    /// The splash screen was asked for (started with no files) and hasn't
    /// been dismissed.
    splash: bool,
    /// Keeps the undo step open across commands (`:normal` over a range).
    undo_held: bool,
    /// What's been typed in the current insert session, for the `.`
    /// register.
    typed: String,
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
    jumps: JumpList,
    last_jump_origin: Option<Jump>,
    /// Which buffer each global mark (`A`-`Z`) is in.
    global_marks: std::collections::HashMap<char, DocumentId>,
    last_selection: Option<selection::LastSelection>,
    /// `$` in visual block mode: every row runs to its end.
    block_to_eol: bool,
    /// Text typed by `I`/`A`/`c` on a block's first row, copied to the
    /// others when insert mode ends.
    block_insert: Option<block::BlockInsert>,
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

        if previous != Mode::Insert && mode == Mode::Insert {
            self.typed.clear();
        }
        if previous == Mode::Insert && mode != Mode::Insert && !self.typed.is_empty() {
            self.registers
                .set_last_insert(std::mem::take(&mut self.typed));
        }
        if previous == Mode::Insert {
            // `I`/`A` on a block: copy what was typed to the other rows.
            let block_insert = self.block_insert.take();
            if mode == Mode::Normal
                && let Some(insert) = block_insert
            {
                self.finish_block_insert(insert);
            }
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
        // A `:s///c` session undoes as one step, across its answers.
        if self.mode != Mode::Insert && self.confirm.is_none() && !self.undo_held {
            self.document.commit();
        }
    }

    /// Inserts typed text into whatever has focus in the current mode.
    pub fn insert_text_char(&mut self, c: char) {
        match self.mode {
            Mode::Insert => {
                self.dedent_for_closing_bracket(c);
                self.document.insert_char(c);
                self.typed.push(c);
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

    /// Notes text entered in insert mode other than by typing characters
    /// (a line break, a tab, a paste), for the `.` register.
    pub fn note_typed(&mut self, text: &str) {
        if self.mode == Mode::Insert {
            self.typed.push_str(text);
        }
    }

    /// Backspace in insert mode: what was typed loses its last character.
    pub fn note_typed_backspace(&mut self) {
        if self.mode == Mode::Insert {
            self.typed.pop();
        }
    }

    /// The language `document` is in, if any (by name, extension or `#!`).
    pub fn language_of(&self, document: &Document) -> Option<&str> {
        self.languages
            .detect(document.path(), &document.buffer().line(0))
    }

    /// Shows the splash screen until the user starts working (see
    /// [`Editor::splash_visible`]).
    pub fn show_splash(&mut self) {
        self.splash = true;
    }

    /// Whether to draw the splash screen: while the editor still holds just
    /// the untouched starting buffer, in one window.
    pub fn splash_visible(&self) -> bool {
        self.splash && self.document.is_pristine_scratch() && self.windows.count() == 1
    }

    /// Dismisses the splash screen for good once it stops being visible
    /// (text typed, a file opened, a window split), so it doesn't come back
    /// when an empty buffer does.
    pub fn update_splash(&mut self) {
        if self.splash && !self.splash_visible() {
            self.splash = false;
        }
    }

    /// A number for a new background job.
    pub fn new_job_id(&mut self) -> u64 {
        self.last_job += 1;
        self.last_job
    }

    /// Asks the app to list files in the background for the file picker.
    pub fn request_file_list(&mut self, request: FileListRequest) {
        self.file_list_request = Some(request);
    }

    pub fn take_file_list_request(&mut self) -> Option<FileListRequest> {
        self.file_list_request.take()
    }

    /// Queues keys for `:normal` to run at `at`.
    pub fn queue_normal(&mut self, keys: &str, at: Position) {
        self.normal_queue.push(NormalRun {
            keys: keys.to_string(),
            at,
        });
    }

    pub fn take_normal_queue(&mut self) -> Vec<NormalRun> {
        std::mem::take(&mut self.normal_queue)
    }

    /// Holds the undo step open across commands, until released (which
    /// closes it).
    pub fn hold_undo(&mut self, held: bool) {
        self.undo_held = held;
        if !held {
            self.finish_command();
        }
    }

    /// Stops row tracking in every buffer (a `:normal` that switched
    /// buffers part-way).
    pub fn stop_tracking_rows_everywhere(&mut self) {
        self.document.stop_tracking_rows();
        for document in &mut self.background {
            document.stop_tracking_rows();
        }
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
