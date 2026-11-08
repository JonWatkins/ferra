//! Language servers (LSP): starting them for the open files, keeping the
//! files in sync, and showing what they report.
//!
//! Each language can name a server in `[languages]` (`language-server`).
//! Opening a file in that language starts the server for its project (the
//! nearest directory above with one of the language's root files, like
//! `Cargo.toml`), unless one is already running there. Servers aren't
//! installed by ferra: one that isn't on `PATH` is reported once, and `:lsp`
//! lists the servers and how they're doing.
//!
//! Each server runs as its own process; a thread per server reads its
//! messages into a channel, which [`Lsp::update`] drains before each draw,
//! when it also sends the edits made since. Diagnostics go to the editor
//! ([`Editor::set_diagnostics`]).

mod client;
pub mod convert;
pub mod transport;

mod features;

pub use client::{ProcessSpawner, Spawner, State, Streams};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::{Value, json};

use crate::editor::diagnostics::{Diagnostic, Severity};
use crate::editor::lsp::LspFeature;
use crate::editor::{DocumentId, Editor, Message};
use client::{Answer, Client, ClientId, Incoming};

pub struct Lsp {
    clients: Vec<Client>,
    sender: Sender<Incoming>,
    receiver: Receiver<Incoming>,
    spawner: Box<dyn Spawner>,
    next_id: ClientId,
    /// Commands that weren't found, so they're reported (and tried) once.
    missing: HashSet<String>,
    /// Servers that failed to start, by command and root, and why.
    failed: HashMap<(String, PathBuf), String>,
    /// Off until the editor runs in a terminal (or a test sets a spawner),
    /// so tests that open files don't start real servers.
    enabled: bool,
    /// The last few completion answers' items as the servers sent them, to
    /// send one back to be resolved when it's taken.
    completions: features::Stash,
    /// The last few code action lists, to make the one chosen.
    actions: features::Stash,
    /// Each document's diagnostics as its server sent them, to send back
    /// with code action requests.
    raw_diagnostics: HashMap<DocumentId, Vec<Value>>,
    /// The answer to the formatting request being waited for.
    formatted: Option<Result<Value, client::ResponseError>>,
}

impl Default for Lsp {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            clients: Vec::new(),
            sender,
            receiver,
            spawner: Box::new(ProcessSpawner),
            next_id: 0,
            missing: HashSet::new(),
            failed: HashMap::new(),
            enabled: false,
            completions: features::Stash::default(),
            actions: features::Stash::default(),
            raw_diagnostics: HashMap::new(),
            formatted: None,
        }
    }
}

/// The editor's way to `:format`: the app's language servers.
pub struct LspFormatter(std::rc::Rc<std::cell::RefCell<Lsp>>);

impl LspFormatter {
    pub fn new(lsp: std::rc::Rc<std::cell::RefCell<Lsp>>) -> Self {
        Self(lsp)
    }
}

impl std::fmt::Debug for LspFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LspFormatter")
    }
}

impl crate::editor::lsp::Formatter for LspFormatter {
    fn format(&mut self, editor: &mut Editor, quiet: bool) -> Result<(), String> {
        // Only ever called from a command, never while drawing, so the
        // app isn't using it.
        self.0.borrow_mut().format(editor, quiet)
    }
}

impl Lsp {
    /// Starts servers with `spawner` instead of running their commands
    /// (for tests).
    pub fn set_spawner(&mut self, spawner: Box<dyn Spawner>) {
        self.spawner = spawner;
        self.enabled = true;
    }

    /// Lets servers start (the event loop calls this).
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Whether any server is running, so the event loop should keep
    /// checking for their messages.
    pub fn active(&self) -> bool {
        self.clients
            .iter()
            .any(|c| !matches!(c.state, State::Stopped(_)))
    }

    /// Handles what servers sent, then starts servers for newly opened
    /// files and sends the changes made since last time.
    /// Returns whether anything happened that could change what's shown
    /// (a message handled, a request sent, a server's state), so the event
    /// loop only redraws then.
    pub fn update(&mut self, editor: &mut Editor) -> bool {
        if !self.enabled || !editor.options.lsp {
            return false;
        }
        let handled = self.receive(editor);
        self.sync(editor);
        let asked = self.send_requests(editor);
        let status = self.status();
        let changed = handled > 0 || asked || status != editor.lsp_status();
        if status != editor.lsp_status() {
            editor.set_lsp_status(status);
        }
        changed
    }

