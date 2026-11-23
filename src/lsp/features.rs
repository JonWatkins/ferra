//! The editor's requests (hover, go to definition, references): sending
//! them to the server that has the document open, and turning the answers
//! into a hover popup, a jump or a picker.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::client::{Client, ResponseError};
use super::convert::{self, Encoding};
use super::{Lsp, State};
use crate::core::Position;
use crate::editor::Editor;
use crate::editor::lsp::{
    Answer, Applied, Busy, Call, Calls, CodeAction, Edits, Location, LspFeature, LspQuery,
    LspRequest, NoAnswer, Reply,
};
use crate::editor::server_completion::ServerItem;
use crate::editor::signature::SignatureHelp;

/// The kind of code actions the agent's asked for when it gives no line:
/// the file-wide ones (organize imports, fix all, remove unused imports).
const FILE_WIDE_ACTIONS: &str = "source";

impl Lsp {
    /// Moving a file: asks the file's server for the edits the move
    /// needs (imports), if it has one that wants asking; else answers that
    /// there's none to ask, for the asker to just move it.
    fn start_move(&mut self, editor: &mut Editor, request: LspRequest, since: std::time::Instant) {
        let LspQuery::MoveFile { to } = request.query.clone() else {
            return;
        };
        let Some(from) = editor
            .document_by_id(request.document)
            .and_then(|d| d.path())
            .map(std::path::Path::to_path_buf)
        else {
            let why = NoAnswer::Unusable("The file's buffer is gone".into());
            editor.lsp_answered(&request, Err(why));
            return;
        };
        let language = editor
            .document_by_id(request.document)
            .and_then(|d| editor.language_of(d))
            .map(str::to_string);
        // Its server's starting: asked once it's ready (it'd update the
        // imports).
        if let Some(name) = self.starting(language.as_deref()) {
            if let Some(request) = self.wait_for_start(request, since) {
                editor.lsp_answered(&request, Err(NoAnswer::Starting(name)));
            }
            return;
        }
        let client = self
            .clients
            .iter_mut()
            .find(|c| c.has_document(request.document) && c.state == State::Ready);
        let why = match client {
            Some(client) if client.will_rename() => {
                client.request_will_rename(request, &from, &to);
                return;
            }
            Some(client) => NoAnswer::Unsupported(client.name.clone()),
            None => NoAnswer::NoServer,
        };
        editor.lsp_answered(&request, Err(why));
    }

    /// The name of the server starting for `language`, if one is.
    fn starting(&self, language: Option<&str>) -> Option<String> {
        self.clients
            .iter()
            .find(|c| c.state == State::Starting && Some(c.language.as_str()) == language)
            .map(|c| c.name.clone())
    }

    /// Keeps `request`, asked `since`, to send once its server is ready,
    /// if its reply waits for that (the agent's: it can't just ask again
    /// as the user would) and it hasn't waited too long; else hands it back
    /// to be answered that the server is still starting.
    fn wait_for_start(
        &mut self,
        request: LspRequest,
        since: std::time::Instant,
    ) -> Option<LspRequest> {
        if request.reply.agent().is_some() && since.elapsed() < START_WAIT {
            self.waiting.push((request, since));
            return None;
        }
        Some(request)
    }
}

/// How long the agent's requests wait for their server to start.
const START_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long `:format` (and saving, with `format-on-save`) waits.
const FORMAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

impl Lsp {
    /// Has the active buffer's server format it, waiting for the answer
    /// (handling anything else servers send meanwhile), and makes its
    /// edits. With `quiet`, a buffer without a server that formats is fine.
    pub fn format(&mut self, editor: &mut Editor, quiet: bool) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        // The server needs the text as it is now.
        self.receive(editor);
        self.sync(editor);
        let document = editor.document.id();
        let Some(client) = self
            .clients
            .iter_mut()
            .find(|c| c.has_document(document) && c.state == State::Ready)
        else {
            return if quiet {
                Ok(())
            } else {
                Err("No language server for this buffer (see :lsp)".into())
            };
        };
        if !client.supports(LspFeature::Format) {
            return if quiet {
                Ok(())
            } else {
                Err(format!("{} doesn't format", client.name))
            };
        }
        let buffer = editor.document.buffer();
        let request = LspRequest {
            query: LspQuery::Format,
            document,
            position: editor.document.cursor(),
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            reply: Reply::Key,
        };
        let (tab_size, spaces) = editor.format_indent(document);
        let (name, encoding, client_id) = (client.name.clone(), client.encoding, client.id);
        let edit_seq = request.edit_seq;
        let id = client.request_format(request, tab_size, spaces);

