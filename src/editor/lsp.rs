//! What the editor asks of language servers (hover, go to definition,
//! references) and shows from their answers. The editor only queues the
//! requests; the app's `Lsp` sends them and hands back the answers, so this
//! layer knows nothing of servers or JSON.

use std::path::{Path, PathBuf};

use super::VirtualText;
use super::picker::{Picker, PickerAction, PickerItem};
use super::{Document, DocumentId, Editor, Message, Mode};
use crate::core::Position;

/// Something a language server can be asked about the cursor's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspFeature {
    Hover,
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
    Completion,
    /// The rest of a completion item that was taken (its import).
    ResolveCompletion,
    Format,
    Rename,
    /// The fixes and changes the server offers for a range.
    CodeAction,
    /// Making the one chosen from the list (see `LspRequest::item`).
    ApplyCodeAction,
    /// The rest of a chosen action that came without its edits.
    ResolveCodeAction,
    SignatureHelp,
    /// The buffer's symbols (functions, types...).
    DocumentSymbols,
    /// The same, for the Outline view and the breadcrumbs (kept, not
    /// listed in a picker).
    Outline,
    /// The other uses of the symbol under the cursor, to highlight.
    DocumentHighlight,
    /// The project's symbols matching a query (`text`).
    WorkspaceSymbols,
    /// Types and parameter names to show in the text, for the buffer.
    InlayHint,
    /// The functions that call the one at the position (call hierarchy:
    /// `prepareCallHierarchy`, then `incomingCalls`).
    Callers,
    /// The functions it calls (`outgoingCalls`).
    Callees,
    /// The agent's `move_file`: the edits a move needs (imports), asked
    /// with `workspace/willRenameFiles`, then the move (`text`: where to).
    MoveFile,
}

impl LspFeature {
    /// What it finds, for messages (`No definition found`).
    pub fn noun(self) -> &'static str {
        match self {
            LspFeature::Hover => "hover information",
            LspFeature::Definition => "definition",
            LspFeature::Declaration => "declaration",
            LspFeature::TypeDefinition => "type definition",
            LspFeature::Implementation => "implementation",
            LspFeature::References => "references",
            LspFeature::Completion | LspFeature::ResolveCompletion => "completion",
            LspFeature::Format => "formatting",
            LspFeature::Rename => "renaming",
            LspFeature::CodeAction
            | LspFeature::ApplyCodeAction
            | LspFeature::ResolveCodeAction => "code actions",
            LspFeature::SignatureHelp => "signature help",
            LspFeature::DocumentSymbols | LspFeature::WorkspaceSymbols | LspFeature::Outline => {
                "symbols"
            }
            LspFeature::DocumentHighlight => "highlights",
            LspFeature::InlayHint => "inlay hints",
            LspFeature::Callers => "callers",
            LspFeature::Callees => "callees",
            LspFeature::MoveFile => "moving files",
        }
    }

    /// The picker's title when there are several.
    pub fn title(self) -> &'static str {
        match self {
            LspFeature::Hover => "Hover",
            LspFeature::Definition => "Definitions",
            LspFeature::Declaration => "Declarations",
            LspFeature::TypeDefinition => "Type definitions",
            LspFeature::Implementation => "Implementations",
            LspFeature::References => "References",
            LspFeature::Completion | LspFeature::ResolveCompletion => "Completions",
            LspFeature::Format => "Formatting",
            LspFeature::Rename => "Rename",
            LspFeature::CodeAction
            | LspFeature::ApplyCodeAction
            | LspFeature::ResolveCodeAction => "Code actions",
            LspFeature::SignatureHelp => "Signature",
            LspFeature::DocumentSymbols | LspFeature::Outline => "Symbols",
            LspFeature::WorkspaceSymbols => "Project symbols",
            LspFeature::DocumentHighlight => "Highlights",
            LspFeature::InlayHint => "Inlay hints",
            LspFeature::Callers => "Callers",
            LspFeature::Callees => "Calls",
            LspFeature::MoveFile => "Move",
        }
    }
}

/// An LSP query waiting to be sent: `query` at `position` in `document`,
/// as its text was at `edit_seq`, and who asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspRequest {
    pub query: LspQuery,
    pub document: DocumentId,
    pub position: Position,
    pub buffer_id: u64,
    pub edit_seq: u64,
    /// Who asked, and so where the answer goes.
    pub reply: Reply,
}

impl LspRequest {
    /// What's asked, without its details.
    pub fn feature(&self) -> LspFeature {
        self.query.feature()
    }

    /// Where the word being completed starts (the place, for others).
    pub fn word_start(&self) -> Position {
        match self.query {
            LspQuery::Completion { word_start, .. } => word_start,
            _ => self.position,
        }
    }
}