    /// One line per server, for `:lsp`.
    fn status(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .clients
            .iter()
            .map(|c| {
                let state = match &c.state {
                    State::Starting => "starting".to_string(),
                    State::Ready => "running".to_string(),
                    State::Stopped(why) => format!("stopped: {why}"),
                };
                format!(
                    "{} ({}): {state} in {}",
                    c.name,
                    c.language,
                    c.root.display()
                )
            })
            .collect();
        let mut missing: Vec<&String> = self.missing.iter().collect();
        missing.sort();
        let any_missing = !missing.is_empty();
        lines.extend(
            missing
                .into_iter()
                .map(|m| format!("{m}: not found on PATH")),
        );
        // The PATH ferra was started with, which can differ from a shell's.
        if any_missing {
            let path = std::env::var("PATH").unwrap_or_default();
            lines.push(format!("PATH={path}"));
        }
        let mut failed: Vec<_> = self.failed.iter().collect();
        failed.sort();
        lines.extend(failed.into_iter().map(|((command, root), why)| {
            format!("{command}: failed in {}: {why}", root.display())
        }));
        lines
    }

    /// Handles what servers sent; returns how many messages there were.
    fn receive(&mut self, editor: &mut Editor) -> usize {
        let mut handled = 0;
        while let Ok(incoming) = self.receiver.try_recv() {
            handled += 1;
            match incoming {
                Incoming::Message(id, message) => self.handle(id, message, editor),
                Incoming::Closed(id) => self.closed(id, editor),
            }
        }
        handled
    }

    /// Server `id`'s output ended: it's stopped, and its diagnostics go.
    fn closed(&mut self, id: ClientId, editor: &mut Editor) {
        let Some(client) = self.clients.iter_mut().find(|c| c.id == id) else {
            return;
        };
        if !matches!(client.state, State::Stopped(_)) {
            editor.set_message(Message::error(format!(
                "{} stopped (see :lsp)",
                client.name
            )));
            client.state = State::Stopped("it exited".into());
        }
        let documents: Vec<DocumentId> = client.document_uris().map(|(d, _)| d).collect();
        for document in documents {
            editor.set_diagnostics(document, Vec::new());
        }
    }