        self.formatted = None;
        let deadline = std::time::Instant::now() + FORMAT_TIMEOUT;
        while self.formatted.is_none() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.receiver.recv_timeout(left) {
                Ok(super::Incoming::Message(from, message)) => self.handle(from, message, editor),
                Ok(super::Incoming::Closed(from)) => self.closed(from, editor),
                Err(_) => return Err(format!("{name} took too long to format")),
            }
            let gone = self
                .clients
                .iter()
                .find(|c| c.id == client_id)
                .is_none_or(|c| !c.waiting_for(id));
            if gone && self.formatted.is_none() {
                return Err(format!("{name} stopped"));
            }
        }
        let edits = match self.formatted.take().expect("loop ends with it") {
            Ok(edits) => edits,
            Err(error) => return Err(format!("{name}: {}", error.message)),
        };
        // Made only if nothing changed while waiting (it can't, but a
        // server's edits on the wrong text would garble it).
        let buffer = editor.document.buffer();
        if editor.document.id() != document || buffer.edit_seq() != edit_seq {
            return Err("The text changed while formatting".into());
        }
        let edits = edits
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                let start = convert::from_lsp(buffer, &e["range"]["start"], encoding);
                let end = convert::from_lsp(buffer, &e["range"]["end"], encoding);
                (start, end, e["newText"].as_str().unwrap_or("").to_string())
            })
            .collect();
        editor.apply_text_edits(edits);
        Ok(())
    }

    /// Sends the editor's requests, each to the server with its document
    /// open (synced just before, so it has the text the request is about).
    /// Returns whether there were any (which may have set a message).
    pub(super) fn send_requests(&mut self, editor: &mut Editor) -> bool {
        // The agent's requests waiting for a server to start, again, then
        // the new ones.
        let now = std::time::Instant::now();
        let mut requests: Vec<(LspRequest, std::time::Instant)> = std::mem::take(&mut self.waiting);
        requests.extend(editor.take_lsp_requests().into_iter().map(|r| (r, now)));
        let any = !requests.is_empty();
        for (request, since) in requests {
            if request.feature() == LspFeature::MoveFile {
                self.start_move(editor, request, since);
                continue;
            }
            let Some(document) = editor.document_by_id(request.document) else {
                continue;
            };
            let language = editor.language_of(document).map(str::to_string);
            let client = self
                .clients
                .iter_mut()
                .find(|c| c.has_document(request.document) && c.state == State::Ready);
            let Some(client) = client else {
                match self.starting(language.as_deref()) {
                    // Asked once it's ready, if the reply waits for that.
                    Some(name) => {
                        if let Some(request) = self.wait_for_start(request, since) {
                            editor.lsp_answered(&request, Err(NoAnswer::Starting(name)));
                        }
                    }
                    None => editor.lsp_answered(&request, Err(NoAnswer::NoServer)),
                }
                continue;
            };
            let mut request = request;
            let typing = !request.reply.explicit();
            match &mut request.query {
                // Asked as you type: on a character the server names, or on
                // any change while it's showing (so the parameter follows).
                LspQuery::SignatureHelp { trigger, retrigger } if typing => {
                    let showing = editor.signature_help().is_some();
                    let triggered = trigger.is_some_and(|c| client.signature_triggered_by(c));
                    if !showing && !triggered {
                        continue;
                    }
                    *retrigger = showing;
                }
                LspQuery::Completion {
                    trigger: Some(c), ..
                } if !client.completes_after(*c) => continue,
                _ => {}
            }
            if !client.supports(request.feature()) {
                editor.lsp_answered(&request, Err(NoAnswer::Unsupported(client.name.clone())));
                continue;
            }
            if let LspQuery::ApplyCodeAction { action } = &request.query {
                let server = action.server.clone();
                make_action(editor, client, request, &server);
                continue;
            }
            // The agent's formatting: answered in `answer` (the app's own
            // waits for its answer when saving).
            if request.feature() == LspFeature::Format {
                let (tab_size, spaces) = editor.format_indent(request.document);
                client.request_format(request, tab_size, spaces);
                continue;
            }
            if request.feature() == LspFeature::CodeAction {
                let buffer = document.buffer();
                let LspQuery::CodeActions {
                    start,
                    end,
                    file_wide,
                    with_line,
                } = request.query
                else {
                    continue;
                };
                // The line too: its own row, start to end.
                let line =
                    with_line.then(|| (Position::new(start.row, 0), buffer.line_end(start.row)));
                let (first, last) = (start.row as u64, end.row as u64);
                let diagnostics: Vec<Value> = self
                    .raw_diagnostics
                    .get(&request.document)
                    .into_iter()
                    .flatten()
                    .filter(|d| {
                        let from = d["range"]["start"]["line"].as_u64().unwrap_or(0);
                        let to = d["range"]["end"]["line"].as_u64().unwrap_or(0);
                        from <= last && first <= to
                    })
                    .cloned()
                    .collect();
                let range = |(start, end)| {
                    (
                        convert::to_lsp(buffer, start, client.encoding),
                        convert::to_lsp(buffer, end, client.encoding),
                    )
                };
                // The file-wide ones: source actions only.
                let only = file_wide.then_some(FILE_WIDE_ACTIONS);
                match line {
                    Some(line) => {
                        let (here, line) = (range((start, end)), range(line));
                        let again = (request.clone(), diagnostics.clone());
                        client.request_code_actions(request, here, diagnostics, only, Some(0));
                        client.request_code_actions(again.0, line, again.1, only, Some(1));
                    }
                    None => {
                        let here = range((start, end));
                        client.request_code_actions(request, here, diagnostics, only, None);
                    }
                }
                continue;
            }
            if request.feature() == LspFeature::InlayHint {
                // The whole buffer.
                let buffer = document.buffer();
                let last = buffer.line_count() - 1;
                let start =
                    convert::to_lsp(buffer, crate::core::Position::new(0, 0), client.encoding);
                let end = convert::to_lsp(buffer, buffer.line_end(last), client.encoding);
                client.request_inlay_hints(request, start, end);
                continue;
            }
            if request.feature() == LspFeature::ResolveCompletion {
                let LspQuery::ResolveCompletion { item } = request.query.clone() else {
                    continue;
                };
                client.resolve_completion(request, item);
                continue;
            }
            let position = convert::to_lsp(document.buffer(), request.position, client.encoding);
            client.request_feature(request, position);
        }
        any
    }
}

/// The answers to a request's parts so far (see `Pending::Part`).
pub(super) type Parts = [Option<Result<Value, ResponseError>>; 2];

/// Code actions asked for in parts (at the symbol, then for its line), as
/// one list: the symbol's first, so the numbers stay the same, then the
/// line's not already there. A part that failed counts as none, unless
/// both did.
pub(super) fn merge_actions(parts: Parts) -> Result<Value, ResponseError> {
    let [here, line] = parts.map(|part| part.unwrap_or(Ok(Value::Null)));
    let (here, line) = match (here, line) {
        (Err(error), Err(_)) => return Err(error),
        (here, line) => (here.unwrap_or_default(), line.unwrap_or_default()),
    };
    let mut list: Vec<Value> = here.as_array().cloned().unwrap_or_default();
    for action in line.as_array().into_iter().flatten() {
        if !list.iter().any(|a| a["title"] == action["title"]) {
            list.push(action.clone());
        }
    }
    Ok(Value::Array(list))
}

