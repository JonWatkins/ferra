//! The agent's tool calls, each from being asked to being answered
//! ([`ToolCalls`]).
//!
//! A call is answered one of three ways:
//! - at once (`editor_state`, `read_buffer`, a bad argument);
//! - when a language server answers (`lsp_*`, `move_file`), which comes
//!   back through the editor (`Editor::take_agent_lsp_answers`);
//! - after it changed files, once their diagnostics are settled
//!   (`diagnostics_wait.rs`), so the answer can say what's wrong in them.
//!
//! A server that never answers doesn't leave the agent hanging: after
//! [`ANSWER_WAIT`] the call is given up (`Editor::agent_give_up`), so
//! whatever comes for it later, edits included, is dropped, and the agent
//! is told nothing changed.
//!
//! Generic over who gets each answer (`R`): MCP's connection and request
//! id for `Agents`, anything for tests. Given the time, so tests walk a
//! made-up timeline.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::diagnostics_wait::{DiagnosticsWait, Settled};
use super::tools::{self, Called};
use crate::editor::diagnostics::DocDiagnostics;
use crate::editor::{DocumentId, Editor};

/// How long a call waits for its language server's answer, at most:
/// time for a server to start (`lsp/features.rs`'s 30s) and then answer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(60);

/// A tool call's answer: its text, or an error.
pub type Answer = Result<String, String>;

/// The agent's tool calls still to be answered.
pub struct ToolCalls<R> {
    next: u64,
    /// Waiting for a language server, by call.
    asked: HashMap<u64, Asked<R>>,
    /// Changed files, waiting for their diagnostics to settle.
    settling: Vec<Settling<R>>,
}

struct Asked<R> {
    reply_to: R,
    started: Instant,
}

struct Settling<R> {
    reply_to: R,
    text: String,
    documents: Vec<DocumentId>,
    wait: DiagnosticsWait,
}

impl<R> Default for ToolCalls<R> {
    fn default() -> Self {
        Self {
            next: 0,
            asked: HashMap::new(),
            settling: Vec::new(),
        }
    }
}

impl<R> ToolCalls<R> {
    /// Runs tool `name` with `args` at `now`, its answer for `reply_to`:
    /// that answer, if it's ready now; else it comes from `poll`.
    pub fn start(
        &mut self,
        editor: &mut Editor,
        now: Instant,
        name: &str,
        args: &Value,
        reply_to: R,
    ) -> Option<(R, Answer)> {
        let call = self.next;
        self.next += 1;
        match tools::call(editor, call, name, args) {
            Called::Done(answer) => Some((reply_to, answer)),
            Called::Waiting => {
                self.asked.insert(
                    call,
                    Asked {
                        reply_to,
                        started: now,
                    },
                );
                None
            }
            Called::Changed(text, documents) => {
                self.settle(editor, now, reply_to, text, documents);
                None
            }
        }
    }

    /// The answers ready at `now`: the language servers' (those that
    /// changed files once their diagnostics settle), and the calls given
    /// up on.
    pub fn poll(&mut self, editor: &mut Editor, now: Instant) -> Vec<(R, Answer)> {
        let mut ready = Vec::new();
        for answer in editor.take_agent_lsp_answers() {
            // Not asked: given up, or its agent's gone.
            let Some(asked) = self.asked.remove(&answer.call) else {
                continue;
            };
            match answer.answer {
                Ok(text) if !answer.changed.is_empty() => {
                    self.settle(editor, now, asked.reply_to, text, answer.changed);
                }
                answer => ready.push((asked.reply_to, answer)),
            }
        }
        let mut late: Vec<u64> = self
            .asked
            .iter()
            .filter(|(_, asked)| now - asked.started >= ANSWER_WAIT)
            .map(|(&call, _)| call)
            .collect();
        late.sort_unstable();
        for call in late {
            let asked = self.asked.remove(&call).expect("listed");
            let made = editor.agent_give_up(call);
            ready.push((asked.reply_to, Err(given_up(made))));
        }
        for mut settling in std::mem::take(&mut self.settling) {
            let state = diagnostics_state(editor, &settling.documents);
            let text = match settling.wait.poll(now, &state) {
                Settled::Waiting => {
                    self.settling.push(settling);
                    continue;
                }
                Settled::Reported => {
                    format!(
                        "{}{}",
                        settling.text,
                        editor.agent_problems(&settling.documents)
                    )
                }
                Settled::Unchecked => settling.text,
                Settled::StillChecking(server) => format!(
                    "{}\n{server} is still checking: get_diagnostics will say what's wrong in a moment.",
                    settling.text
                ),
            };
            ready.push((settling.reply_to, Ok(text)));
        }
        ready
    }