    fn handle(&mut self, id: ClientId, message: Value, editor: &mut Editor) {
        let Some(client) = self.clients.iter_mut().find(|c| c.id == id) else {
            return;
        };
        let method = message["method"].as_str();
        match (method, message.get("id")) {
            // An answer to one of our requests.
            (None, Some(request)) => {
                let Some(request) = request.as_i64() else {
                    return;
                };
                match client.handle_response(request, &message) {
                    Answer::Done => {}
                    Answer::Failed(error) => {
                        editor.set_message(Message::error(format!("{}: {error}", client.name)));
                    }
                    Answer::Feature(request, result) if request.feature == LspFeature::Format => {
                        self.formatted = Some(result);
                    }
                    Answer::Feature(request, result) => {
                        let stashes = features::Stashes {
                            completions: &mut self.completions,
                            actions: &mut self.actions,
                        };
                        features::answer(editor, client, request, result, stashes);
                    }
                }
            }
            // A request from the server: answered, so it doesn't wait.
            (Some(method), Some(request)) => {
                let result = match method {
                    // Each section asked for, from the config's settings, or
                    // `{}` (the vscode JSON and CSS servers do nothing with
                    // `null`).
                    "workspace/configuration" => Value::Array(
                        message["params"]["items"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|item| client.settings_section(item["section"].as_str()))
                            .collect(),
                    ),
                    "workspace/workspaceFolders" => json!([{
                        "uri": convert::path_to_uri(&client.root),
                        "name": client.root.display().to_string(),
                    }]),
                    // A command it ran (a code action's) wants edits made.
                    "workspace/applyEdit" => {
                        let params = &message["params"];
                        match features::apply_workspace_edit(
                            editor,
                            &params["edit"],
                            client.encoding,
                        ) {
                            Ok(applied) => {
                                let label = params["label"]
                                    .as_str()
                                    .or(client.command_title.as_deref())
                                    .unwrap_or("Changed");
                                editor.set_message(Message::info(format!(
                                    "{label}: {}",
                                    applied.describe()
                                )));
                                json!({ "applied": true })
                            }
                            Err(error) => json!({ "applied": false, "failureReason": error }),
                        }
                    }
                    _ => Value::Null,
                };
                client.respond(request.clone(), result);
            }
            (Some("textDocument/publishDiagnostics"), None) => {
                let params = &message["params"];
                let Some(uri) = params["uri"].as_str() else {
                    return;
                };
                // The document it was opened as, else (a server that writes
                // URIs its own way) whichever is at that path.
                let by_uri = client
                    .document_uris()
                    .find(|(_, open)| *open == uri)
                    .map(|(d, _)| d);
                let Some(document) = by_uri.or_else(|| {
                    convert::uri_to_path(uri).and_then(|path| document_at(editor, &path))
                }) else {
                    return;
                };
                // A list for text since changed would put back places the
                // edits moved diagnostics from; a newer one is coming.
                let sent = client.document_version(document);
                let for_version = params["version"].as_i64();
                if let (Some(sent), Some(for_version)) = (sent, for_version)
                    && for_version < sent
                {
                    return;
                }
                let encoding = client.encoding;
                let Some(doc) = editor.document_by_id(document) else {
                    return;
                };
                let buffer = doc.buffer();
                let diagnostics = params["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|d| Diagnostic {
                        start: convert::from_lsp(buffer, &d["range"]["start"], encoding),
                        end: convert::from_lsp(buffer, &d["range"]["end"], encoding),
                        severity: Severity::from_lsp(d["severity"].as_u64()),
                        message: d["message"].as_str().unwrap_or_default().to_string(),
                        source: d["source"].as_str().map(str::to_string),
                    })
                    .collect();
                editor.set_diagnostics(document, diagnostics);
                let raw = params["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                self.raw_diagnostics.insert(document, raw);
            }
            (Some("window/showMessage"), None) => {
                let params = &message["params"];
                let text = format!(
                    "{}: {}",
                    client.name,
                    params["message"].as_str().unwrap_or_default()
                );
                if params["type"].as_u64() == Some(1) {
                    editor.set_message(Message::error(text));
                } else if params["type"].as_u64() == Some(2) {
                    editor.set_message(Message::info(text));
                }
            }
            _ => {}
        }
    }

    fn sync(&mut self, editor: &mut Editor) {
        // What each open file with a server should be open in.
        let mut wanted: Vec<(DocumentId, usize)> = Vec::new();
        let ids: Vec<DocumentId> = editor.buffers().iter().map(|b| b.id).collect();
        for document_id in ids {
            let Some(document) = editor.document_by_id(document_id) else {
                continue;
            };
            let Some(path) = document.path().map(Path::to_path_buf) else {
                continue;
            };
            let Some(language) = editor.language_of(document).map(str::to_string) else {
                continue;
            };
            let Some(config) = editor.languages.server(&language).cloned() else {
                continue;
            };
            if self.missing.contains(&config.command) {
                continue;
            }
            let root = find_root(&path, &config.roots);
            let key = (config.command.clone(), root.clone());
            if self.failed.contains_key(&key) {
                continue;
            }
            let index = match self
                .clients
                .iter()
                .position(|c| c.name == config.command && c.root == root)
            {
                Some(index) => index,
                None => {
                    let id = self.next_id;
                    self.next_id += 1;
                    match Client::start(
                        id,
                        &language,
                        &config,
                        &root,
                        self.spawner.as_ref(),
                        self.sender.clone(),
                    ) {
                        Ok(client) => {
                            self.clients.push(client);
                            self.clients.len() - 1
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            editor.set_message(Message::info(format!(
                                "{} not found: install it for {language} language features (see :lsp)",
                                config.command
                            )));
                            self.missing.insert(config.command.clone());
                            continue;
                        }
                        Err(error) => {
                            editor.set_message(Message::error(format!(
                                "{} didn't start: {error}",
                                config.command
                            )));
                            self.failed.insert(key, error.to_string());
                            continue;
                        }
                    }
                }
            };
            wanted.push((document_id, index));
            let client = &mut self.clients[index];
            if client.state != State::Ready {
                continue;
            }
            let document = editor.document_by_id(document_id).expect("listed");
            // Renamed (`:saveas`): reopened under its new name.
            let uri = convert::path_to_uri(&path);
            if client
                .document_uri(document_id)
                .is_some_and(|open| open != uri)
            {
                client.close(document_id);
            }
            if client.has_document(document_id) {
                client.change(document);
                client.saved(document);
            } else {
                let language_id = config.language_id.clone().unwrap_or(language);
                client.open(document, &path, &language_id);
            }
        }
        // Close what's no longer open (or moved to another server).
        for (index, client) in self.clients.iter_mut().enumerate() {
            let stale: Vec<DocumentId> = client
                .document_uris()
                .map(|(d, _)| d)
                .filter(|d| !wanted.contains(&(*d, index)))
                .collect();
            for document in stale {
                client.close(document);
                editor.set_diagnostics(document, Vec::new());
            }
        }
    }

