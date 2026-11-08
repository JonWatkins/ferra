//! Editor state: the document, current mode, registers, command line and
//! messages.
//!
//! The editor is UI-agnostic. It's mutated by commands and read by the UI.

mod buffers;
mod command_line;
mod completion;
mod document;
mod files;
mod increment;
mod jumps;
mod marks;
mod message;
mod mode;
mod operator;
pub mod picker;
pub mod registers;
mod replace;
mod search;
pub mod server_completion;
pub mod signature;
pub use search::{SearchDone, SearchHit};
mod block;
mod comment;
mod confirm;
pub mod diagnostics;
pub mod lsp;
mod selection;
mod transform;
pub mod windows;

pub use buffers::BufferInfo;
pub use command_line::{CommandLine, Completion, CompletionContext, Prompt};
pub use completion::WordCompletion;
pub use document::{DiskState, Document, DocumentId, OpenInfo, SaveInfo};
pub use files::UNSAVED_CHANGES;
pub use jumps::{Jump, JumpList};
pub use message::{Message, Severity};
pub use mode::Mode;
pub use operator::{Operator, PendingOperator};
pub use registers::{Register, Registers};
pub use selection::{Selection, SelectionKind, VisualShape};
pub use transform::CaseChange;
pub use windows::{Window, WindowId, Windows};

use std::collections::HashMap;

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
    /// The options in force for the active window and buffer: the global
    /// ones with any `:setlocal` values for them laid over (see
    /// [`Editor::refresh_options`]).
    pub options: EditorOptions,
    /// The options as set in config and by `:set`.
    global_options: EditorOptions,
    /// `:setlocal` values: each window's own (how it shows text) and each
    /// buffer's own (how it's edited).
    window_options: HashMap<WindowId, toml::Table>,
    buffer_options: HashMap<DocumentId, toml::Table>,
    /// The language whose options `options` includes (see
    /// [`Editor::refresh_options`]).
    options_language: Option<String>,
    /// Which files are in which language (`[languages]` config).
    pub languages: crate::config::Languages,
    /// Each document's syntax tree, kept up to date as it's edited, for
    /// indentation and highlighting.
    pub syntax_trees: crate::syntax::SyntaxTrees,
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
    /// The shape of the last selection an operator took, until the app
    /// collects it for `.` (see [`Editor::take_visual_shape`]).
    taken_shape: Option<selection::VisualShape>,
    /// The indentation a line had before typing a keyword moved it
    /// (`fi`), by document and row, to put back if the word goes on
    /// (`file`).
    keyword_indent: Option<(DocumentId, usize, String)>,
    /// The screen column `gj`/`gk` aim for, while the cursor stays where
    /// they left it (so a short row on the way doesn't lose it).
    screen_column: Option<(DocumentId, Position, usize)>,
    /// A count on the insert command (`3i`, `5o`): how many times to put
    /// in what's typed, and whether each copy is on a line of its own.
    insert_repeat: Option<(usize, bool)>,
    /// How each language server is doing, for `:lsp` (set by the app).
    lsp_status: Vec<String>,
    /// Requests for language servers, waiting for the app to send them.
    lsp_requests: Vec<lsp::LspRequest>,
    /// A language server's hover, shown by the cursor.
    hover: Option<lsp::Hover>,
    /// Whether the key being handled scrolled the hover (so it stays).
    hover_used: bool,
    /// The project symbol picker's query when the server was last asked.
    symbol_query: Option<String>,
    /// A language server's signature help, shown while typing a call.
    signature: Option<signature::SignatureHelp>,
    /// Added to the next save's message (see `note_for_save`).
    save_note: Option<String>,
    /// How `:format` reaches language servers (set by the app).
    formatter: Option<Box<dyn lsp::Formatter>>,
    /// The buffer and its edit count before the key being handled, to tell
    /// whether it changed the text (for completing as you type).
    text_before_key: (DocumentId, u64, u64),
    /// What each Lua `editor.status` function last returned, by name, for
    /// `plugin:<name>` status line elements (the app refreshes them before
    /// drawing).
    plugin_statuses: HashMap<String, String>,
    /// Replace mode (`R`) while in insert mode.
    replacing: Option<replace::Replacing>,
    /// Insert-mode word completion in progress (`<C-n>`).
    completion: Option<WordCompletion>,
    /// Whether the key being handled used the completion.
    completion_used: bool,
    /// Text typed by `I`/`A`/`c` on a block's first row, copied to the
    /// others when insert mode ends.
    block_insert: Option<block::BlockInsert>,
    message: Option<Message>,
    should_quit: bool,
}