pub(super) fn answer(
    editor: &mut Editor,
    client: &mut Client,
    request: LspRequest,
    result: Result<Value, ResponseError>,
) {
    // Call hierarchy: the function at the place, then the calls to or
    // from it.
    if matches!(request.feature(), LspFeature::Callers | LspFeature::Callees) {
        calls_answer(editor, client, request, result);
        return;
    }
    if matches!(
        request.feature(),
        LspFeature::Hover
            | LspFeature::Definition
            | LspFeature::Declaration
            | LspFeature::TypeDefinition
            | LspFeature::Implementation
            | LspFeature::References
    ) {
        let answer = match result {
            Ok(result) if request.feature() == LspFeature::Hover => {
                Ok(Answer::Hover(hover_markdown(&result["contents"])))
            }
            Ok(result) => Ok(Answer::Locations {
                found: locations(editor, &result, client.encoding),
                busy: client.busy_with().map(|work| Busy {
                    server: client.name.clone(),
                    work,
                }),
            }),
            Err(error) => Err(server_error(client, error)),
        };
        editor.lsp_answered(&request, answer);
        return;
    }
    // Edits for the asker to make: a rename's, a move's (its imports).
    if matches!(request.feature(), LspFeature::Rename | LspFeature::MoveFile) {
        let answer = match result {
            Ok(result) if result.is_null() => Ok(Answer::Edits(Edits::default())),
            Ok(result) => workspace_edits(editor, &result, client.encoding)
                .map(Answer::Edits)
                .map_err(NoAnswer::Unusable),
            Err(error) => Err(server_error(client, error)),
        };
        editor.lsp_answered(&request, answer);
        return;
    }
    if let LspQuery::CodeActions { file_wide, .. } = request.query {
        let answer = match result {
            Ok(result) => Ok(Answer::Actions(code_actions(&result, file_wide))),
            Err(error) => Err(server_error(client, error)),
        };
        editor.lsp_answered(&request, answer);
        return;
    }
    if request.feature() == LspFeature::ResolveCodeAction {
        match result {
            Ok(resolved) => make_action(editor, client, request, &resolved),
            Err(error) => editor.lsp_answered(&request, Err(server_error(client, error))),
        }
        return;
    }
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            editor.lsp_answered(&request, Err(server_error(client, error)));
            return;
        }
    };
    let busy = || {
        client.busy_with().map(|work| Busy {
            server: client.name.clone(),
            work,
        })
    };
    let answer = match request.feature() {
        LspFeature::Completion => {
            let resolves = client.resolves_completions();
            let (items, incomplete, raw) =
                completion_items(editor, &request, &result, client.encoding);
            // Resolved when taken, for its import: if the server says it
            // resolves items, or the item has `data` for it to
            // (rust-analyzer says it doesn't, but resolves those).
            let items = items
                .into_iter()
                .zip(raw)
                .map(|(mut item, raw)| {
                    if (resolves || raw.get("data").is_some()) && item.other_edits.is_empty() {
                        item.resolve = Some(raw);
                    }
                    item
                })
                .collect();
            Answer::Completion { items, incomplete }
        }
        // A taken item's other edits (its import), in its buffer.
        LspFeature::ResolveCompletion | LspFeature::Format => {
            let Some(document) = editor.document_by_id(request.document) else {
                return;
            };
            let buffer = document.buffer();
            let edits = if request.feature() == LspFeature::Format {
                &result
            } else {
                &result["additionalTextEdits"]
            };
            let edits = edits
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    let start = convert::from_lsp(buffer, &e["range"]["start"], client.encoding);
                    let end = convert::from_lsp(buffer, &e["range"]["end"], client.encoding);
                    (start, end, e["newText"].as_str().unwrap_or("").to_string())
                })
                .collect();
            Answer::Edits(Edits {
                documents: vec![(request.document, edits)],
                ..Edits::default()
            })
        }
        LspFeature::InlayHint => {
            let Some(document) = editor.document_by_id(request.document) else {
                return;
            };
            Answer::InlayHints(inlay_hints(document.buffer(), &result, client.encoding))
        }
        LspFeature::DocumentHighlight => {
            let Some(document) = editor.document_by_id(request.document) else {
                return;
            };
            let buffer = document.buffer();
            let ranges = result
                .as_array()
                .into_iter()
                .flatten()
                .map(|h| {
                    (
                        convert::from_lsp(buffer, &h["range"]["start"], client.encoding),
                        convert::from_lsp(buffer, &h["range"]["end"], client.encoding),
                    )
                })
                .collect();
            Answer::Highlights(ranges)
        }
        LspFeature::Outline => {
            Answer::Outline(outline(editor, request.document, &result, client.encoding))
        }
        LspFeature::DocumentSymbols | LspFeature::WorkspaceSymbols => Answer::Symbols {
            found: symbols(editor, request.document, &result, client.encoding),
            busy: busy(),
        },
        LspFeature::SignatureHelp => Answer::Signature(signature_help(&result)),
        _ => return,
    };
    editor.lsp_answered(&request, Ok(answer));
}

/// A server's error answer, as the reason there's no answer.
fn server_error(client: &Client, error: ResponseError) -> NoAnswer {
    NoAnswer::ServerError {
        server: client.name.clone(),
        stale: error.is_stale(),
        message: error.message,
    }
}

/// A call hierarchy answer: after `prepareCallHierarchy`, asks for the
/// calls to or from the function found; after that, answers with them.
fn calls_answer(
    editor: &mut Editor,
    client: &mut Client,
    request: LspRequest,
    result: Result<Value, ResponseError>,
) {
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            editor.lsp_answered(&request, Err(server_error(client, error)));
            return;
        }
    };
    let found = matches!(
        request.query,
        LspQuery::Callers { found: true } | LspQuery::Callees { found: true }
    );
    if !found {
        match result.as_array().and_then(|items| items.first()) {
            Some(item) => {
                let item = item.clone();
                client.request_calls(
                    LspRequest {
                        query: match request.query {
                            LspQuery::Callers { .. } => LspQuery::Callers { found: true },
                            _ => LspQuery::Callees { found: true },
                        },
                        ..request
                    },
                    item,
                );
            }
            None => editor.lsp_answered(&request, Err(NoAnswer::NothingThere)),
        }
        return;
    }
    let callers = request.feature() == LspFeature::Callers;
    let calls = calls(&result, callers, &client.root);
    editor.lsp_answered(&request, Ok(Answer::Calls(calls)));
}

