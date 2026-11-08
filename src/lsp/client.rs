//! One running language server: its process, the requests waiting for an
//! answer, and the documents it has open.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use serde_json::{Value, json};

use super::convert::{Encoding, path_to_uri};
use super::transport::{read_message, write_message};
use crate::config::ServerConfig;
use crate::editor::lsp::{LspFeature, LspRequest};
use crate::editor::{Document, DocumentId};

/// Identifies a client among the running ones.
pub type ClientId = usize;

/// What a client's reader thread passes on.
pub enum Incoming {
    Message(ClientId, Value),
    /// The server's output ended (it exited or crashed).
    Closed(ClientId),
}

/// A server's input and output, and its process if it has one.
pub struct Streams {
    pub reader: Box<dyn BufRead + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Option<Child>,
}

/// Starts language servers. Tests replace it with one that talks to a fake
/// server in the same process.
pub trait Spawner {
    fn spawn(&self, config: &ServerConfig, root: &Path) -> io::Result<Streams>;
}

/// Runs the server's command, in the project root, talking over its stdin
/// and stdout (its stderr is dropped).
pub struct ProcessSpawner;

impl Spawner for ProcessSpawner {
    fn spawn(&self, config: &ServerConfig, root: &Path) -> io::Result<Streams> {
        // Found first, so that `NotFound` means just that: other failures
        // (a root that isn't there) say what they are.
        let program = find_program(&config.command, std::env::var_os("PATH").as_deref())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not on PATH"))?;
        if !root.is_dir() {
            return Err(io::Error::other(format!(
                "its root {} isn't a directory",
                root.display()
            )));
        }
        let mut child = Command::new(program)
            .args(&config.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped");
        let stdin = child.stdin.take().expect("piped");
        Ok(Streams {
            reader: Box::new(BufReader::new(stdout)),
            writer: Box::new(stdin),
            child: Some(child),
        })
    }
}

/// Where `command` is: itself if it has a `/`, else the first executable
/// file of that name in `path`'s directories.
pub fn find_program(command: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let runnable = |p: &Path| {
        p.metadata().is_ok_and(|m| {
            #[cfg(unix)]
            let executable = std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0;
            #[cfg(not(unix))]
            let executable = true;
            m.is_file() && executable
        })
    };
    if command.contains('/') {
        let p = PathBuf::from(command);
        return runnable(&p).then_some(p);
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(command))
        .find(|p| runnable(p))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Waiting for the answer to `initialize`.
    Starting,
    Ready,
    /// It stopped, or never started properly; the reason.
    Stopped(String),
}

/// How the server wants document changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sync {
    None,
    Full,
    Incremental,
}

/// A document the server has open.
struct Synced {
    uri: String,
    saves: u64,
    version: i64,
    buffer_id: u64,
    edit_seq: u64,
}

/// What a request was, so its answer can be handled.
enum Pending {
    Initialize,
    Shutdown,
    Feature(LspRequest),
    /// `workspace/executeCommand`: only an error is worth showing.
    Command,
}

/// An error a server answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

impl ResponseError {
    /// Whether it only means the answer is out of date (the text changed,
    /// or the request was cancelled), so it's not worth reporting.
    pub fn is_stale(&self) -> bool {
        matches!(self.code, -32802..=-32800)
    }
}

/// What an answer from the server was for.
pub enum Answer {
    /// Nothing more to do.
    Done,
    /// `initialize` failed: why.
    Failed(String),
    /// The answer to an editor request.
    Feature(LspRequest, Result<Value, ResponseError>),
}

