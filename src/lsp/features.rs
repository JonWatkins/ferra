//! The editor's requests (hover, go to definition, references): sending
//! them to the server that has the document open, and turning the answers
//! into a hover popup, a jump or a picker.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use super::client::{Client, ResponseError};
use super::convert::{self, Encoding};
use super::{Lsp, State};
use crate::core::Position;
use crate::editor::lsp::{Location, LspFeature, LspRequest};
use crate::editor::server_completion::ServerItem;
use crate::editor::{Editor, Message};

/// Recent completion answers' items, by answer.
#[derive(Default)]
pub(super) struct Stash {
    answers: std::collections::VecDeque<(u64, Vec<Value>)>,
    next: u64,
}

impl Stash {
    /// How many answers are kept: an item can only be taken from one of the
    /// last few.
    const KEPT: usize = 4;

    fn keep(&mut self, items: Vec<Value>) -> u64 {
        self.next += 1;
        if self.answers.len() == Self::KEPT {
            self.answers.pop_front();
        }
        self.answers.push_back((self.next, items));
        self.next
    }

    fn item(&self, (answer, index): (u64, usize)) -> Option<&Value> {
        let (_, items) = self.answers.iter().find(|(id, _)| *id == answer)?;
        items.get(index)
    }
}

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
            feature: LspFeature::Format,
            document,
            position: editor.document.cursor(),
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            manual: true,
            trigger: None,
            word_start: editor.document.cursor(),
            item: None,
            text: None,
        };
        let options = &editor.options;
        let (name, encoding, client_id) = (client.name.clone(), client.encoding, client.id);
        let edit_seq = request.edit_seq;
        let id = client.request_format(request, options.tab_width, options.expand_tab);

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
    pub(super) fn send_requests(&mut self, editor: &mut Editor) {
        for request in editor.take_lsp_requests() {
            let Some(document) = editor.document_by_id(request.document) else {
                continue;
            };
            let language = editor.language_of(document).map(str::to_string);
            let client = self
                .clients
                .iter_mut()
                .find(|c| c.has_document(request.document) && c.state == State::Ready);
            let Some(client) = client else {
                // Completing as you type stays quiet about it.
                if !request.manual {
                    continue;
                }
                let starting = self
                    .clients
                    .iter()
                    .find(|c| c.state == State::Starting && Some(&c.language) == language.as_ref());
                let text = match starting {
                    Some(c) => format!("{} is still starting", c.name),
                    None => "No language server for this buffer (see :lsp)".to_string(),
                };
                editor.set_message(Message::info(text));
                continue;
            };
            if let Some(c) = request.trigger
                && !client.completes_after(c)
            {
                continue;
            }
            if !client.supports(request.feature) {
                if !request.manual {
                    continue;
                }
                editor.set_message(Message::info(format!(
                    "{} doesn't offer {}",
                    client.name,
                    request.feature.noun()
                )));
                continue;
            }
            if request.feature == LspFeature::ResolveCompletion {
                let item = request.item.and_then(|i| self.completions.item(i));
                if let Some(item) = item {
                    client.resolve_completion(request, item.clone());
                }
                continue;
            }
            let position = convert::to_lsp(document.buffer(), request.position, client.encoding);
            client.request_feature(request, position);
        }
    }
}

