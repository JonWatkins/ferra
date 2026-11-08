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
use crate::editor::{Editor, Message};

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
            if !client.supports(request.feature) {
                editor.set_message(Message::info(format!(
                    "{} doesn't offer {}",
                    client.name,
                    request.feature.noun()
                )));
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
) {
    if !editor.still_at(&request) {
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