/// The method that asks for `feature`, and the capability that says the
/// server can answer it.
fn feature_method(feature: LspFeature) -> (&'static str, &'static str) {
    match feature {
        LspFeature::Hover => ("textDocument/hover", "hoverProvider"),
        LspFeature::Definition => ("textDocument/definition", "definitionProvider"),
        LspFeature::Declaration => ("textDocument/declaration", "declarationProvider"),
        LspFeature::TypeDefinition => ("textDocument/typeDefinition", "typeDefinitionProvider"),
        LspFeature::Implementation => ("textDocument/implementation", "implementationProvider"),
        LspFeature::References => ("textDocument/references", "referencesProvider"),
        LspFeature::Completion => ("textDocument/completion", "completionProvider"),
        LspFeature::ResolveCompletion => ("completionItem/resolve", "completionProvider"),
        LspFeature::Format => ("textDocument/formatting", "documentFormattingProvider"),
        LspFeature::Rename => ("textDocument/rename", "renameProvider"),
        LspFeature::CodeAction | LspFeature::ApplyCodeAction => {
            ("textDocument/codeAction", "codeActionProvider")
        }
        LspFeature::ResolveCodeAction => ("codeAction/resolve", "codeActionProvider"),
    }
}

pub struct Client {
    pub id: ClientId,
    /// The command, which names it in messages (`rust-analyzer`).
    pub name: String,
    pub language: String,
    pub root: PathBuf,
    pub state: State,
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
    next_request: i64,
    pending: HashMap<i64, Pending>,
    pub encoding: Encoding,
    sync: Sync,
    documents: HashMap<DocumentId, Synced>,
    /// The settings from config, as JSON.
    settings: Value,
    /// What it said it can do, in its answer to `initialize`.
    capabilities: Value,
    /// The title of the last command it was asked to run (a code
    /// action's), for the message when it sends edits back.
    pub command_title: Option<String>,
}

