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

impl Lsp {
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
            let edit_range = edit.get("range").or_else(|| edit.get("replace"));
            if let Some(edit_range) = edit_range {
                let (start, _) = range(edit_range);
                if start < request.word_start && start.row == request.word_start.row {
                    let before = buffer.text_range(start, request.word_start);
                    if let Some(rest) = text.strip_prefix(&before) {
                        text = rest.to_string();
                    }
                }
            }
            if item["insertTextFormat"].as_u64() == Some(2) {
                text = strip_snippet(&text);
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

/// A snippet (`area(${1:r})$0`) as plain text: its placeholders' text,
/// without the tab stops.
fn strip_snippet(snippet: &str) -> String {
    let mut out = String::new();
    let mut chars = snippet.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
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
                let inner = if let Some(rest) = inner.strip_prefix(':') {
                    strip_snippet(rest)
                } else if let Some(choices) = inner.strip_prefix('|') {
                    choices.split([',', '|']).next().unwrap_or("").to_string()
                } else {
                    String::new()
                };
                out.push_str(&inner);
            }
            _ => out.push(c),
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
    fn snippets_become_plain_text() {
        assert_eq!(strip_snippet("area(${1:r}, ${2:s})$0"), "area(r, s)");
        assert_eq!(strip_snippet("if ${1:x} {\n\t$0\n}"), "if x {\n\t\n}");
        assert_eq!(strip_snippet("${1:outer ${2:inner}}"), "outer inner");
        assert_eq!(strip_snippet("${1|one,two|}"), "one");
        assert_eq!(strip_snippet("cost \\$5"), "cost $5");
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
        };
        let range = |from: u64| {
            json!({"start": {"line": 0, "character": from},
                                       "end": {"line": 0, "character": 4}})
        };
        let result = json!({"isIncomplete": true, "items": [
            // An edit from the `.`: what goes in is from the word on.
            {"label": "area", "textEdit": {"range": range(1), "newText": ".area"},
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
            .map(|i| (i.text.as_str(), i.detail.as_str(), i.filter.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("area", "(&self) -> f64", "area"),
                ("arity", "usize", "ar_ity"),
                ("Argh", "struct", "Argh"),
            ]
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