/// The functions in an `incomingCalls` / `outgoingCalls` answer: those in
/// the project (under the server's `root`) and the names of those outside
/// it (the standard library, other crates). Enum members (`Ok`, `Err`)
/// aren't calls.
fn calls(result: &Value, callers: bool, root: &std::path::Path) -> Calls {
    let mut answer = Calls::default();
    for call in result.as_array().into_iter().flatten() {
        let item = if callers { &call["from"] } else { &call["to"] };
        let name = item["name"].as_str().unwrap_or("?");
        let kind = symbol_kind(item["kind"].as_u64());
        if kind == "enum member" {
            continue;
        }
        let path = item["uri"].as_str().and_then(convert::uri_to_path);
        let in_project = path.as_ref().is_some_and(|p| {
            p.starts_with(root)
                || std::fs::canonicalize(p)
                    .ok()
                    .zip(std::fs::canonicalize(root).ok())
                    .is_some_and(|(p, root)| p.starts_with(root))
        });
        let (Some(path), true) = (path, in_project) else {
            if !answer.outside.iter().any(|n| n == name) {
                answer.outside.push(name.to_string());
            }
            continue;
        };
        let line = item["selectionRange"]["start"]["line"]
            .as_u64()
            .unwrap_or(0) as usize
            + 1;
        let mut at: Vec<usize> = call["fromRanges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["start"]["line"].as_u64().map(|l| l as usize + 1))
            .collect();
        at.sort_unstable();
        at.dedup();
        let detail = item["detail"]
            .as_str()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        answer.ours.push(Call {
            name: name.to_string(),
            kind,
            detail,
            path,
            line,
            at,
        });
    }
    answer
}

/// The symbols in a `documentSymbol` answer (`DocumentSymbol`s, nested, in
/// `document`; or flat `SymbolInformation`s) or a `workspace/symbol` one:
/// each one's name, what it is (`method in Circle`), and where it is, in
/// order.
fn symbols(
    editor: &Editor,
    document: crate::editor::DocumentId,
    result: &Value,
    encoding: Encoding,
) -> Vec<(String, String, Location)> {
    // Nested `DocumentSymbol`s, in this file: their `selectionRange` is the
    // name; what they're in is their parent.
    fn nested(symbols: &[Value], parent: Option<&str>, out: &mut Vec<(String, String, Value)>) {
        for symbol in symbols {
            let name = symbol["name"].as_str().unwrap_or("").to_string();
            let kind = symbol_kind(symbol["kind"].as_u64());
            let detail = match parent {
                Some(parent) => format!("{kind} in {parent}"),
                None => kind.to_string(),
            };
            out.push((name.clone(), detail, symbol["selectionRange"].clone()));
            if let Some(children) = symbol["children"].as_array() {
                nested(children, Some(&name), out);
            }
        }
    }
    let list = result.as_array().map_or(&[][..], Vec::as_slice);
    let mut named: Vec<(String, String, Value)> = Vec::new();
    let places: Vec<Value> = if list
        .first()
        .is_some_and(|s| s.get("selectionRange").is_some())
    {
        nested(list, None, &mut named);
        let path = editor
            .document_by_id(document)
            .and_then(|d| d.path())
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        let uri = convert::path_to_uri(&path);
        named
            .iter()
            .map(|(_, _, range)| json!({ "uri": uri, "range": range }))
            .collect()
    } else {
        // `SymbolInformation`s or `WorkspaceSymbol`s: a location each, in
        // any file, and the name of what they're in (a `WorkspaceSymbol`'s
        // location can lack a range: its file's start, then).
        list.iter()
            .map(|symbol| {
                let name = symbol["name"].as_str().unwrap_or("").to_string();
                let kind = symbol_kind(symbol["kind"].as_u64());
                let detail = match symbol["containerName"].as_str().filter(|c| !c.is_empty()) {
                    Some(container) => format!("{kind} in {container}"),
                    None => kind.to_string(),
                };
                named.push((name, detail, Value::Null));
                let location = &symbol["location"];
                let range = location
                    .get("range")
                    .cloned()
                    .unwrap_or_else(|| json!({ "start": { "line": 0, "character": 0 } }));
                json!({ "uri": location["uri"], "range": range })
            })
            .collect()
    };
    named
        .into_iter()
        .zip(resolve_places(editor, &places, encoding))
        .filter_map(|((name, detail, _), place)| Some((name, detail, place?)))
        .collect()
}

/// A short name for an LSP `SymbolKind`.
/// The Outline's symbols from a `textDocument/documentSymbol` answer: the
/// nested `DocumentSymbol`s in order with their depth, or the flat
/// `SymbolInformation`s, their depth from which contain which.
fn outline(
    editor: &Editor,
    document: crate::editor::DocumentId,
    result: &Value,
    encoding: Encoding,
) -> Vec<crate::editor::outline::OutlineSymbol> {
    use crate::editor::outline::OutlineSymbol;
    let Some(buffer) = editor.document_by_id(document).map(|d| d.buffer()) else {
        return Vec::new();
    };
    let position = |v: &Value| convert::from_lsp(buffer, v, encoding);
    fn nested(
        symbols: &[Value],
        depth: usize,
        position: &dyn Fn(&Value) -> crate::core::Position,
        out: &mut Vec<OutlineSymbol>,
    ) {
        for symbol in symbols {
            out.push(OutlineSymbol {
                name: symbol["name"].as_str().unwrap_or("").to_string(),
                kind: symbol_kind(symbol["kind"].as_u64()).to_string(),
                depth,
                start: position(&symbol["range"]["start"]),
                end: position(&symbol["range"]["end"]),
                at: position(&symbol["selectionRange"]["start"]),
            });
            if let Some(children) = symbol["children"].as_array() {
                nested(children, depth + 1, position, out);
            }
        }
    }
    let list = result.as_array().map_or(&[][..], Vec::as_slice);
    let mut out = Vec::new();
    if list
        .first()
        .is_some_and(|s| s.get("selectionRange").is_some())
    {
        nested(list, 0, &position, &mut out);
        return out;
    }
    // Flat: in order, each as deep as the symbols around it.
    let mut flat: Vec<OutlineSymbol> = list
        .iter()
        .map(|symbol| {
            let range = &symbol["location"]["range"];
            OutlineSymbol {
                name: symbol["name"].as_str().unwrap_or("").to_string(),
                kind: symbol_kind(symbol["kind"].as_u64()).to_string(),
                depth: 0,
                start: position(&range["start"]),
                end: position(&range["end"]),
                at: position(&range["start"]),
            }
        })
        .collect();
    flat.sort_by_key(|s| s.start);
    for i in 0..flat.len() {
        let (before, rest) = flat.split_at_mut(i);
        let symbol = &mut rest[0];
        symbol.depth = before
            .iter()
            .filter(|outer| outer.start <= symbol.start && symbol.end <= outer.end)
            .count();
    }
    flat
}

fn symbol_kind(kind: Option<u64>) -> &'static str {
    match kind {
        Some(1) => "file",
        Some(2) => "module",
        Some(3) => "namespace",
        Some(4) => "package",
        Some(5) => "class",
        Some(6) => "method",
        Some(7) => "property",
        Some(8) => "field",
        Some(9) => "constructor",
        Some(10) => "enum",
        Some(11) => "interface",
        Some(12) => "function",
        Some(13) => "variable",
        Some(14) => "constant",
        Some(15) => "string",
        Some(16) => "number",
        Some(17) => "boolean",
        Some(18) => "array",
        Some(19) => "object",
        Some(20) => "key",
        Some(21) => "null",
        Some(22) => "enum member",
        Some(23) => "struct",
        Some(24) => "event",
        Some(25) => "operator",
        Some(26) => "type parameter",
        _ => "symbol",
    }
}