/// What a language server is asked, with what that question needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspQuery {
    Hover,
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
    /// Completing the word that starts at `word_start`, maybe on typing
    /// `trigger` (a `.`).
    Completion {
        trigger: Option<char>,
        word_start: Position,
    },
    /// A taken completion item's details (its import): which answer's,
    /// and which item (see `ServerItem::resolve`).
    ResolveCompletion {
        item: (u64, usize),
    },
    Format,
    Rename {
        name: String,
    },
    /// The code actions for `start..end`; with `file_wide`, only the
    /// file-wide ones (organize imports, fix all); `with_line`, those for
    /// its whole line too (with every problem on it: a fix can hang on a
    /// hint beside the error), after its own.
    CodeActions {
        start: Position,
        end: Position,
        file_wide: bool,
        with_line: bool,
    },
    /// Making a code action from a list a server gave.
    ApplyCodeAction {
        action: CodeAction,
    },
    /// A chosen code action's edit, from a server that fills it in then.
    ResolveCodeAction {
        action: CodeAction,
    },
    /// The signature being typed, maybe on typing `trigger` (a `(`);
    /// `retrigger` while it's showing.
    SignatureHelp {
        trigger: Option<char>,
        retrigger: bool,
    },
    DocumentSymbols,
    WorkspaceSymbols {
        query: String,
    },
    Outline,
    DocumentHighlight,
    InlayHint,
    /// The functions that call the one at the place; `found` once the
    /// server's found that function, for asking for its calls.
    Callers {
        found: bool,
    },
    /// The functions it calls (`found` as for `Callers`).
    Callees {
        found: bool,
    },
    /// Moving the document's file to `to`.
    MoveFile {
        to: PathBuf,
    },
}

impl LspQuery {
    /// What's asked, without its details.
    pub fn feature(&self) -> LspFeature {
        match self {
            LspQuery::Hover => LspFeature::Hover,
            LspQuery::Definition => LspFeature::Definition,
            LspQuery::Declaration => LspFeature::Declaration,
            LspQuery::TypeDefinition => LspFeature::TypeDefinition,
            LspQuery::Implementation => LspFeature::Implementation,
            LspQuery::References => LspFeature::References,
            LspQuery::Completion { .. } => LspFeature::Completion,
            LspQuery::ResolveCompletion { .. } => LspFeature::ResolveCompletion,
            LspQuery::Format => LspFeature::Format,
            LspQuery::Rename { .. } => LspFeature::Rename,
            LspQuery::CodeActions { .. } => LspFeature::CodeAction,
            LspQuery::ApplyCodeAction { .. } => LspFeature::ApplyCodeAction,
            LspQuery::ResolveCodeAction { .. } => LspFeature::ResolveCodeAction,
            LspQuery::SignatureHelp { .. } => LspFeature::SignatureHelp,
            LspQuery::DocumentSymbols => LspFeature::DocumentSymbols,
            LspQuery::WorkspaceSymbols { .. } => LspFeature::WorkspaceSymbols,
            LspQuery::Outline => LspFeature::Outline,
            LspQuery::DocumentHighlight => LspFeature::DocumentHighlight,
            LspQuery::InlayHint => LspFeature::InlayHint,
            LspQuery::Callers { .. } => LspFeature::Callers,
            LspQuery::Callees { .. } => LspFeature::Callees,
            LspQuery::MoveFile { .. } => LspFeature::MoveFile,
        }
    }

    /// The query for `feature` when a place is all it needs (hover,
    /// definitions, references...); `None` for those that need more.
    pub fn at(feature: LspFeature) -> Option<LspQuery> {
        Some(match feature {
            LspFeature::Hover => LspQuery::Hover,
            LspFeature::Definition => LspQuery::Definition,
            LspFeature::Declaration => LspQuery::Declaration,
            LspFeature::TypeDefinition => LspQuery::TypeDefinition,
            LspFeature::Implementation => LspQuery::Implementation,
            LspFeature::References => LspQuery::References,
            LspFeature::Format => LspQuery::Format,
            LspFeature::DocumentSymbols => LspQuery::DocumentSymbols,
            LspFeature::Outline => LspQuery::Outline,
            LspFeature::DocumentHighlight => LspQuery::DocumentHighlight,
            LspFeature::InlayHint => LspQuery::InlayHint,
            LspFeature::Callers => LspQuery::Callers { found: false },
            LspFeature::Callees => LspQuery::Callees { found: false },
            _ => return None,
        })
    }
}

/// Who asked an LSP query, and so where its answer goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// With a key or a command: says so when there's no answer.
    Key,
    /// The mouse (a hover, a Ctrl+hover peek): quiet without an answer.
    Pointer,
    /// As the user types (completion, signature help): quiet.
    Typing,
    /// To fill a view (the outline, inlay hints, highlights): quiet.
    View,
    /// The coding agent's tool call, by number: the answer goes back to
    /// it as text, not on the screen.
    Agent(u64),
}

impl Reply {
    /// The agent's tool call, if it asked.
    pub fn agent(self) -> Option<u64> {
        match self {
            Reply::Agent(call) => Some(call),
            _ => None,
        }
    }

    /// Asked for on purpose (a key, or the agent), so it's told when there
    /// is no answer, rather than left quiet.
    pub fn explicit(self) -> bool {
        matches!(self, Reply::Key | Reply::Agent(_))
    }
}

/// An LSP query's answer. Features move over to it one at a time; until
/// then their answers go straight from `lsp/` to the screen or the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The functions that call the one asked about (`Callers`), or that
    /// it calls (`Callees`).
    Calls(Calls),
    /// A hover's Markdown, as the server sent it (maybe with HTML; empty
    /// if it had nothing to say).
    Hover(String),
    /// Changes the server proposes: a rename's.
    Edits(Edits),
    /// The code actions there are (those that can be done there).
    Actions(Vec<CodeAction>),
    /// A code action made: its title, and its edits (none if all it does
    /// is run a command on the server, which sends edits of its own).
    Action { title: String, edits: Option<Edits> },
    /// Places: definitions, declarations, type definitions,
    /// implementations, references.
    Locations {
        found: Vec<Location>,
        /// What the server was busy with, if it was at work: it may know
        /// more in a moment (still indexing, say).
        busy: Option<Busy>,
    },
}