impl Client {
    /// Starts the server for `language` in `root` and sends `initialize`;
    /// its messages go to `sender`.
    pub fn start(
        id: ClientId,
        language: &str,
        config: &ServerConfig,
        root: &Path,
        spawner: &dyn Spawner,
        sender: Sender<Incoming>,
    ) -> io::Result<Client> {
        let Streams {
            mut reader,
            writer,
            child,
        } = spawner.spawn(config, root)?;
        thread::spawn(move || {
            while let Ok(Some(message)) = read_message(&mut reader) {
                if sender.send(Incoming::Message(id, message)).is_err() {
                    return;
                }
            }
            let _ = sender.send(Incoming::Closed(id));
        });
        let mut client = Client {
            id,
            name: config.command.clone(),
            language: language.to_string(),
            root: root.to_path_buf(),
            state: State::Starting,
            writer,
            child,
            next_request: 1,
            pending: HashMap::new(),
            encoding: Encoding::Utf16,
            sync: Sync::Full,
            documents: HashMap::new(),
            settings: serde_json::to_value(&config.settings).unwrap_or_else(|_| json!({})),
            capabilities: Value::Null,
            command_title: None,
        };
        let root_uri = path_to_uri(root);
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "clientInfo": { "name": "ferra", "version": env!("CARGO_PKG_VERSION") },
                "rootUri": root_uri,
                "rootPath": root.display().to_string(),
                "workspaceFolders": [{ "uri": root_uri, "name": name }],
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] },
                    "textDocument": {
                        "synchronization": { "dynamicRegistration": false, "didSave": true },
                        "publishDiagnostics": { "relatedInformation": false },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "definition": { "linkSupport": true },
                        "declaration": { "linkSupport": true },
                        "typeDefinition": { "linkSupport": true },
                        "implementation": { "linkSupport": true },
                        "references": {},
                        "rename": { "prepareSupport": false },
                        "codeAction": {
                            "codeActionLiteralSupport": {
                                "codeActionKind": { "valueSet": [
                                    "", "quickfix", "refactor", "refactor.extract",
                                    "refactor.inline", "refactor.rewrite", "source",
                                    "source.organizeImports", "source.fixAll",
                                ]},
                            },
                            "isPreferredSupport": true,
                            "disabledSupport": true,
                            "dataSupport": true,
                            "resolveSupport": { "properties": ["edit", "command"] },
                        },
                        // Snippets go in as their text, with the cursor at
                        // the first tab stop (the CSS and HTML servers
                        // complete nothing without them).
                        "completion": {
                            // clangd's: edits before the word (`.` to `->`).
                            "editsNearCursor": true,
                            "completionItem": {
                                "snippetSupport": true,
                                "labelDetailsSupport": true,
                                "documentationFormat": ["markdown", "plaintext"],
                                // Imports can come once an item is taken
                                // (rust-analyzer only offers items needing
                                // one when they can).
                                "resolveSupport": { "properties": ["additionalTextEdits"] },
                            },
                            "contextSupport": true,
                        },
                    },
                    "workspace": {
                        "workspaceFolders": true,
                        "configuration": true,
                        // Edits by file, with the version they're for; no
                        // creating, renaming or deleting files.
                        "workspaceEdit": { "documentChanges": true },
                        // Servers' commands can ask ferra to make edits.
                        "applyEdit": true,
                        "executeCommand": { "dynamicRegistration": false },
                    },
                    "window": { "workDoneProgress": false },
                },
            }),
            Pending::Initialize,
        );
        Ok(client)
    }

    fn send(&mut self, message: Value) {
        if write_message(&mut self.writer, &message).is_err() {
            self.state = State::Stopped("it stopped reading its input".into());
        }
    }

    fn request(&mut self, method: &str, params: Value, kind: Pending) -> i64 {
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, kind);
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    /// Whether request `id` is still waiting for its answer.
    pub fn waiting_for(&self, id: i64) -> bool {
        self.pending.contains_key(&id)
    }

    /// Asks the server to format `request`'s document, indenting with
    /// `tab_size` columns, in spaces or tabs. Returns the request's id.
    pub fn request_format(&mut self, request: LspRequest, tab_size: usize, spaces: bool) -> i64 {
        let uri = self
            .documents
            .get(&request.document)
            .map(|s| s.uri.clone())
            .unwrap_or_default();
        let params = json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": tab_size, "insertSpaces": spaces },
        });
        self.request("textDocument/formatting", params, Pending::Feature(request))
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// The settings in `section` (`python.analysis`), or `{}`.
    pub fn settings_section(&self, section: Option<&str>) -> Value {
        let mut value = &self.settings;
        for part in section.into_iter().flat_map(|s| s.split('.')) {
            match value.get(part) {
                Some(inner) => value = inner,
                None => return json!({}),
            }
        }
        value.clone()
    }

    /// Answers a request from the server.
    pub fn respond(&mut self, id: Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    /// Whether the server said it can answer `feature`.
    pub fn supports(&self, feature: LspFeature) -> bool {
        let (_, capability) = feature_method(feature);
        !matches!(
            self.capabilities.get(capability),
            None | Some(Value::Null | Value::Bool(false))
        )
    }

    /// Whether the server fills in completion items' details on asking.
    pub fn resolves_completions(&self) -> bool {
        self.capabilities["completionProvider"]["resolveProvider"].as_bool() == Some(true)
    }

    /// Asks for the code actions for `request`'s range (`start`..`end` as
    /// LSP positions), with the diagnostics there they might fix.
    pub fn request_code_actions(
        &mut self,
        request: LspRequest,
        start: Value,
        end: Value,
        diagnostics: Vec<Value>,
    ) {
        let Some(synced) = self.documents.get(&request.document) else {
            return;
        };
        let params = json!({
            "textDocument": { "uri": synced.uri },
            "range": { "start": start, "end": end },
            "context": { "diagnostics": diagnostics, "triggerKind": 1 },
        });
        self.request("textDocument/codeAction", params, Pending::Feature(request));
    }

    /// Whether the server fills in code actions' edits on asking.
    pub fn resolves_code_actions(&self) -> bool {
        self.capabilities["codeActionProvider"]["resolveProvider"].as_bool() == Some(true)
    }

    /// Asks for the rest of code action `action` (as the server sent it).
    pub fn resolve_code_action(&mut self, request: LspRequest, action: Value) {
        self.request("codeAction/resolve", action, Pending::Feature(request));
    }

    /// Has the server run `command` (a `Command`: its name and arguments),
    /// for the action titled `title`.
    pub fn execute_command(&mut self, command: &Value, title: &str) {
        self.command_title = Some(title.to_string());
        let params = json!({
            "command": command["command"],
            "arguments": command.get("arguments").cloned().unwrap_or(json!([])),
        });
        self.request("workspace/executeCommand", params, Pending::Command);
    }

    /// Asks for the rest of completion `item` (as the server sent it).
    pub fn resolve_completion(&mut self, request: LspRequest, item: Value) {
        self.request("completionItem/resolve", item, Pending::Feature(request));
    }

    /// Whether typing `c` should start completing (`.` in most languages).
    pub fn completes_after(&self, c: char) -> bool {
        self.capabilities["completionProvider"]["triggerCharacters"]
            .as_array()
            .is_some_and(|chars| {
                chars
                    .iter()
                    .any(|t| t.as_str() == Some(c.encode_utf8(&mut [0; 4])))
            })
    }

    /// Asks for `request`'s feature in the document it's in (which the
    /// server has open), at LSP position `position`.
    pub fn request_feature(&mut self, request: LspRequest, position: Value) {
        let Some(synced) = self.documents.get(&request.document) else {
            return;
        };
        let (method, _) = feature_method(request.feature);
        let mut params = json!({
            "textDocument": { "uri": synced.uri },
            "position": position,
        });
        match request.feature {
            LspFeature::References => {
                params["context"] = json!({ "includeDeclaration": true });
            }
            LspFeature::Rename => {
                params["newName"] = json!(request.text.clone().unwrap_or_default());
            }
            LspFeature::Completion => {
                params["context"] = match request.trigger {
                    Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
                    None => json!({ "triggerKind": 1 }),
                };
            }
            _ => {}
        }
        self.request(method, params, Pending::Feature(request));
    }

    /// Handles the answer to one of our requests.
    pub fn handle_response(&mut self, id: i64, message: &Value) -> Answer {
        match self.pending.remove(&id) {
            Some(Pending::Initialize) => {
                if let Some(error) = message.get("error") {
                    let text = error["message"].as_str().unwrap_or("initialize failed");
                    self.state = State::Stopped(text.to_string());
                    return Answer::Failed(text.to_string());
                }
                self.capabilities = message["result"]["capabilities"].clone();
                let capabilities = &message["result"]["capabilities"];
                self.encoding = Encoding::from_name(capabilities["positionEncoding"].as_str());
                let change = match &capabilities["textDocumentSync"] {
                    Value::Number(n) => n.as_u64(),
                    Value::Object(o) => o.get("change").and_then(Value::as_u64),
                    _ => None,
                };
                self.sync = match change {
                    Some(0) => Sync::None,
                    Some(2) => Sync::Incremental,
                    _ => Sync::Full,
                };
                self.state = State::Ready;
                self.notify("initialized", json!({}));
                // Some servers (pyright) do nothing until told their
                // settings changed; they then ask for them.
                let settings = self.settings.clone();
                self.notify(
                    "workspace/didChangeConfiguration",
                    json!({ "settings": settings }),
                );
                Answer::Done
            }
            Some(Pending::Shutdown) => {
                self.notify("exit", Value::Null);
                Answer::Done
            }
            Some(Pending::Command) => match message.get("error") {
                Some(error) => Answer::Failed(error["message"].as_str().unwrap_or("failed").into()),
                None => Answer::Done,
            },
            Some(Pending::Feature(request)) => {
                let result = match message.get("error") {
                    Some(error) => Err(ResponseError {
                        code: error["code"].as_i64().unwrap_or(0),
                        message: error["message"].as_str().unwrap_or("failed").to_string(),
                    }),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                Answer::Feature(request, result)
            }
            None => Answer::Done,
        }
    }

    /// The version of `id` the server last had sent.
    pub fn document_version(&self, id: DocumentId) -> Option<i64> {
        self.documents.get(&id).map(|s| s.version)
    }

    /// Whether the server has `id` open.
    pub fn has_document(&self, id: DocumentId) -> bool {
        self.documents.contains_key(&id)
    }

    /// The URI `id` is open as, if it is.
    pub fn document_uri(&self, id: DocumentId) -> Option<&str> {
        self.documents.get(&id).map(|s| s.uri.as_str())
    }

    /// Tells the server `document` was saved, if it was since it was last
    /// told (rust-analyzer runs `cargo check` then). Its changes go first.
    pub fn saved(&mut self, document: &Document) {
        let Some(synced) = self.documents.get_mut(&document.id()) else {
            return;
        };
        if synced.saves == document.saves() {
            return;
        }
        synced.saves = document.saves();
        let uri = synced.uri.clone();
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": uri } }),
        );
    }

    /// The URIs of the documents it has open, by document.
    pub fn document_uris(&self) -> impl Iterator<Item = (DocumentId, &str)> {
        self.documents.iter().map(|(id, s)| (*id, s.uri.as_str()))
    }

    /// Opens `document` (at `path`, in the language the server calls
    /// `language_id`) on the server.
    pub fn open(&mut self, document: &Document, path: &Path, language_id: &str) {
        let uri = path_to_uri(path);
        let buffer = document.buffer();
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": buffer.text(),
            }}),
        );
        self.documents.insert(
            document.id(),
            Synced {
                uri,
                saves: document.saves(),
                version: 1,
                buffer_id: buffer.id(),
                edit_seq: buffer.edit_seq(),
            },
        );
    }

    /// Sends `document`'s changes since it was last sent: just the edits
    /// (from the buffer's edit log) when the server takes them in bytes,
    /// otherwise the whole text.
    pub fn change(&mut self, document: &Document) {
        let Some(synced) = self.documents.get(&document.id()) else {
            return;
        };
        let buffer = document.buffer();
        if synced.buffer_id == buffer.id() && synced.edit_seq == buffer.edit_seq() {
            return;
        }
        let edits = (synced.buffer_id == buffer.id())
            .then(|| buffer.edits_since(synced.edit_seq))
            .flatten();
        let changes = match (self.sync, self.encoding, edits) {
            (Sync::None, ..) => None,
            // One edit since the last sync, in a server that counts bytes:
            // just that edit. Its range is in the text before it, and what
            // it put in is in the text now. (With several, only the last
            // one's text is still where it was put, so the whole text goes.)
            (Sync::Incremental, Encoding::Utf8, Some([edit])) => {
                let inserted = buffer.byte_text(edit.start_byte..edit.new_end_byte);
                Some(vec![json!({
                    "range": {
                        "start": { "line": edit.start.0, "character": edit.start.1 },
                        "end": { "line": edit.old_end.0, "character": edit.old_end.1 },
                    },
                    "text": inserted,
                })])
            }
            _ => Some(vec![json!({ "text": buffer.text() })]),
        };
        let synced = self.documents.get_mut(&document.id()).expect("checked");
        synced.version += 1;
        synced.buffer_id = buffer.id();
        synced.edit_seq = buffer.edit_seq();
        let (uri, version) = (synced.uri.clone(), synced.version);
        if let Some(changes) = changes {
            self.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": changes,
                }),
            );
        }
    }

    /// Closes `id` on the server.
    pub fn close(&mut self, id: DocumentId) {
        if let Some(synced) = self.documents.remove(&id) {
            self.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": synced.uri } }),
            );
        }
    }

    /// Asks the server to shut down; it exits once it answers.
    pub fn shutdown(&mut self) {
        if self.state == State::Ready {
            self.request("shutdown", Value::Null, Pending::Shutdown);
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn programs_are_found_on_the_path_if_they_can_run() {
        use std::os::unix::fs::PermissionsExt;
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let write = |dir: &Path, mode| {
            let p = dir.join("ls-x");
            std::fs::write(&p, "").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            p
        };
        write(a.path(), 0o644);
        let runnable = write(b.path(), 0o755);
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(
            find_program("ls-x", Some(&path)),
            Some(runnable.clone()),
            "skips one that can't run"
        );
        assert_eq!(find_program("nope", Some(&path)), None);
        assert_eq!(find_program("ls-x", None), None);
        let full = runnable.display().to_string();
        assert_eq!(
            find_program(&full, None),
            Some(runnable),
            "a path is used as is"
        );
    }
}