    /// Asks every server to shut down (when quitting).
    pub fn shutdown(&mut self) {
        for client in &mut self.clients {
            client.shutdown();
        }
        // Give them a moment to exit cleanly before they're killed.
        let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
        while std::time::Instant::now() < until && !self.clients.is_empty() {
            match self
                .receiver
                .recv_timeout(std::time::Duration::from_millis(20))
            {
                Ok(Incoming::Message(id, message)) => {
                    if let (None, Some(request)) =
                        (message["method"].as_str(), message["id"].as_i64())
                        && let Some(client) = self.clients.iter_mut().find(|c| c.id == id)
                    {
                        let _ = client.handle_response(request, &message);
                    }
                }
                Ok(Incoming::Closed(id)) => self.clients.retain(|c| c.id != id),
                Err(_) => {}
            }
        }
        self.clients.clear();
    }
}

/// The open document at `path`, if any.
fn document_at(editor: &Editor, path: &Path) -> Option<DocumentId> {
    let wanted = absolute(path);
    editor.buffers().iter().map(|b| b.id).find(|&id| {
        editor
            .document_by_id(id)
            .and_then(|d| d.path())
            .is_some_and(|p| absolute(p) == wanted)
    })
}

/// `path` canonicalized, or for one that isn't there yet, made absolute.
fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

/// The project root for the file at `path`: the nearest directory above it
/// holding one of `markers`, else the nearest with `.git`, else its own.
pub fn find_root(path: &Path, markers: &[String]) -> PathBuf {
    // A new file (`:e new.rs`) isn't there to canonicalize: its nearest
    // folder that is, made absolute.
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let dir = absolute
        .ancestors()
        .skip(1)
        .find_map(|d| std::fs::canonicalize(d).ok())
        .unwrap_or_else(|| absolute.parent().unwrap_or(&absolute).to_path_buf());
    let with = |names: &[&str]| {
        dir.ancestors()
            .find(|d| names.iter().any(|m| d.join(m).exists()))
            .map(Path::to_path_buf)
    };
    let markers: Vec<&str> = markers.iter().map(String::as_str).collect();
    with(&markers).or_else(|| with(&[".git"])).unwrap_or(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_the_nearest_marked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("crate/src/deep")).unwrap();
        std::fs::write(root.join("crate/Cargo.toml"), "").unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let file = root.join("crate/src/deep/a.rs");
        assert_eq!(find_root(&file, &["Cargo.toml".into()]), root.join("crate"));
        assert_eq!(
            find_root(&file, &["nothing".into()]),
            root,
            "else the repository"
        );
        let loose = tempfile::tempdir().unwrap();
        let loose = std::fs::canonicalize(loose.path()).unwrap();
        assert_eq!(
            find_root(&loose.join("x.py"), &[]),
            loose,
            "else its own directory"
        );
        assert_eq!(
            find_root(&root.join("crate/new/dirs/b.rs"), &["Cargo.toml".into()]),
            root.join("crate"),
            "a new file in new folders: from the nearest folder there is"
        );
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(
            find_root(Path::new("not-yet.rs"), &["Cargo.toml".into()]),
            here,
            "a new file named relatively (`:e not-yet.rs`)"
        );
    }
}