/// The signature to show from a `SignatureHelp` answer: the active one,
/// with its active parameter's place in its label, and the parameter's
/// documentation (or else the signature's). `None` for none.
fn signature_help(result: &Value) -> Option<SignatureHelp> {
    let signatures = result["signatures"].as_array().filter(|s| !s.is_empty())?;
    let index =
        (result["activeSignature"].as_u64().unwrap_or(0) as usize).min(signatures.len() - 1);
    let signature = &signatures[index];
    let label = signature["label"].as_str()?.to_string();
    let parameters = signature["parameters"].as_array();
    // The signature's own active parameter wins over the answer's.
    let active = signature["activeParameter"]
        .as_u64()
        .or_else(|| result["activeParameter"].as_u64())
        .and_then(|i| parameters?.get(i as usize));
    let place = active.and_then(|parameter| match &parameter["label"] {
        // A substring of the signature's label.
        Value::String(text) => {
            let byte = label.find(text.as_str())?;
            let start = label[..byte].chars().count();
            Some((start, start + text.chars().count()))
        }
        // Offsets into it, in UTF-16 units.
        Value::Array(offsets) => {
            let at = |i: usize| offsets.get(i).and_then(Value::as_u64).map(|n| n as usize);
            let utf16_to_chars = |units: usize| {
                let mut seen = 0;
                label
                    .chars()
                    .take_while(|c| {
                        seen += c.len_utf16();
                        seen <= units
                    })
                    .count()
            };
            Some((utf16_to_chars(at(0)?), utf16_to_chars(at(1)?)))
        }
        _ => None,
    });
    let documentation = |value: &Value| -> Vec<String> {
        let text = match value {
            Value::String(text) => text.as_str(),
            Value::Object(markup) => markup["value"].as_str().unwrap_or(""),
            _ => "",
        };
        // The first paragraph, up to three lines, leaving out code blocks
        // (the signature is the code).
        let mut in_code = false;
        text.lines()
            .map(str::trim_end)
            .filter(|l| {
                if l.trim_start().starts_with("```") {
                    in_code = !in_code;
                    return false;
                }
                !in_code
            })
            .skip_while(|l| l.trim().is_empty())
            .take_while(|l| !l.trim().is_empty())
            .take(3)
            .map(str::to_string)
            .collect()
    };
    let mut docs = active
        .map(|p| documentation(&p["documentation"]))
        .unwrap_or_default();
    if docs.is_empty() {
        docs = documentation(&signature["documentation"]);
    }
    Some(SignatureHelp {
        label,
        active: place,
        index,
        total: signatures.len(),
        documentation: docs,
    })
}

/// Makes code action `action` (as the server has it) for `request`: runs
/// its command, if it has one, and answers with its edits for the asker to
/// make. One with neither is resolved first, if the server fills them in
/// then.
fn make_action(editor: &mut Editor, client: &mut Client, request: LspRequest, action: &Value) {
    let title = action["title"].as_str().unwrap_or("Done").to_string();
    // A bare `Command` rather than a `CodeAction`.
    if action["command"].is_string() {
        client.execute_command(action, &title);
        editor.lsp_answered(&request, Ok(Answer::Action { title, edits: None }));
        return;
    }
    let edit = action.get("edit").filter(|e| !e.is_null());
    let command = action.get("command").filter(|c| c.is_object());
    if edit.is_none() && command.is_none() {
        if let LspQuery::ApplyCodeAction { action: listed } = &request.query
            && (client.resolves_code_actions() || action.get("data").is_some())
        {
            let request = LspRequest {
                query: LspQuery::ResolveCodeAction {
                    action: listed.clone(),
                },
                ..request
            };
            client.resolve_code_action(request, action.clone());
            return;
        }
        let why = NoAnswer::Unusable("That code action does nothing".into());
        editor.lsp_answered(&request, Err(why));
        return;
    }
    let edits = match edit.map(|edit| workspace_edits(editor, edit, client.encoding)) {
        Some(Ok(edits)) => Some(edits),
        Some(Err(error)) => {
            editor.lsp_answered(&request, Err(NoAnswer::Unusable(error)));
            return;
        }
        None => None,
    };
    if let Some(command) = command {
        client.execute_command(command, &title);
    }
    editor.lsp_answered(&request, Ok(Answer::Action { title, edits }));
}

/// The code actions in an answer, but those the server says can't be done
/// there; with `file_wide`, only the source ones (servers send quick fixes
/// too, for the diagnostics sent with the request: the client's to filter
/// by `only`).
fn code_actions(result: &Value, file_wide: bool) -> Vec<CodeAction> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a.get("disabled").is_none())
        .filter(|a| {
            !file_wide
                || a["kind"].as_str().is_some_and(|kind| {
                    kind == FILE_WIDE_ACTIONS || kind.starts_with(&format!("{FILE_WIDE_ACTIONS}."))
                })
        })
        .map(|a| CodeAction {
            title: a["title"].as_str().unwrap_or("(untitled)").to_string(),
            kind: a["kind"].as_str().unwrap_or("").to_string(),
            server: a.clone(),
        })
        .collect()
}

/// A `WorkspaceEdit`'s changes (`changes` by URI, or `documentChanges`)
/// in the editor's terms, opening files that aren't open in the
/// background to read their columns. Creating, renaming and deleting
/// files is left out (and counted).
pub(super) fn workspace_edits(
    editor: &mut Editor,
    edit: &Value,
    encoding: Encoding,
) -> Result<Edits, String> {
    let mut files: Vec<(&str, &Vec<Value>)> = Vec::new();
    let mut edits = Edits::default();
    if let Some(changes) = edit["documentChanges"].as_array() {
        for change in changes {
            match (
                change["textDocument"]["uri"].as_str(),
                change["edits"].as_array(),
            ) {
                (Some(uri), Some(changes)) => files.push((uri, changes)),
                _ => edits.skipped += 1,
            }
        }
    } else if let Some(changes) = edit["changes"].as_object() {
        for (uri, changes) in changes {
            if let Some(changes) = changes.as_array() {
                files.push((uri, changes));
            }
        }
    }
    for (uri, changes) in files {
        if changes.is_empty() {
            continue;
        }
        let path = convert::uri_to_path(uri).ok_or_else(|| format!("Can't edit {uri}"))?;
        let was_open = editor.find_open_path(&path).is_some();
        let id = editor.buffer_for_path(&path)?;
        if !was_open {
            edits.opened.push(id);
        }
        let buffer = editor.document_by_id(id).expect("just found").buffer();
        let converted = changes
            .iter()
            .map(|e| {
                let start = convert::from_lsp(buffer, &e["range"]["start"], encoding);
                let end = convert::from_lsp(buffer, &e["range"]["end"], encoding);
                (start, end, e["newText"].as_str().unwrap_or("").to_string())
            })
            .collect();
        edits.documents.push((id, converted));
    }
    Ok(edits)
}

/// Makes a `WorkspaceEdit`'s changes (see `workspace_edits`), each file's
/// as one undo step.
pub(super) fn apply_workspace_edit(
    editor: &mut Editor,
    edit: &Value,
    encoding: Encoding,
) -> Result<Applied, String> {
    let edits = workspace_edits(editor, edit, encoding)?;
    Ok(editor.apply_lsp_edits(edits))
}