impl Editor {
    pub fn new(options: EditorOptions) -> Self {
        Self {
            global_options: options.clone(),
            options,
            ..Self::default()
        }
    }

    /// The options as set in config and by `:set`, without any window's or
    /// buffer's own values.
    pub fn global_options(&self) -> &EditorOptions {
        &self.global_options
    }

    /// Replaces the global options (`:set`, `:setglobal`).
    pub fn set_global_options(&mut self, options: EditorOptions) {
        self.global_options = options;
        self.refresh_options();
    }

    /// Gives the active window (for a window option) or buffer (for a
    /// buffer option) its own `value` for option `name` (`:setlocal`).
    pub fn set_local_option(&mut self, name: &str, value: toml::Value) {
        let table = match crate::config::set::scope(name) {
            Some(crate::config::set::Scope::Window) => self
                .window_options
                .entry(self.windows.active())
                .or_default(),
            Some(crate::config::set::Scope::Buffer) => {
                self.buffer_options.entry(self.document.id()).or_default()
            }
            None => return,
        };
        table.insert(name.to_string(), value);
        self.refresh_options();
    }

    /// Sets option `name` to `value`: for the active window or buffer only
    /// with `local` (`:setlocal`), otherwise everywhere, dropping their own
    /// values (`:set`). For plugins; checked as config is.
    pub fn set_option(
        &mut self,
        name: &str,
        value: toml::Value,
        local: bool,
    ) -> Result<(), String> {
        use crate::config::set;
        if local {
            let mut options = self.options.clone();
            let name = set::set_value(&mut options, name, value)?;
            if set::scope(&name).is_none() {
                return Err(format!(
                    "{name} is the same everywhere: it can't be set locally"
                ));
            }
            let value = set::value_of(&options, &name).expect("just set");
            self.set_local_option(&name, value);
            return Ok(());
        }
        let mut global = self.global_options.clone();
        let name = set::set_value(&mut global, name, value)?;
        self.set_global_options(global);
        self.follow_global_options(std::slice::from_ref(&name));
        self.apply_option_effects(std::slice::from_ref(&name));
        Ok(())
    }

    /// The value of option `name` in force here.
    pub fn option_value(&self, name: &str) -> Option<toml::Value> {
        crate::config::set::get_value(&self.options, name)
    }

    /// After options `changed` changed: re-applies what was put in place at
    /// startup (the clipboard).
    pub fn apply_option_effects(&mut self, changed: &[String]) {
        let options = self.global_options.clone();
        let is = |name: &str| changed.iter().any(|c| c == name);
        if is("clipboard-provider") {
            let provider = crate::clipboard::for_provider(options.clipboard_provider);
            self.registers.set_clipboard(provider, options.clipboard);
        } else if is("clipboard") {
            self.registers.set_clipboard_mode(options.clipboard);
        }
    }

    /// Records how the language servers are doing, for `:lsp`.
    pub fn set_lsp_status(&mut self, lines: Vec<String>) {
        self.lsp_status = lines;
    }

    /// How the language servers are doing, one line each.
    pub fn lsp_status(&self) -> &[String] {
        &self.lsp_status
    }