/// A code action a server offered: its title and kind, and the server's
/// own form of it, which only `lsp/` reads, to make it later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    /// `quickfix`, `refactor.extract`, `source.organizeImports`... (maybe
    /// empty).
    pub kind: String,
    pub server: serde_json::Value,
}

/// A list of code actions kept to make one from: its number, and the
/// document and text (edit count) the actions were made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionList {
    pub number: u64,
    pub actions: Vec<CodeAction>,
    pub document: DocumentId,
    pub edit_seq: u64,
}

/// One document's changes: ranges and their new text (see
/// [`Document::apply_edits`]).
pub type TextEdits = Vec<(Position, Position, String)>;

/// Changes to documents a language server proposed, in the editor's own
/// terms, for the asker to make.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edits {
    /// Each document's changes.
    pub documents: Vec<(DocumentId, TextEdits)>,
    /// The buffers opened in the background to hold them.
    pub opened: Vec<DocumentId>,
    /// File operations (creating, renaming, deleting) left out.
    pub skipped: usize,
}

/// What applying a workspace edit did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub changes: usize,
    pub files: usize,
    /// Files changed that aren't the buffer being edited: left unsaved.
    pub others: usize,
    /// Creating, renaming or deleting files, which ferra doesn't do.
    pub skipped: usize,
}

impl Applied {
    /// `3 changes in 2 files (1 other file not saved yet: :wa)`.
    pub fn describe(&self) -> String {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut text = format!(
            "{} in {}",
            plural(self.changes, "change", "changes"),
            plural(self.files, "file", "files")
        );
        if self.others > 0 {
            text.push_str(&format!(
                " ({} not saved yet: :wa)",
                plural(self.others, "other file", "other files")
            ));
        }
        if self.skipped > 0 {
            text.push_str(&format!(
                "; {} left out",
                plural(self.skipped, "file operation", "file operations")
            ));
        }
        text
    }
}

/// A server at work, by name, and what it's doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    pub server: String,
    pub work: String,
}

/// A call hierarchy's functions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calls {
    /// Those in the project, as the server listed them.
    pub ours: Vec<Call>,
    /// The names of those outside it (the standard library, dependencies),
    /// once each.
    pub outside: Vec<String>,
}

/// A function in a call hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    /// What it is: "function", "method"...
    pub kind: &'static str,
    /// What the server adds (often its signature).
    pub detail: Option<String>,
    pub path: PathBuf,
    /// Where it's named, from 1.
    pub line: usize,
    /// The lines the calls are on, from 1: in the caller's file for
    /// callers, in the function asked about for callees.
    pub at: Vec<usize>,
}

/// Why an LSP query has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoAnswer {
    /// No language server has the document.
    NoServer,
    /// Its server (by name) is still starting.
    Starting(String),
    /// Its server (by name) doesn't offer what was asked.
    Unsupported(String),
    /// Its server (by name) stopped before answering.
    Stopped(String),
    /// Its server answered with an error: `stale` if it's that the text
    /// changed while it worked (so the question was out of date).
    ServerError {
        server: String,
        message: String,
        stale: bool,
    },
    /// Nothing at the place asked about to answer for.
    NothingThere,
    /// The server's answer couldn't be used (edits for a file that can't
    /// be opened, say): why.
    Unusable(String),
}

/// A place a server pointed to: a file, and a row and char column in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub position: Position,
    /// The line's text, shown in the picker.
    pub line: String,
}

/// A hover popup: the server's Markdown, in a document of its own so it
/// can be highlighted like any other (code blocks in their language).
#[derive(Debug, Clone)]
pub struct Hover {
    pub document: Document,
    /// The first of its rows shown (`K` again scrolls).
    pub scroll: usize,
    /// How many rows were shown last time it was drawn, and how many there
    /// are (wrapped, without code fences), so `K` can scroll by a page.
    pub shown: std::cell::Cell<usize>,
    pub total: std::cell::Cell<usize>,
    /// Where the pointer rested, for a hover it showed (else it's by the
    /// cursor): the screen cell.
    pub anchor: Option<(u16, u16)>,
}

/// The HTML tags docs use (VS Code draws them): taken out of a hover, as
/// a terminal can't, keeping their text. Others (`Vec<T>`) are text.
const HTML_TAGS: &[&str] = &[
    "a", "b", "br", "code", "del", "details", "div", "em", "font", "h1", "h2", "h3", "h4", "h5",
    "h6", "hr", "i", "img", "kbd", "li", "ol", "p", "pre", "s", "small", "span", "strong", "sub",
    "summary", "sup", "table", "tbody", "td", "th", "thead", "tr", "u", "ul",
];

/// `markdown` without HTML tags and with its entities (`&quot;`) made
/// chars, outside code blocks (where they're the code).
pub fn without_html(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_code = false;
    for (i, line) in markdown.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push_str(line);
            continue;
        }
        if in_code {
            out.push_str(line);
            continue;
        }
        out.push_str(&decode_entities(&strip_tags(line)));
    }
    out
}

fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name: String = after
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let close = after.find('>');
        match close {
            Some(close) if HTML_TAGS.contains(&name.as_str()) => {
                rest = &after[close + 1..];
            }
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entities(line: &str) -> String {
    if !line.contains('&') {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let entity = after
            .find(';')
            .filter(|&end| end <= 8)
            .map(|end| &after[..end]);
        let decoded = entity.and_then(|e| match e {
            "quot" => Some('"'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => e
                .strip_prefix("#x")
                .or_else(|| e.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| e.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        });
        match (entity, decoded) {
            (Some(entity), Some(c)) => {
                out.push(c);
                rest = &after[entity.len() + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Formats a buffer by asking its language server, waiting for the answer
/// (so `:wq` saves the formatted text). The app gives the editor one.
pub trait Formatter: std::fmt::Debug {
    /// Formats the active buffer. With `quiet` (formatting on save), not
    /// having a server that formats it is fine.
    fn format(&mut self, editor: &mut Editor, quiet: bool) -> Result<(), String>;
}

/// The uses of the symbol under the cursor, as the server found them for
/// a buffer's text at `edit_seq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct References {
    pub document: DocumentId,
    pub edit_seq: u64,
    pub ranges: Vec<(Position, Position)>,
}

/// The id hover documents have: never one of the editor's buffers.
pub const HOVER_DOCUMENT: DocumentId = DocumentId(u64::MAX);

impl Editor {
    /// Keeps the uses of the symbol under the cursor, found for `document`'s
    /// text at `edit_seq`.
    pub fn set_references(
        &mut self,
        document: DocumentId,
        edit_seq: u64,
        ranges: Vec<(Position, Position)>,
    ) {
        self.references = Some(References {
            document,
            edit_seq,
            ranges,
        });
    }

    /// The uses of the symbol under the cursor to highlight: while the text
    /// is as it was when they were found, in normal mode, and the cursor is
    /// on one of them (moving off hides them until the server's asked about
    /// the new place).
    pub fn references_shown(&self) -> &[(Position, Position)] {
        let Some(references) = &self.references else {
            return &[];
        };
        let cursor = self.document.cursor();
        let current = references.document == self.document.id()
            && references.edit_seq == self.document.buffer().edit_seq()
            && self.resting()
            && references
                .ranges
                .iter()
                .any(|(start, end)| *start <= cursor && cursor < *end);
        if current && self.options.highlight_references {
            &references.ranges
        } else {
            &[]
        }
    }

    /// Lets `:format` and `format-on-save` ask language servers.
    pub fn set_formatter(&mut self, formatter: Box<dyn Formatter>) {
        self.formatter = Some(formatter);
    }

    /// `:format`, or before saving with `format-on-save` (`quiet`): has
    /// the buffer's language server format it.
    pub fn format(&mut self, quiet: bool) -> Result<(), String> {
        if !self.options.lsp {
            return if quiet {
                Ok(())
            } else {
                Err("Language servers are off (lsp = false)".into())
            };
        }
        let Some(mut formatter) = self.formatter.take() else {
            return if quiet {
                Ok(())
            } else {
                Err("No language server for this buffer (see :lsp)".into())
            };
        };
        let result = formatter.format(self, quiet);
        self.formatter = Some(formatter);
        result
    }

    /// Replaces each `start..end` with its text (a server's edits: they
    /// don't overlap, and are all about the text as it is now), keeping the
    /// cursor on the text it was on: moved along by edits before it, and
    /// kept at its line and column (as far as it can be) in one that
    /// replaces the text it was in.
    pub fn apply_text_edits(&mut self, edits: Vec<(Position, Position, String)>) {
        self.document.apply_edits(edits);
        // In insert mode the cursor can be after the last char (after a
        // word just completed).
        if self.mode() != Mode::Insert {
            self.document.clamp_to_last_char();
        }
    }

    /// Buffer `id`, to change.
    pub fn document_by_id_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        if self.document.id() == id {
            return Some(&mut self.document);
        }
        self.background.iter_mut().find(|d| d.id() == id)
    }

    /// The buffer with `path` open, opening it in the background (not
    /// shown, no jump) if it isn't: for edits to files that aren't open (a
    /// rename's).
    pub fn buffer_for_path(&mut self, path: &Path) -> Result<DocumentId, String> {
        if let Some(id) = self.find_open_path(path) {
            return Ok(id);
        }
        let fallback = self.options.fallback_encoding.0;
        let (mut document, info) =
            Document::open_with(path, fallback).map_err(|e| e.to_string())?;
        let id = self.new_document_id();
        document.set_id(id);
        self.background.push(document);
        self.emit(crate::events::Event::Opened { path: info.path });
        Ok(id)
    }

    /// Makes `edits` (see [`Document::apply_edits`]) to buffer `id`, as one
    /// undo step of their own. Returns whether there was such a buffer.
    pub fn edit_document(
        &mut self,
        id: DocumentId,
        edits: Vec<(Position, Position, String)>,
    ) -> bool {
        let active = id == self.document.id();
        let Some(document) = self.document_by_id_mut(id) else {
            return false;
        };
        document.commit();
        document.apply_edits(edits);
        document.commit();
        if active && self.mode() != Mode::Insert {
            self.document.clamp_to_last_char();
        }
        true
    }

    /// Asks the language server for `feature` at the cursor (`K`, `gd`...),
    /// for one that needs no more than the place.
    pub fn request_lsp(&mut self, feature: LspFeature) -> Result<(), String> {
        let query = LspQuery::at(feature)
            .ok_or_else(|| format!("Language servers can't be asked for {}", feature.noun()))?;
        self.ask_lsp(query)
    }

    /// Asks the language server `query` at the cursor, for the user.
    pub fn ask_lsp(&mut self, query: LspQuery) -> Result<(), String> {
        if !self.options.lsp {
            return Err("Language servers are off (lsp = false)".into());
        }
        if self.document.path().is_none() {
            return Err("No language server: the buffer has no file".into());
        }
        let buffer = self.document.buffer();
        self.lsp_requests.push(LspRequest {
            query,
            document: self.document.id(),
            position: self.document.cursor(),
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            reply: Reply::Key,
        });
        Ok(())
    }

    /// A server's inlay hints for the buffer `request` was about, if its
    /// text hasn't changed since (else new ones are on their way).
    pub fn set_inlay_hints(&mut self, request: &LspRequest, hints: Vec<VirtualText>) {
        let Some(document) = self.document_by_id_mut(request.document) else {
            return;
        };
        if document.buffer().edit_seq() == request.edit_seq {
            document.set_virtual_text("inlay", hints);
        }
    }

    /// `inlay-hints = false` (or `:inlay_hints`): takes them away.
    pub fn clear_inlay_hints(&mut self) {
        let ids: Vec<DocumentId> = self.buffers().iter().map(|b| b.id).collect();
        for id in ids {
            if let Some(document) = self.document_by_id_mut(id) {
                document.clear_virtual_text("inlay");
            }
        }
    }

    /// `<Space>a`: asks the language server what it can do here: at the
    /// cursor, or with the visual selection (which ends).
    pub fn request_code_actions(&mut self) -> Result<(), String> {
        let range = self.take_selection();
        let cursor = self.document.cursor();
        let (start, end) = match range {
            Some(crate::core::Range::Chars { start, end }) => (start, end),
            Some(crate::core::Range::Lines { first, last }) => (
                Position::new(first, 0),
                self.document.buffer().line_end(last),
            ),
            None => (cursor, cursor),
        };
        self.ask_lsp(LspQuery::CodeActions {
            start,
            end,
            file_wide: false,
            with_line: false,
        })
    }

    /// Lists a server's code actions (their titles and kinds), to choose
    /// one; kept as the screen's list of them.
    fn show_code_actions(&mut self, request: &LspRequest, actions: Vec<CodeAction>) {
        if actions.is_empty() {
            self.set_message(Message::info("No code actions here"));
            return;
        }
        let list = self.keep_code_actions(request, actions);
        let items = list
            .actions
            .iter()
            .enumerate()
            .map(|(i, action)| PickerItem {
                label: action.title.clone(),
                detail: action.kind.clone(),
                action: PickerAction::CodeAction(list.number, i),
            })
            .collect();
        self.code_actions = Some(list);
        self.open_picker(Picker::new("Code actions", items).keep_label_start());
    }

    /// `actions`, the answer to `request`, as a numbered list to make one
    /// from.
    pub(super) fn keep_code_actions(
        &mut self,
        request: &LspRequest,
        actions: Vec<CodeAction>,
    ) -> ActionList {
        self.action_lists_made += 1;
        ActionList {
            number: self.action_lists_made,
            actions,
            document: request.document,
            edit_seq: request.edit_seq,
        }
    }

    /// Asks for code action `index` of `list` to be made, if the text is
    /// still what the list was made for; `reply` says who for.
    pub(super) fn make_code_action(
        &mut self,
        list: Option<&ActionList>,
        index: usize,
        reply: Reply,
    ) -> Result<(), &'static str> {
        const TOO_OLD: &str = "That list of code actions is too old (or has no such number)";
        let list = list.ok_or(TOO_OLD)?;
        let action = list.actions.get(index).ok_or(TOO_OLD)?.clone();
        let document = self
            .document_by_id(list.document)
            .filter(|d| d.buffer().edit_seq() == list.edit_seq)
            .ok_or("The file has changed since that list of code actions: ask for them again")?;
        let buffer = document.buffer();
        self.lsp_requests.push(LspRequest {
            query: LspQuery::ApplyCodeAction { action },
            document: list.document,
            position: Position::default(),
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            reply,
        });
        Ok(())
    }

    /// `<Space>S`: opens a picker of the project's symbols, which the
    /// language server searches for as the query is typed.
    pub fn request_workspace_symbols(&mut self) -> Result<(), String> {
        self.ask_lsp(LspQuery::WorkspaceSymbols {
            query: String::new(),
        })?;
        self.symbol_query = Some(String::new());
        self.open_picker(
            Picker::new(LspFeature::WorkspaceSymbols.title(), Vec::new()).searched_by_server(),
        );
        Ok(())
    }

    /// After each key: if the project symbol picker's query changed, asks
    /// the server again.
    pub fn search_symbols_after_key(&mut self) {
        let query = self
            .picker
            .as_ref()
            .filter(|p| p.is_searched_by_server() && self.mode() == Mode::Picker)
            .map(|p| p.query().to_string());
        let Some(query) = query else {
            self.symbol_query = None;
            return;
        };
        if self.symbol_query.as_deref() == Some(query.as_str()) {
            return;
        }
        self.symbol_query = Some(query.clone());
        // The picker's buffer is the one it was opened from.
        let _ = self.ask_lsp(LspQuery::WorkspaceSymbols { query });
    }

    /// Lists `symbols` (a label, what it is, where) in a picker, or for the
    /// project's, replaces what the open picker has if `query` is still
    /// what's typed in it.
    pub fn show_symbols(
        &mut self,
        feature: LspFeature,
        query: Option<&str>,
        symbols: Vec<(String, String, Location)>,
    ) {
        let items: Vec<PickerItem> = symbols
            .into_iter()
            .map(|(label, detail, location)| PickerItem {
                label,
                detail,
                action: PickerAction::GoTo(location.path, location.position),
            })
            .collect();
        if feature == LspFeature::WorkspaceSymbols {
            let picker = self
                .picker
                .as_mut()
                .filter(|p| p.is_searched_by_server() && Some(p.query()) == query);
            if let Some(picker) = picker {
                picker.replace_items(items);
            }
            return;
        }
        if items.is_empty() {
            self.set_message(Message::info("No symbols in this buffer"));
            return;
        }
        self.open_picker(Picker::new(feature.title(), items).keep_label_start());
    }

    /// Makes the code action chosen from the list.
    pub fn apply_code_action(&mut self, list: u64, index: usize) {
        let kept = self.code_actions.take().filter(|l| l.number == list);
        if let Err(why) = self.make_code_action(kept.as_ref(), index, Reply::Key) {
            self.set_message(Message::info(why));
        }
        self.code_actions = kept;
    }

    /// `:rename name`: has the language server rename the symbol under
    /// the cursor to `name`, everywhere it's used.
    pub fn request_rename(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Rename to what? :rename new_name".into());
        }
        self.ask_lsp(LspQuery::Rename {
            name: name.to_string(),
        })
    }

    /// The requests waiting to be sent (to adjust the one just made).
    pub fn lsp_requests_mut(&mut self) -> &mut Vec<LspRequest> {
        &mut self.lsp_requests
    }

    /// The requests made since this was last called, oldest first.
    pub fn take_lsp_requests(&mut self) -> Vec<LspRequest> {
        std::mem::take(&mut self.lsp_requests)
    }

    /// Whether the editor is still where `request` was made (same buffer,
    /// text and cursor), so its answer still applies.
    pub fn still_at(&self, request: &LspRequest) -> bool {
        let buffer = self.document.buffer();
        self.document.id() == request.document
            && buffer.id() == request.buffer_id
            && buffer.edit_seq() == request.edit_seq
            && self.document.cursor() == request.position
            && self.resting()
    }

    /// Shows `markdown` in a popup by the cursor, until the next key.
    pub fn show_hover(&mut self, markdown: &str) {
        let markdown = without_html(markdown);
        let mut document = Document::with_text(markdown.trim());
        document.set_id(HOVER_DOCUMENT);
        let lines = document.buffer().line_count();
        self.hover = Some(Hover {
            document,
            scroll: 0,
            shown: std::cell::Cell::new(lines),
            total: std::cell::Cell::new(lines),
            anchor: None,
        });
        // Shown as it arrives, between keys: the next key is the first that
        // could close it.
        self.hover_used = true;
    }

    pub fn hover(&self) -> Option<&Hover> {
        self.hover.as_ref()
    }

    /// `K` with a hover open: scrolls it a page, or closes it at the end.
    /// Returns whether there was one.
    pub fn scroll_hover(&mut self) -> bool {
        let Some(hover) = &mut self.hover else {
            return false;
        };
        let page = hover.shown.get().max(1);
        let total = hover.total.get();
        if hover.scroll + page >= total {
            self.hover = None;
        } else {
            // The last page ends at the bottom, rather than half empty.
            hover.scroll = (hover.scroll + page).min(total - page);
            self.hover_used = true;
        }
        true
    }

    /// Called before each key.
    pub fn note_key_for_hover(&mut self) {
        self.hover_used = false;
    }

    /// Called after each key: one that didn't scroll the hover closes it.
    pub fn end_hover_unless_used(&mut self) {
        if !self.hover_used {
            self.hover = None;
            self.mouse_hover = None;
        }
    }

    /// Goes to `location` (opening its file if need be), as a jump.
    pub fn goto_location(&mut self, location: &Location) -> Result<(), String> {
        let here = self
            .document
            .path()
            .is_some_and(|p| same_file(p, &location.path));
        if here {
            self.push_jump();
        } else {
            // Named as the user would (`src/shapes.rs`) when it's under
            // the current folder, as servers give full paths.
            let path = std::env::current_dir()
                .ok()
                .and_then(|dir| std::fs::canonicalize(dir).ok())
                .and_then(|dir| location.path.strip_prefix(dir).ok().map(Path::to_path_buf))
                .unwrap_or_else(|| location.path.clone());
            self.open(&path).map_err(|e| e.to_string())?;
        }
        self.document.set_cursor(location.position);
        self.document.clamp_to_last_char();
        self.reveal_cursor();
        Ok(())
    }

    /// Shows what a server found for `feature`: goes straight there if it's
    /// one place, else lists them in a picker.
    pub fn show_locations(
        &mut self,
        feature: LspFeature,
        locations: Vec<Location>,
    ) -> Result<(), String> {
        match locations.as_slice() {
            [] => Err(format!("No {} found", feature.noun())),
            [one] => self.goto_location(one),
            _ => {
                let here = std::env::current_dir().ok();
                let items = locations
                    .into_iter()
                    .map(|location| {
                        let shown = here
                            .as_deref()
                            .and_then(|dir| location.path.strip_prefix(dir).ok())
                            .unwrap_or(&location.path);
                        PickerItem {
                            label: format!(
                                "{}:{}:{}",
                                shown.display(),
                                location.position.row + 1,
                                location.position.col + 1
                            ),
                            detail: location.line.trim().to_string(),
                            action: PickerAction::GoTo(location.path, location.position),
                        }
                    })
                    .collect();
                self.open_picker(Picker::new(feature.title(), items));
                Ok(())
            }
        }
    }

    /// Reports a failed or empty answer.
    pub fn lsp_failed(&mut self, text: String) {
        self.set_message(Message::error(text));
    }

    /// An LSP query's answer, or why there's none: to the agent if it
    /// asked, else shown (or not) as the reply says.
    pub fn lsp_answered(&mut self, request: &LspRequest, answer: Result<Answer, NoAnswer>) {
        match request.reply {
            Reply::Agent(call) => self.agent_lsp_answered(call, request, answer),
            _ => self.show_lsp_answer(request, answer),
        }
    }

    /// Makes a language server's edits, each document's as one undo step.
    pub fn apply_lsp_edits(&mut self, edits: Edits) -> Applied {
        let active = self.document.id();
        let mut applied = Applied {
            skipped: edits.skipped,
            ..Applied::default()
        };
        for (id, changes) in edits.documents {
            applied.changes += changes.len();
            applied.files += 1;
            if id != active {
                applied.others += 1;
            }
            self.edit_document(id, changes);
        }
        applied
    }

    /// The screen's side of `lsp_answered`: shown if it's still wanted
    /// (the cursor, or the pointer, still where it was asked).
    fn show_lsp_answer(&mut self, request: &LspRequest, answer: Result<Answer, NoAnswer>) {
        let pointer = request.reply == Reply::Pointer;
        let wanted = |e: &Self| match request.feature() {
            // Picked from a list: made unless the text changed meanwhile.
            LspFeature::ApplyCodeAction | LspFeature::ResolveCodeAction => {
                e.document.id() == request.document
                    && e.document.buffer().edit_seq() == request.edit_seq
            }
            _ if pointer => e.mouse_hover_wanted(request),
            _ => e.still_at(request),
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(why) => {
                if wanted(self) {
                    self.show_no_answer(request, why);
                }
                return;
            }
        };
        match answer {
            // Only the agent asks for calls.
            Answer::Calls(_) => {}
            Answer::Hover(_)
            | Answer::Locations { .. }
            | Answer::Edits(_)
            | Answer::Actions(_)
            | Answer::Action { .. }
                if !wanted(self) => {}
            Answer::Actions(actions) => self.show_code_actions(request, actions),
            Answer::Action { title, edits } => {
                if let Some(edits) = edits {
                    let applied = self.apply_lsp_edits(edits);
                    self.set_message(Message::info(format!("{title}: {}", applied.describe())));
                }
            }
            Answer::Edits(edits) => {
                let name = match &request.query {
                    LspQuery::Rename { name } => name.as_str(),
                    _ => "",
                };
                let text = match self.apply_lsp_edits(edits) {
                    Applied { changes: 0, .. } => "Nothing to rename here".to_string(),
                    done => format!("Renamed to {name}: {}", done.describe()),
                };
                self.set_message(Message::info(text));
            }
            Answer::Hover(markdown) if pointer => self.mouse_hover_answer(&markdown),
            Answer::Hover(markdown) if markdown.trim().is_empty() => {
                self.set_message(Message::info("No hover information here"));
            }
            Answer::Hover(markdown) => self.show_hover(&markdown),
            // Ctrl held on the pointer's word: the definition peeked at.
            Answer::Locations { found, .. } if pointer => {
                if request.feature() == LspFeature::Definition {
                    self.mouse_peek_answer(&found);
                }
            }
            Answer::Locations { found, .. } => {
                if let Err(error) = self.show_locations(request.feature(), found) {
                    self.set_message(Message::info(error));
                }
            }
        }
    }

    /// Why there's no answer, said if it was asked for with a key.
    fn show_no_answer(&mut self, request: &LspRequest, why: NoAnswer) {
        if request.reply != Reply::Key {
            return;
        }
        let text = match why {
            // Asked again as the text changes: not worth saying.
            NoAnswer::ServerError { stale: true, .. } => return,
            NoAnswer::ServerError {
                server, message, ..
            } => {
                self.lsp_failed(format!("{server}: {message}"));
                return;
            }
            NoAnswer::NothingThere => format!("No {} here", request.feature().noun()),
            NoAnswer::Unusable(why) => {
                self.lsp_failed(why);
                return;
            }
            NoAnswer::NoServer => "No language server for this buffer (see :lsp)".to_string(),
            NoAnswer::Starting(name) => format!("{name} is still starting"),
            NoAnswer::Unsupported(name) => {
                format!("{name} doesn't offer {}", request.feature().noun())
            }
            // Its stopping is said already.
            NoAnswer::Stopped(_) => return,
        };
        self.set_message(Message::info(text));
    }
}

impl Document {
    /// Replaces each `start..end` with its text (a server's edits: they
    /// don't overlap, and are all about the text as it is now), keeping the
    /// cursor on the text it was on: moved along by edits before it, and
    /// kept at its line and column (as far as it can be) in one that
    /// replaces the text it was in.
    pub fn apply_edits(&mut self, mut edits: Vec<(Position, Position, String)>) {
        // Last first, so each edit's place is still where it was.
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let mut cursor = self.cursor();
        for (start, end, text) in edits {
            self.delete(start, end);
            let new_end = self.insert(start, &text);
            if end <= cursor {
                cursor = moved(cursor, end, new_end);
            } else if start < cursor {
                cursor = cursor.min(new_end);
            }
        }
        self.set_cursor(cursor);
    }
}

/// Where `cursor` (after `end`) is once the text up to `end` is replaced
/// by text ending at `new_end`.
fn moved(cursor: Position, end: Position, new_end: Position) -> Position {
    if end.row == cursor.row {
        Position::new(new_end.row, new_end.col + (cursor.col - end.col))
    } else {
        let rows = new_end.row as isize - end.row as isize;
        Position::new((cursor.row as isize + rows) as usize, cursor.col)
    }
}

/// Whether `a` and `b` are the same file, however they're written.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || canonical(a) == canonical(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hovers_lose_html_but_not_generics_or_code() {
        let shown = super::without_html(
            "<span style=\"color: #F15D22\"><b>Finished</b></span> &quot;x&quot; &amp; Vec<T> &#39;\n```html\n<b>kept</b>\n```",
        );
        assert_eq!(
            shown,
            "Finished \"x\" & Vec<T> '\n```html\n<b>kept</b>\n```"
        );
    }
    use super::*;

    fn location(path: &Path, row: usize, col: usize) -> Location {
        Location {
            path: path.to_path_buf(),
            position: Position::new(row, col),
            line: "  some line ".into(),
        }
    }

    #[test]
    fn one_location_is_gone_to_and_several_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.rs");
        let b = dir.path().join("b.rs");
        std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
        std::fs::write(&b, "x\ny\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&a).unwrap();
        editor
            .show_locations(LspFeature::Definition, vec![location(&a, 2, 1)])
            .unwrap();
        assert_eq!(editor.document.cursor(), Position::new(2, 1));
        editor.jump_back(1);
        assert_eq!(editor.document.cursor(), Position::new(0, 0), "a jump");

        editor
            .show_locations(LspFeature::Definition, vec![location(&b, 1, 0)])
            .unwrap();
        assert!(same_file(editor.document.path().unwrap(), &b));
        assert_eq!(editor.document.cursor(), Position::new(1, 0));

        editor
            .show_locations(
                LspFeature::References,
                vec![location(&a, 0, 0), location(&b, 1, 0)],
            )
            .unwrap();
        let picker = editor.picker().unwrap();
        assert_eq!(picker.title(), "References");
        assert_eq!(picker.total(), 2);

        let none = editor.show_locations(LspFeature::Definition, vec![]);
        assert_eq!(none, Err("No definition found".into()));
    }

    #[test]
    fn answers_apply_only_while_the_editor_is_still_where_they_were_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "one two\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&path).unwrap();
        editor.request_lsp(LspFeature::Hover).unwrap();
        let request = editor.take_lsp_requests().remove(0);
        assert!(editor.still_at(&request));
        editor.document.set_cursor(Position::new(0, 4));
        assert!(!editor.still_at(&request), "the cursor moved");
        editor.document.set_cursor(Position::new(0, 0));
        assert!(editor.still_at(&request));
        editor.document.insert(Position::new(0, 0), "x");
        assert!(!editor.still_at(&request), "the text changed");
        assert!(editor.take_lsp_requests().is_empty(), "taken");

        let mut scratch = Editor::default();
        assert!(
            scratch.request_lsp(LspFeature::Definition).is_err(),
            "no file"
        );
    }

    #[test]
    fn the_cursor_moves_with_edits_before_it() {
        let at = Position::new;
        // `use a;\n` put in above: one row down.
        assert_eq!(moved(at(3, 4), at(0, 0), at(1, 0)), at(4, 4));
        // On its row, before it: along by the difference (`2..5` to `2..3`).
        assert_eq!(moved(at(0, 10), at(0, 5), at(0, 3)), at(0, 8));
        // A line joined into the one above (`0,3..1,0` deleted).
        assert_eq!(moved(at(1, 4), at(1, 0), at(0, 3)), at(0, 7));
    }

    #[test]
    fn edits_are_made_last_first_and_the_cursor_stays_on_its_text() {
        let mut editor = Editor::default();
        editor
            .document
            .insert(Position::new(0, 0), "fn  main(){\nlet x=1;\n}");
        editor.document.set_cursor(Position::new(1, 6));
        let at = Position::new;
        editor.apply_text_edits(vec![
            (at(0, 2), at(0, 4), " ".into()),
            (at(0, 10), at(0, 10), " ".into()),
            (at(1, 0), at(1, 0), "    ".into()),
            (at(1, 5), at(1, 6), " = ".into()),
        ]);
        assert_eq!(
            editor.document.buffer().text(),
            "fn main() {\n    let x = 1;\n}"
        );
        // Still on the `1`, moved along by the spaces before it.
        assert_eq!(editor.document.cursor(), at(1, 12));
        // In text an edit replaced: kept where it was, within the new text.
        editor.document.set_cursor(at(1, 6));
        editor.apply_text_edits(vec![(at(1, 4), at(1, 9), "letting".into())]);
        assert_eq!(editor.document.cursor(), at(1, 6));
    }

    #[test]
    fn k_scrolls_the_hover_then_closes_it() {
        let mut editor = Editor::default();
        editor.show_hover("1\n2\n3\n4\n5");
        editor.hover().unwrap().shown.set(2);
        editor.note_key_for_hover();
        assert!(editor.scroll_hover());
        editor.end_hover_unless_used();
        assert_eq!(editor.hover().unwrap().scroll, 2);
        editor.note_key_for_hover();
        editor.scroll_hover();
        editor.end_hover_unless_used();
        assert_eq!(editor.hover().unwrap().scroll, 3, "the last page, full");
        editor.note_key_for_hover();
        editor.scroll_hover();
        editor.end_hover_unless_used();
        assert!(editor.hover().is_none(), "past the end");

        editor.show_hover("x");
        editor.note_key_for_hover();
        editor.end_hover_unless_used();
        assert!(editor.hover().is_none(), "any other key closes it");
    }
}