/// The items in a completion answer (`CompletionItem[]`, a
/// `CompletionList` or `null`), whether the server has more, and the items
/// as it sent them.
fn completion_items(
    editor: &Editor,
    request: &LspRequest,
    result: &Value,
    encoding: Encoding,
) -> (Vec<ServerItem>, bool, Vec<Value>) {
    let (items, incomplete) = match result {
        Value::Array(items) => (items.as_slice(), false),
        Value::Object(list) => (
            list.get("items")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice),
            list.get("isIncomplete")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        _ => (&[][..], false),
    };
    let Some(document) = editor.document_by_id(request.document) else {
        return (Vec::new(), false, Vec::new());
    };
    let buffer = document.buffer();
    let range = |range: &Value| {
        (
            convert::from_lsp(buffer, &range["start"], encoding),
            convert::from_lsp(buffer, &range["end"], encoding),
        )
    };
    // Items' edits are nearly all on the word's line: it's read once, not
    // once per item (there can be hundreds).
    let row = request.word_start().row;
    let line = buffer.line(row);
    let start_of = |range: &Value| {
        let start = &range["start"];
        if start["line"].as_u64() == Some(row as u64) {
            let character = start["character"].as_u64().unwrap_or(0) as usize;
            Position::new(row, convert::col_from_lsp(&line, character, encoding))
        } else {
            convert::from_lsp(buffer, start, encoding)
        }
    };
    let items = items
        .iter()
        .map(|item| {
            let label = item["label"].as_str().unwrap_or_default().to_string();
            let edit = &item["textEdit"];
            // An edit's range can start before the word (`.` included):
            // what it puts in from the word's start on.
            let mut text = edit["newText"]
                .as_str()
                .or_else(|| item["insertText"].as_str())
                .unwrap_or(&label)
                .to_string();
            // Where it goes in: from the start of its edit's range when
            // that's before the word on its line (the TypeScript server
            // replaces the `.` typed with `?.` for an optional value, clangd
            // with `->` for a pointer), else from the word's start.
            let edit_range = edit
                .get("range")
                .or_else(|| edit.get("insert"))
                .or_else(|| edit.get("replace"));
            let start = edit_range
                .map(start_of)
                .filter(|s| s.row == request.word_start().row && *s < request.word_start())
                .unwrap_or(request.word_start());
            let mut cursor = None;
            if item["insertTextFormat"].as_u64() == Some(2) {
                (text, cursor) = expand_snippet(&text);
            }
            let detail = [
                item["labelDetails"]["detail"].as_str(),
                item["detail"].as_str(),
                item["labelDetails"]["description"].as_str(),
            ]
            .into_iter()
            .flatten()
            .map(|d| d.lines().next().unwrap_or("").trim())
            .find(|d| !d.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| kind_name(item["kind"].as_u64()).to_string());
            let other_edits = item["additionalTextEdits"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    let (start, end) = range(&e["range"]);
                    (start, end, e["newText"].as_str().unwrap_or("").to_string())
                })
                .collect();
            ServerItem {
                start,
                cursor,
                filter: item["filterText"].as_str().unwrap_or(&label).to_string(),
                sort: item["sortText"].as_str().unwrap_or(&label).to_string(),
                label,
                text,
                detail,
                other_edits,
                resolve: None,
            }
        })
        .collect();
    (items, incomplete, items_raw(result))
}

fn items_raw(result: &Value) -> Vec<Value> {
    match result {
        Value::Array(items) => items.clone(),
        Value::Object(list) => list
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A snippet (`printf(${1:fmt}, ${2:args})$0`) as plain text, and where
/// the cursor goes after it's put in, in chars: the first tab stop (`$1`),
/// or else the last (`$0`). The first stop's placeholder is left out, as
/// there's no selecting it for typing over (`printf(|, args)`); the
/// others' text is kept.
fn expand_snippet(snippet: &str) -> (String, Option<usize>) {
    let mut stops = Vec::new();
    let first_stop = (1..100)
        .find(|n| snippet.contains(&format!("${{{n}:")) || snippet.contains(&format!("${n}")));
    let text = expand_into(snippet, 0, &mut stops, first_stop);
    let first = stops
        .iter()
        .filter(|(n, _)| *n > 0)
        .min_by_key(|(n, _)| *n)
        .or_else(|| stops.iter().find(|(n, _)| *n == 0))
        .map(|&(_, at)| at);
    (text, first)
}

/// Expands `snippet`, whose text starts `offset` chars in, noting each tab
/// stop's number and place in `stops`.
fn expand_into(
    snippet: &str,
    offset: usize,
    stops: &mut Vec<(u32, usize)>,
    blank: Option<u32>,
) -> String {
    let mut out = String::new();
    let mut len = 0;
    let mut chars = snippet.chars().peekable();
    let number = |chars: &mut std::iter::Peekable<std::str::Chars>| {
        let mut digits = String::new();
        while let Some(d) = chars.next_if(char::is_ascii_digit) {
            digits.push(d);
        }
        digits.parse::<u32>().ok()
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                    len += 1;
                }
            }
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                if let Some(n) = number(&mut chars) {
                    stops.push((n, offset + len));
                }
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                let n = number(&mut chars);
                if let Some(n) = n {
                    stops.push((n, offset + len));
                }
                // `${1:text}` keeps `text` (which may nest); `${1|a,b|}`
                // the first choice.
                let mut depth = 1;
                let mut inner = String::new();
                for c in chars.by_ref() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    inner.push(c);
                }
                let inner = if n.is_some() && n == blank {
                    String::new()
                } else if let Some(rest) = inner.strip_prefix(':') {
                    expand_into(rest, offset + len, stops, blank)
                } else if let Some(choices) = inner.strip_prefix('|') {
                    choices.split([',', '|']).next().unwrap_or("").to_string()
                } else {
                    String::new()
                };
                len += inner.chars().count();
                out.push_str(&inner);
            }
            _ => {
                out.push(c);
                len += 1;
            }
        }
    }
    out
}

/// A short name for an LSP `CompletionItemKind`.
fn kind_name(kind: Option<u64>) -> &'static str {
    match kind {
        Some(2) => "method",
        Some(3) => "function",
        Some(4) => "constructor",
        Some(5) => "field",
        Some(6) => "variable",
        Some(7) => "class",
        Some(8) => "interface",
        Some(9) => "module",
        Some(10) => "property",
        Some(13) => "enum",
        Some(14) => "keyword",
        Some(15) => "snippet",
        Some(17) => "file",
        Some(19) => "folder",
        Some(20) => "enum member",
        Some(21) => "constant",
        Some(22) => "struct",
        Some(25) => "type parameter",
        _ => "",
    }
}

/// A hover's contents (Markdown, a marked string, or a list of them) as
/// one Markdown text.
fn hover_markdown(contents: &Value) -> String {
    match contents {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(hover_markdown)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(object) => {
            let value = object.get("value").and_then(Value::as_str).unwrap_or("");
            match object.get("language").and_then(Value::as_str) {
                // A `MarkedString` in a language: a code block.
                Some(language) => format!("```{language}\n{value}\n```"),
                None => value.to_string(),
            }
        }
        _ => String::new(),
    }
}

/// The places in a definition or references answer (`Location`,
/// `Location[]`, `LocationLink[]` or `null`), with char columns, in order
/// and without repeats.
fn locations(editor: &Editor, result: &Value, encoding: Encoding) -> Vec<Location> {
    let items: Vec<&Value> = match result {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![result],
        _ => Vec::new(),
    };
    // A `LocationLink` points at the name in `targetSelectionRange`.
    let places: Vec<Value> = items
        .iter()
        .map(|item| match item.get("targetUri") {
            Some(uri) => json!({ "uri": uri, "range": item["targetSelectionRange"] }),
            None => (*item).clone(),
        })
        .collect();
    let mut found: Vec<Location> = Vec::new();
    for location in resolve_places(editor, &places, encoding)
        .into_iter()
        .flatten()
    {
        if !found.contains(&location) {
            found.push(location);
        }
    }
    found
}