    /// What plugin status function `name` last returned.
    pub fn plugin_status(&self, name: &str) -> Option<&str> {
        self.plugin_statuses.get(name).map(String::as_str)
    }

    /// Records what plugin status function `name` returned.
    pub fn set_plugin_status(&mut self, name: &str, text: String) {
        self.plugin_statuses.insert(name.to_string(), text);
    }

    /// After `:set` (or a plugin) changed the global `names`: drops the
    /// active window's and buffer's own values for them, so they follow the
    /// new global ones, except where the buffer's language sets one: that
    /// would otherwise still win, so the buffer gets the new value as its
    /// own, as `:set` does in Vim.
    pub fn follow_global_options(&mut self, names: &[String]) {
        self.clear_local_options(names);
        let language = self.language_of(&self.document).map(str::to_string);
        let Some(language_options) = language
            .as_deref()
            .and_then(|language| self.languages.options(language))
            .cloned()
        else {
            return;
        };
        for name in names
            .iter()
            .filter(|name| language_options.contains_key(*name))
        {
            if let Some(value) = crate::config::set::value_of(&self.global_options, name) {
                self.set_local_option(name, value);
            }
        }
    }

    /// Drops the active window's and buffer's own values for `names`, so
    /// they follow the global ones again (`:set`).
    pub fn clear_local_options(&mut self, names: &[String]) {
        for table in [
            self.window_options.get_mut(&self.windows.active()),
            self.buffer_options.get_mut(&self.document.id()),
        ]
        .into_iter()
        .flatten()
        {
            table.retain(|name, _| !names.iter().any(|n| n == name));
        }
        self.refresh_options();
    }

    /// `name=value` for each of the active window's and buffer's own
    /// option values (`:setlocal` alone).
    pub fn local_options(&self) -> Vec<String> {
        let mut shown = Vec::new();
        for table in [
            self.window_options.get(&self.windows.active()),
            self.buffer_options.get(&self.document.id()),
        ]
        .into_iter()
        .flatten()
        {
            shown.extend(crate::config::set::describe(table));
        }
        shown
    }

    /// The options in force for window `window` showing buffer `document`.
    pub fn options_for(&self, window: WindowId, document: DocumentId) -> EditorOptions {
        if window == self.windows.active() && document == self.document.id() {
            return self.options.clone();
        }
        let empty = toml::Table::new();
        let language = self
            .document_by_id(document)
            .and_then(|doc| self.language_of(doc))
            .and_then(|language| self.languages.options(language));
        crate::config::set::with_locals(
            &self.global_options,
            &[
                language.unwrap_or(&empty),
                self.buffer_options.get(&document).unwrap_or(&empty),
                self.window_options.get(&window).unwrap_or(&empty),
            ],
        )
    }

    /// Gives window `to` the same own option values as window `from`.
    pub(crate) fn copy_window_options(&mut self, from: WindowId, to: WindowId) {
        if let Some(table) = self.window_options.get(&from).cloned() {
            self.window_options.insert(to, table);
        }
    }

    /// Recomputes [`Editor::options`] for the active window and buffer;
    /// called when either changes.
    pub fn refresh_options(&mut self) {
        let empty = toml::Table::new();
        let language = self.language_of(&self.document).map(str::to_string);
        let language_options = language
            .as_deref()
            .and_then(|language| self.languages.options(language));
        self.options = crate::config::set::with_locals(
            &self.global_options,
            &[
                language_options.unwrap_or(&empty),
                self.buffer_options
                    .get(&self.document.id())
                    .unwrap_or(&empty),
                self.window_options
                    .get(&self.windows.active())
                    .unwrap_or(&empty),
            ],
        );
        self.options_language = language;
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
            self.insert_repeat = None;
        }
        if previous == Mode::Insert && mode != Mode::Insert {
            // `3R`: type the text twice more before it's stored.
            let leaving = std::mem::replace(&mut self.mode, previous);
            self.finish_replace();
            self.finish_insert_repeat();
            self.mode = leaving;
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
        if previous == Mode::Insert && mode != Mode::Insert {
            // `gi` (and `` `^ ``) go back to where insert mode stopped.
            self.document.set_mark('^');
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
            // A closed fold shows as its first line, so that's where the
            // cursor sits.
            let cursor = self.document.cursor();
            if let Some((first, _)) = self.document.closed_fold_at(cursor.row)
                && first != cursor.row
            {
                self.document.set_cursor(Position::new(first, cursor.col));
            }
            self.document.clamp_to_last_char();
        }
    }