/// Shows the server's answer to `request`, unless the editor has moved on
/// since (another buffer, an edit, the cursor moved).
pub(super) fn answer(
    editor: &mut Editor,
    client: &Client,
    request: LspRequest,
    result: Result<Value, ResponseError>,
    stash: &mut Stash,
) {
    let current = match request.feature {
        LspFeature::Completion => editor.still_completing(&request),
        // The item was taken: its import goes in if it's the same buffer.
        LspFeature::ResolveCompletion => editor.document.id() == request.document,
        _ => editor.still_at(&request),
    };
    if !current {
        return;
    }
    let result = match result {
        Ok(result) => result,
        Err(error) if error.is_stale() => return,
        Err(error) => {
            editor.lsp_failed(format!("{}: {}", client.name, error.message));
            return;
        }
    };
    if request.feature == LspFeature::Completion {
        let resolves = client.resolves_completions();
        let (items, incomplete, raw) = completion_items(editor, &request, &result, client.encoding);
        // Resolved when taken, for its import: if the server says it
        // resolves items, or the item has `data` for it to (rust-analyzer
        // says it doesn't, but resolves those).
        let needs: Vec<bool> = raw
            .iter()
            .map(|item| resolves || item.get("data").is_some())
            .collect();
        let answer = stash.keep(raw);
        let items = items
            .into_iter()
            .enumerate()
            .map(|(i, mut item)| {
                if needs[i] && item.other_edits.is_empty() {
                    item.resolve = Some((answer, i));
                }
                item
            })
            .collect();
        editor.show_server_completion(&request, items, incomplete);
        return;
    }
    if request.feature == LspFeature::ResolveCompletion {
        let buffer = editor.document.buffer();
        let edits = result["additionalTextEdits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                let start = convert::from_lsp(buffer, &e["range"]["start"], client.encoding);
                let end = convert::from_lsp(buffer, &e["range"]["end"], client.encoding);
                (start, end, e["newText"].as_str().unwrap_or("").to_string())
            })
            .collect();
        editor.make_other_edits(edits);
        return;
    }
    if request.feature == LspFeature::Rename {
        let name = request.text.clone().unwrap_or_default();
        let text = apply_workspace_edit(editor, &result, client.encoding).map(|done| match done {
            Applied { changes: 0, .. } => "Nothing to rename here".to_string(),
            done => format!("Renamed to {name}: {}", done.describe()),
        });
        match text {
            Ok(text) => editor.set_message(Message::info(text)),
            Err(error) => editor.lsp_failed(error),
        }
        return;
    }
    if request.feature == LspFeature::Hover {
        let markdown = hover_markdown(&result["contents"]);
        if markdown.trim().is_empty() {
            editor.set_message(Message::info("No hover information here"));
        } else {
            editor.show_hover(&markdown);
        }
        return;
    }
    let locations = locations(editor, &result, client.encoding);
    if let Err(error) = editor.show_locations(request.feature, locations) {
        editor.set_message(Message::info(error));
    }
}

/// What applying a workspace edit did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Applied {
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

/// Makes a `WorkspaceEdit`'s changes (`changes` by URI, or
/// `documentChanges`), each file's as one undo step, opening files that
/// aren't open in the background. Creating, renaming and deleting files
/// is left out (and counted).
pub(super) fn apply_workspace_edit(
    editor: &mut Editor,
    edit: &Value,
    encoding: Encoding,
) -> Result<Applied, String> {
    let mut files: Vec<(&str, &Vec<Value>)> = Vec::new();
    let mut applied = Applied::default();
    if let Some(changes) = edit["documentChanges"].as_array() {
        for change in changes {
            match (
                change["textDocument"]["uri"].as_str(),
                change["edits"].as_array(),
            ) {
                (Some(uri), Some(edits)) => files.push((uri, edits)),
                _ => applied.skipped += 1,
            }
        }
    } else if let Some(changes) = edit["changes"].as_object() {
        for (uri, edits) in changes {
            if let Some(edits) = edits.as_array() {
                files.push((uri, edits));
            }
        }
    }
    let active = editor.document.id();
    for (uri, edits) in files {
        if edits.is_empty() {
            continue;
        }
        let path = convert::uri_to_path(uri).ok_or_else(|| format!("Can't edit {uri}"))?;
        let id = editor.buffer_for_path(&path)?;
        let buffer = editor.document_by_id(id).expect("just found").buffer();
        let converted = edits
            .iter()
            .map(|e| {
                let start = convert::from_lsp(buffer, &e["range"]["start"], encoding);
                let end = convert::from_lsp(buffer, &e["range"]["end"], encoding);
                (start, end, e["newText"].as_str().unwrap_or("").to_string())
            })
            .collect();
        editor.edit_document(id, converted);
        applied.changes += edits.len();
        applied.files += 1;
        if id != active {
            applied.others += 1;
        }
    }
    Ok(applied)
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
                .map(|r| range(r).0)
                .filter(|s| s.row == request.word_start.row && *s < request.word_start)
                .unwrap_or(request.word_start);
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
    // Files not open in the editor, read once each.
    let mut files: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut found: Vec<Location> = Vec::new();
    for item in items {
        // A `LocationLink` points at the name in `targetSelectionRange`.
        let (uri, start) = match item.get("targetUri") {
            Some(uri) => (uri, &item["targetSelectionRange"]["start"]),
            None => (&item["uri"], &item["range"]["start"]),
        };
        let Some(path) = uri.as_str().and_then(convert::uri_to_path) else {
            continue;
        };
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
        let location = Location {
            path,
            position: Position::new(row, col),
            line: line.trim_end_matches(['\n', '\r']).to_string(),
        };
        if !found.contains(&location) {
            found.push(location);
        }
    }
    found
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
            feature: LspFeature::Completion,
            document: editor.document.id(),
            position: Position::new(0, 4),
            buffer_id: 0,
            edit_seq: 0,
            manual: false,
            trigger: None,
            word_start: Position::new(0, 2),
            item: None,
            text: None,
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