    /// Drops the calls whose answers would go to someone `gone` (an agent
    /// that disconnected). Those waiting on a server are given up.
    pub fn forget(&mut self, editor: &mut Editor, gone: impl Fn(&R) -> bool) {
        let calls: Vec<u64> = self
            .asked
            .iter()
            .filter(|(_, asked)| gone(&asked.reply_to))
            .map(|(&call, _)| call)
            .collect();
        for call in calls {
            self.asked.remove(&call);
            editor.agent_give_up(call);
        }
        self.settling.retain(|s| !gone(&s.reply_to));
    }

    fn settle(
        &mut self,
        editor: &Editor,
        now: Instant,
        reply_to: R,
        text: String,
        documents: Vec<DocumentId>,
    ) {
        let wait = DiagnosticsWait::new(now, &diagnostics_state(editor, &documents));
        self.settling.push(Settling {
            reply_to,
            text,
            documents,
            wait,
        });
    }
}

fn diagnostics_state(editor: &Editor, documents: &[DocumentId]) -> Vec<DocDiagnostics> {
    documents
        .iter()
        .map(|&d| editor.diagnostics_state(d))
        .collect()
}

/// What the agent's told of a call given up on: `made`, the note on edits
/// a command made for it before then, if it did.
fn given_up(made: Option<(String, Vec<DocumentId>)>) -> String {
    let waited = ANSWER_WAIT.as_secs();
    match made {
        None => format!(
            "The language server didn't answer in {waited}s, so this was given up: nothing was \
             changed. language_servers says how the servers are doing."
        ),
        Some((note, _)) => format!(
            "The language server didn't finish in {waited}s, so this was given up: the edits it \
             had made by then were kept{note}. language_servers says how the servers are doing."
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::core::Position;
    use crate::editor::lsp::{Answer as LspAnswer, Edits};

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// An editor with language servers on, and a file to ask about.
    fn with_file(text: &str) -> (tempfile::TempDir, Editor, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, text).unwrap();
        let mut editor = Editor::default();
        editor.options.lsp = true;
        (dir, editor, path.to_str().unwrap().to_string())
    }

    #[test]
    fn what_the_editor_knows_is_answered_at_once() {
        let mut editor = Editor::default();
        let mut calls = ToolCalls::default();
        let now = Instant::now();
        let answered = calls.start(&mut editor, now, "editor_state", &json!({}), 7);
        assert!(matches!(answered, Some((7, Ok(_)))), "{answered:?}");
        let bad = calls.start(&mut editor, now, "read_buffer", &json!({}), 8);
        assert_eq!(bad, Some((8, Err("Which file? (path)".into()))));
        assert!(calls.poll(&mut editor, now).is_empty());
    }

    #[test]
    fn a_servers_answer_comes_from_poll() {
        let (_dir, mut editor, file) = with_file("fn main() {}\n");
        let mut calls = ToolCalls::default();
        let t0 = Instant::now();
        let args = json!({"path": file, "line": 1, "symbol": "main"});
        assert_eq!(calls.start(&mut editor, t0, "lsp_hover", &args, 1), None);
        assert!(calls.poll(&mut editor, t0 + secs(1)).is_empty());
        let request = editor.take_lsp_requests().pop().unwrap();
        editor.lsp_answered(&request, Ok(LspAnswer::Hover("fn main()".into())));
        let ready = calls.poll(&mut editor, t0 + secs(2));
        assert!(
            matches!(&ready[..], [(1, Ok(text))] if text.contains("fn main()")),
            "{ready:?}"
        );
    }

    #[test]
    fn a_server_that_never_answers_is_given_up_on() {
        let (_dir, mut editor, file) = with_file("fn main() {}\n");
        let mut calls = ToolCalls::default();
        let t0 = Instant::now();
        let args = json!({"path": file, "line": 1, "symbol": "main"});
        calls.start(&mut editor, t0, "lsp_hover", &args, 1);
        assert!(calls.poll(&mut editor, t0 + secs(59)).is_empty());
        let ready = calls.poll(&mut editor, t0 + secs(60));
        assert!(
            matches!(&ready[..], [(1, Err(why))]
                if why.starts_with("The language server didn't answer in 60s")
                    && why.contains("nothing was changed")),
            "{ready:?}"
        );
        // Told once.
        assert!(calls.poll(&mut editor, t0 + secs(61)).is_empty());
    }

    #[test]
    fn a_late_answer_is_dropped_edits_and_all() {
        let (_dir, mut editor, file) = with_file("fn old() {}\n");
        let mut calls = ToolCalls::default();
        let t0 = Instant::now();
        let args = json!({"path": file, "line": 1, "symbol": "old", "new_name": "new"});
        calls.start(&mut editor, t0, "lsp_rename", &args, 1);
        let request = editor.take_lsp_requests().pop().unwrap();
        assert_eq!(calls.poll(&mut editor, t0 + secs(60)).len(), 1);
        // The rename comes after all.
        let edits = Edits {
            documents: vec![(
                request.document,
                vec![(Position::new(0, 3), Position::new(0, 6), "new".into())],
            )],
            ..Edits::default()
        };
        editor.lsp_answered(&request, Ok(LspAnswer::Edits(edits.clone())));
        assert!(calls.poll(&mut editor, t0 + secs(61)).is_empty());
        let text = editor
            .document_by_id(request.document)
            .unwrap()
            .buffer()
            .text();
        assert_eq!(text, "fn old() {}", "not renamed");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn old() {}\n");
        // Nor are a command's edits for it.
        assert!(!editor.lsp_edits_pushed(&request, None, edits));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn old() {}\n");
    }

    #[test]
    fn an_answer_in_time_makes_its_edits() {
        let (_dir, mut editor, file) = with_file("fn old() {}\n");
        let mut calls = ToolCalls::default();
        let t0 = Instant::now();
        let args = json!({"path": file, "line": 1, "symbol": "old", "new_name": "new"});
        calls.start(&mut editor, t0, "lsp_rename", &args, 1);
        let request = editor.take_lsp_requests().pop().unwrap();
        let edits = Edits {
            documents: vec![(
                request.document,
                vec![(Position::new(0, 3), Position::new(0, 6), "new".into())],
            )],
            ..Edits::default()
        };
        editor.lsp_answered(&request, Ok(LspAnswer::Edits(edits)));
        // It changed a file: answered once its diagnostics settle. Nothing
        // checks it here, but rust-analyzer's set up for it, so a moment's
        // given for it to open it; the 60s no longer count.
        assert!(calls.poll(&mut editor, t0 + secs(59)).is_empty());
        let ready = calls.poll(&mut editor, t0 + secs(61));
        assert!(matches!(&ready[..], [(1, Ok(_))]), "{ready:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn new() {}\n");
    }

    #[test]
    fn calls_for_an_agent_thats_gone_are_dropped() {
        let (_dir, mut editor, file) = with_file("fn main() {}\n");
        let mut calls = ToolCalls::default();
        let t0 = Instant::now();
        let args = json!({"path": file, "line": 1, "symbol": "main"});
        calls.start(&mut editor, t0, "lsp_hover", &args, 1);
        calls.start(&mut editor, t0, "lsp_hover", &args, 2);
        calls.forget(&mut editor, |&r| r == 1);
        for request in editor.take_lsp_requests() {
            editor.lsp_answered(&request, Ok(LspAnswer::Hover("fn main()".into())));
        }
        let ready = calls.poll(&mut editor, t0 + secs(1));
        assert!(matches!(&ready[..], [(2, Ok(_))]), "{ready:?}");
        // And none is given up on later.
        assert!(calls.poll(&mut editor, t0 + secs(60)).is_empty());
    }

    #[test]
    fn a_change_is_answered_once_its_diagnostics_settle() {
        let (_dir, mut editor, file) = with_file("one\n");
        let mut calls = ToolCalls::default();
        let t0 = Instant::now();
        let args = json!({"path": file, "old_text": "one", "new_text": "two"});
        assert_eq!(calls.start(&mut editor, t0, "edit_buffer", &args, 1), None);
        // A moment for rust-analyzer to open it.
        assert!(calls.poll(&mut editor, t0).is_empty());
        let ready = calls.poll(&mut editor, t0 + secs(1));
        assert!(matches!(&ready[..], [(1, Ok(_))]), "{ready:?}");
    }
}