    /// Opens any closed fold the cursor is in, for moves that should show
    /// where they land (a search, a mark, undo), as Vim's `foldopen` does.
    pub fn reveal_cursor(&mut self) {
        let row = self.document.cursor().row;
        while self.document.open_fold_at(row) {}
    }

    /// Housekeeping after every command: enforces cursor rules and closes the
    /// undo step. An insert session stays open until insert mode is left, so
    /// that it (and the command that started it, like `o` or `cw`) undoes as
    /// one step.
    pub fn finish_command(&mut self) {
        self.enforce_cursor_rules();
        // A `#!` line typed into a new file can change its language, and so
        // the options its language sets.
        if self.language_of(&self.document) != self.options_language.as_deref() {
            self.refresh_options();
        }
        // A `:s///c` session undoes as one step, across its answers.
        if self.mode != Mode::Insert && self.confirm.is_none() && !self.undo_held {
            self.document.commit();
        }
    }

    /// Inserts typed text into whatever has focus in the current mode.
    pub fn insert_text_char(&mut self, c: char) {
        match self.mode {
            Mode::Insert => {
                if self.replacing.is_some() {
                    self.type_replacing(c);
                } else {
                    self.type_with_indent(c);
                    self.auto_wrap(c);
                }
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

    /// Makes the insert just started put its text in `count` times when it
    /// ends (`3ihi<Esc>`), each copy on a line of its own with `lines`
    /// (`5o`).
    pub fn repeat_insert(&mut self, count: Option<usize>, lines: bool) {
        self.insert_repeat = count.filter(|&n| n > 1).map(|n| (n, lines));
    }

    /// Leaving insert mode after a counted insert: puts in what was typed
    /// the rest of the times. Copies on lines of their own (`o`, `O`) get
    /// the indentation of the line typed on.
    fn finish_insert_repeat(&mut self) {
        let Some((count, lines)) = self.insert_repeat.take() else {
            return;
        };
        let typed = self.typed.clone();
        if typed.is_empty() {
            return;
        }
        let doc = &mut self.document;
        for _ in 1..count {
            if lines {
                let row = doc.cursor().row;
                let line = doc.buffer().line(row);
                let indent: String = line
                    .chars()
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .collect();
                let end = doc.buffer().line_end(row);
                let after = doc.insert(end, &format!("\n{indent}{typed}"));
                doc.set_cursor(after);
            } else {
                doc.insert_str(&typed);
            }
        }
    }

    /// Completion replaced the last `removed` characters typed with `text`.
    pub(crate) fn note_typed_replacing(&mut self, removed: usize, text: &str) {
        if self.mode == Mode::Insert {
            for _ in 0..removed {
                self.typed.pop();
            }
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

    /// The screen column `gj`/`gk` last aimed for, if the cursor is still
    /// where they left it.
    pub fn wanted_screen_column(&self) -> Option<usize> {
        let (doc, at, column) = self.screen_column?;
        (doc == self.document.id() && at == self.document.cursor()).then_some(column)
    }

    /// Records the screen column `gj`/`gk` aimed for, with the cursor where
    /// they left it.
    pub fn set_wanted_screen_column(&mut self, column: usize) {
        self.screen_column = Some((self.document.id(), self.document.cursor(), column));
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