/// Each `Location` (`{uri, range}`) as a path and a char position, with the
/// text of its line: from the buffer if the file is open, else read from
/// disk (once per file). `None` for one that isn't a file.
fn resolve_places(editor: &Editor, places: &[Value], encoding: Encoding) -> Vec<Option<Location>> {
    let mut files: HashMap<PathBuf, Option<String>> = HashMap::new();
    places
        .iter()
        .map(|place| {
            let path = place["uri"].as_str().and_then(convert::uri_to_path)?;
            let start = &place["range"]["start"];
            let row = start["line"].as_u64().unwrap_or(0) as usize;
            let character = start["character"].as_u64().unwrap_or(0) as usize;
            let line = match editor.find_open_path(&path) {
                Some(id) => editor.document_by_id(id).and_then(|d| {
                    let buffer = d.buffer();
                    (row < buffer.line_count()).then(|| buffer.line(row).to_string())
                }),
                None => files
                    .entry(path.clone())
                    .or_insert_with(|| std::fs::read_to_string(&path).ok())
                    .as_deref()
                    .and_then(|text| text.lines().nth(row))
                    .map(str::to_string),
            }
            .unwrap_or_default();
            let col = convert::col_from_lsp(&line, character, encoding);
            Some(Location {
                path,
                position: Position::new(row, col),
                line: line.trim_end_matches(['\n', '\r']).to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hover_contents_of_every_kind_become_markdown() {
        let markup = json!({"kind": "markdown", "value": "**x**"});
        assert_eq!(hover_markdown(&markup), "**x**");
        let marked = json!([{"language": "rust", "value": "fn f()"}, "", "docs"]);
        assert_eq!(hover_markdown(&marked), "```rust\nfn f()\n```\n\ndocs");
        assert_eq!(hover_markdown(&Value::Null), "");
    }

    #[test]
    fn snippets_become_text_with_the_cursors_place() {
        let expand = |s: &str| expand_snippet(s);
        // The first stop's placeholder is left for typing in.
        assert_eq!(
            expand("area(${1:r}, ${2:s})$0"),
            ("area(, s)".into(), Some(5))
        );
        assert_eq!(
            expand("if ${1:x} {\n\t$0\n}"),
            ("if  {\n\t\n}".into(), Some(3))
        );
        assert_eq!(
            expand("color: $0;"),
            ("color: ;".into(), Some(7)),
            "only $0"
        );
        assert_eq!(
            expand("a ${2:b ${1:c}}"),
            ("a b ".into(), Some(4)),
            "nested $1"
        );
        assert_eq!(expand("area()$0"), ("area()".into(), Some(6)));
        assert_eq!(
            expand("${1|one,two|}"),
            ("one".into(), Some(0)),
            "a choice: the first"
        );
        assert_eq!(expand("cost \\$5"), ("cost $5".into(), None));
    }

    #[test]
    fn completion_items_keep_the_word_and_its_details() {
        let mut editor = Editor::default();
        editor.document.insert(Position::new(0, 0), "x.ar");
        let request = LspRequest {
            query: LspQuery::Completion {
                trigger: None,
                word_start: Position::new(0, 2),
            },
            document: editor.document.id(),
            position: Position::new(0, 4),
            buffer_id: 0,
            edit_seq: 0,
            reply: Reply::Typing,
        };
        let range = |from: u64| {
            json!({"start": {"line": 0, "character": from},
                                       "end": {"line": 0, "character": 4}})
        };
        let result = json!({"isIncomplete": true, "items": [
            // An edit from the `.`: it goes in from there.
            {"label": "area", "textEdit": {"range": range(1), "newText": "?.area"},
             "labelDetails": {"detail": "(&self) -> f64"}},
            {"label": "arity", "detail": "usize\nmore", "filterText": "ar_ity",
             "sortText": "0"},
            {"label": "Argh", "kind": 22},
        ]});
        let (items, incomplete, raw) = completion_items(&editor, &request, &result, Encoding::Utf8);
        assert!(incomplete);
        assert_eq!(raw.len(), 3);
        let shown: Vec<_> = items
            .iter()
            .map(|i| {
                (
                    i.start.col,
                    i.text.as_str(),
                    i.detail.as_str(),
                    i.filter.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (1, "?.area", "(&self) -> f64", "area"),
                (2, "arity", "usize", "ar_ity"),
                (2, "Argh", "struct", "Argh"),
            ]
        );
    }

    #[test]
    fn signatures_with_the_active_parameter_by_name_or_offsets() {
        // `é` is one UTF-16 unit but two bytes; `😀` two units.
        let answer = json!({"signatures": [
            {"label": "f(a: u8)", "parameters": [{"label": "a: u8"}]},
            {"label": "é😀(x, why)", "activeParameter": 1,
             "documentation": {"kind": "markdown", "value": "```rust\nfn g()\n```\nDoes g.\n\nMore."},
             "parameters": [{"label": [4, 5]}, {"label": [7, 10], "documentation": ""}]},
        ], "activeSignature": 1, "activeParameter": 0});
        let help = signature_help(&answer).unwrap();
        assert_eq!((help.index, help.total), (1, 2));
        let (s, e) = help.active.unwrap();
        let active: String = help.label.chars().skip(s).take(e - s).collect();
        assert_eq!(active, "why", "the signature's own active parameter");
        assert_eq!(
            help.documentation,
            ["Does g."],
            "first paragraph, no fences"
        );

        let first = json!({"signatures": [{"label": "f(a: u8)", "parameters": [{"label": "a: u8"}]}],
                           "activeParameter": 0});
        assert_eq!(signature_help(&first).unwrap().active, Some((2, 7)));
        assert_eq!(signature_help(&json!({"signatures": []})), None);
        assert_eq!(signature_help(&Value::Null), None);
    }

    #[test]
    fn workspace_edits_by_uri_and_what_was_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.rs");
        let b = dir.path().join("b.rs");
        std::fs::write(&a, "old here\n").unwrap();
        std::fs::write(&b, "x old\nold\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&a).unwrap();
        let edit = |line: u64, from: u64, to: u64| {
            json!({"range": {"start": {"line": line, "character": from},
                             "end": {"line": line, "character": to}}, "newText": "new"})
        };
        let changes = json!({"changes": {
            convert::path_to_uri(&a): [edit(0, 0, 3)],
            convert::path_to_uri(&b): [edit(0, 2, 5), edit(1, 0, 3)],
        }});
        let applied = apply_workspace_edit(&mut editor, &changes, Encoding::Utf8).unwrap();
        assert_eq!(
            applied,
            Applied {
                changes: 3,
                files: 2,
                others: 1,
                skipped: 0
            }
        );
        assert_eq!(editor.document.buffer().text(), "new here");
        let other = editor.find_open_path(&b).unwrap();
        let other = editor.document_by_id(other).unwrap();
        assert_eq!(other.buffer().text(), "x new\nnew");
        assert!(other.is_modified(), "left for :wa");

        let with_ops = json!({"documentChanges": [
            {"kind": "rename", "oldUri": "file:///x", "newUri": "file:///y"},
        ]});
        let applied = apply_workspace_edit(&mut editor, &with_ops, Encoding::Utf8).unwrap();
        assert_eq!(applied.skipped, 1);
        assert_eq!(
            applied.describe(),
            "0 changes in 0 files; 1 file operation left out"
        );
        let many = Applied {
            changes: 1,
            files: 1,
            others: 2,
            skipped: 0,
        };
        assert_eq!(
            many.describe(),
            "1 change in 1 file (2 other files not saved yet: :wa)"
        );
    }

    #[test]
    fn code_actions_asked_in_parts_are_one_list() {
        let titles = |v: &Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|a| a["title"].as_str().unwrap().to_string())
                .collect()
        };
        let error = || ResponseError {
            code: -32603,
            message: "busy".into(),
        };
        let merged = merge_actions([
            Some(Ok(json!([{"title": "Extract"}, {"title": "Fix"}]))),
            Some(Ok(json!([{"title": "Fix"}, {"title": "Fix the line"}]))),
        ]);
        assert_eq!(
            titles(&merged.unwrap()),
            ["Extract", "Fix", "Fix the line"],
            "the symbol's first, then the line's not already there"
        );
        let one_failed = merge_actions([Some(Err(error())), Some(Ok(json!([{"title": "Fix"}])))]);
        assert_eq!(titles(&one_failed.unwrap()), ["Fix"]);
        let none = merge_actions([Some(Ok(Value::Null)), Some(Ok(json!([])))]);
        assert_eq!(none.unwrap(), json!([]));
        let both = merge_actions([Some(Err(error())), Some(Err(error()))]);
        assert_eq!(both.unwrap_err().message, "busy");
    }

    #[test]
    fn calls_in_the_project_and_outside_it() {
        let range = |line: u64| json!({"start": {"line": line, "character": 4}, "end": {"line": line, "character": 7}});
        let item = |name: &str, kind: u64, uri: &str, detail: &str| {
            json!({"from": {"name": name, "kind": kind, "uri": uri, "detail": detail,
                            "range": range(3), "selectionRange": range(3)},
                   "fromRanges": [range(9), range(5), range(9)]})
        };
        let answer = calls(
            &json!([
                item("go", 12, "file:///p/a.rs", "  fn go()  "),
                item("Ok", 22, "file:///p/a.rs", ""),
                item("push", 6, "file:///rust/vec.rs", ""),
                item("push", 6, "file:///rust/vec.rs", ""),
            ]),
            true,
            std::path::Path::new("/p"),
        );
        assert_eq!(
            answer.ours,
            [Call {
                name: "go".into(),
                kind: "function",
                detail: Some("fn go()".into()),
                path: PathBuf::from("/p/a.rs"),
                line: 4,
                at: vec![6, 10],
            }]
        );
        assert_eq!(answer.outside, ["push"], "once each; Ok isn't a call");
    }

    #[test]
    fn locations_and_links_with_columns_from_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "first\n  é𝄞 thing\n").unwrap();
        let uri = convert::path_to_uri(&path);
        let at = |line, character| json!({"line": line, "character": character});
        let result = json!([
            {"uri": uri, "range": {"start": at(1, 6), "end": at(1, 11)}},
            {"targetUri": uri, "targetRange": {"start": at(0, 0), "end": at(1, 0)},
             "targetSelectionRange": {"start": at(0, 1), "end": at(0, 2)}},
            {"uri": uri, "range": {"start": at(1, 6), "end": at(1, 11)}},
        ]);
        let editor = Editor::default();
        let found = locations(&editor, &result, Encoding::Utf16);
        let places: Vec<_> = found
            .iter()
            .map(|l| (l.position, l.line.as_str()))
            .collect();
        assert_eq!(
            places,
            [
                // `é` is one UTF-16 unit, `𝄞` two: column 6 is `thing`'s.
                (Position::new(1, 5), "  é𝄞 thing"),
                (Position::new(0, 1), "first"),
            ],
            "repeats dropped"
        );
        assert!(locations(&editor, &Value::Null, Encoding::Utf8).is_empty());
    }
}

/// A server's inlay hints as virtual text: each label (a string, or parts
/// with `value`s), with a space either side if it asks for padding.
pub(super) fn inlay_hints(
    buffer: &crate::core::Buffer,
    result: &Value,
    encoding: convert::Encoding,
) -> Vec<crate::editor::VirtualText> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|hint| {
            let label = match &hint["label"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
                _ => return None,
            };
            if label.is_empty() {
                return None;
            }
            let pad = |side: &str| {
                if hint[side].as_bool() == Some(true) {
                    " "
                } else {
                    ""
                }
            };
            let text = format!(
                "{}{}{}",
                pad("paddingLeft"),
                label.trim_end_matches('\n'),
                pad("paddingRight")
            );
            Some(crate::editor::VirtualText {
                at: convert::from_lsp(buffer, &hint["position"], encoding),
                text,
                // 1 is a type; 2 a parameter's name.
                style: match hint["kind"].as_u64() {
                    Some(2) => "ui.virtual.inlay-hint.parameter".into(),
                    Some(1) => "ui.virtual.inlay-hint.type".into(),
                    _ => "ui.virtual.inlay-hint".into(),
                },
                source: "inlay".into(),
                end_of_line: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod inlay_tests {
    use serde_json::json;

    #[test]
    fn inlay_hints_become_virtual_text() {
        let buffer = crate::core::Buffer::from_text("let x = f(1);");
        let result = json!([
            { "position": { "line": 0, "character": 5 }, "label": ": i32", "kind": 1 },
            { "position": { "line": 0, "character": 10 }, "label": [{ "value": "n" }, { "value": ":" }],
              "kind": 2, "paddingRight": true },
            { "position": { "line": 0, "character": 1 }, "label": "" },
        ]);
        let hints = super::inlay_hints(&buffer, &result, super::convert::Encoding::Utf16);
        assert_eq!(hints.len(), 2, "empty labels left out");
        assert_eq!((hints[0].at.col, hints[0].text.as_str()), (5, ": i32"));
        assert_eq!(hints[0].style, "ui.virtual.inlay-hint.type");
        assert_eq!((hints[1].at.col, hints[1].text.as_str()), (10, "n: "));
        assert_eq!(hints[1].style, "ui.virtual.inlay-hint.parameter");
    }
}
