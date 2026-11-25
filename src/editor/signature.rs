//! Signature help: while typing a call's arguments, the function's
//! signature in a popup above the line, the parameter being typed picked
//! out. The server is asked when a character it names is typed (`(`, `,`),
//! and again on each change while it's showing, so the parameter follows
//! the cursor; it goes when the server has none, or insert mode ends.
//!
//! - Signature help (`editor/signature.rs`, drawn by `ui/signature.rs`): after
//!   each key that changes the text in insert mode, `LspQuery::SignatureHelp`
//!   is asked with the character before the cursor; `Lsp` sends it if that's
//!   one of the server's trigger or retrigger characters (`(`, `,`), or on any
//!   change while a signature is showing (`isRetrigger`), so the active
//!   parameter follows the cursor. An answer with a newer request waiting isn't
//!   read, as for completion. The active signature's label is shown with its
//!   active parameter (by substring, or UTF-16 offsets) in `ui.popup.match`,
//!   and the first paragraph of its documentation (code blocks left out)
//!   wrapped to at most four rows. No signature in the answer closes it; so
//!   does leaving insert mode. It goes above the line, so the completion menu
//!   can be below, or below when there's no room.
//! - Overloads: all the answer's signatures are kept, and `<C-s>` while
//!   more than one is showing steps to the next (round to the first), as
//!   Neovim's `<C-s>` in its signature window; the popup's bottom edge says
//!   which (`2/3`). The one stepped to goes back with each retrigger as
//!   `activeSignatureHelp`, and if the server picks its own anyway, an
//!   answer that still has it (by label) keeps it shown.

use super::lsp::{LspQuery, LspRequest, Reply};
use super::{Editor, Message, Mode};

/// The server's signatures (overloads) for the call being typed, and the
/// one shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    /// Never empty.
    pub signatures: Vec<Signature>,
    /// The one shown: the server's active one, or the one stepped to.
    pub index: usize,
    /// It was stepped to (`<C-s>`), so a new answer keeps it if it has it.
    pub picked: bool,
}

/// One signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// `fn area(&self, scale: f64) -> f64`.
    pub label: String,
    /// The parameter being typed: its chars in `label`.
    pub active: Option<(usize, usize)>,
    /// The parameter's documentation, or the signature's: a few lines.
    pub documentation: Vec<String>,
}

impl SignatureHelp {
    /// The signature shown.
    pub fn shown(&self) -> &Signature {
        &self.signatures[self.index]
    }

    /// The one after the shown one, round to the first.
    fn step(&mut self) {
        self.index = (self.index + 1) % self.signatures.len();
        self.picked = true;
    }
}

impl Editor {
    /// The signature being shown, if any.
    pub fn signature_help(&self) -> Option<&SignatureHelp> {
        self.signature.as_ref()
    }

    /// `<C-s>` in insert mode: asks for the signature of the call the
    /// cursor is in, or, with one showing that has overloads, shows the
    /// next of them (as Neovim's `<C-s>` in its signature window).
    pub fn request_signature_help(&mut self) -> Result<(), String> {
        if let Some(help) = &mut self.signature
            && help.signatures.len() > 1
        {
            help.step();
            return Ok(());
        }
        self.queue_signature_help(true, None)
    }

    /// After each key: leaving insert mode closes the signature; a change
    /// to the text may ask for it (the server decides, by the character
    /// typed, or because it's showing).
    pub(super) fn signature_after_key(&mut self, typed: bool) {
        if self.mode() != Mode::Insert {
            self.signature = None;
            return;
        }
        if !typed || !self.options.lsp {
            return;
        }
        let cursor = self.document.cursor();
        let before = (cursor.col > 0)
            .then(|| {
                self.document
                    .buffer()
                    .char_at(cursor.with_col(cursor.col - 1))
            })
            .flatten();
        let _ = self.queue_signature_help(false, before);
    }

    fn queue_signature_help(&mut self, manual: bool, typed: Option<char>) -> Result<(), String> {
        if !self.options.lsp {
            return Err("Language servers are off (lsp = false)".into());
        }
        if self.document.path().is_none() {
            return Err("No language server: the buffer has no file".into());
        }
        let cursor = self.document.cursor();
        let buffer = self.document.buffer();
        self.lsp_requests.push(LspRequest {
            query: LspQuery::SignatureHelp {
                trigger: typed,
                retrigger: None,
            },
            document: self.document.id(),
            position: cursor,
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            reply: if manual { Reply::Key } else { Reply::Typing },
        });
        Ok(())
    }

    /// Shows the server's answer to `request`: its signatures, or none
    /// (which closes it), if the editor is still typing in that buffer. An
    /// overload stepped to stays shown if the answer has it, for servers
    /// that pick their own whatever `activeSignatureHelp` says.
    pub fn show_signature_help(&mut self, request: &LspRequest, help: Option<SignatureHelp>) {
        if self.document.id() != request.document || self.mode() != Mode::Insert {
            return;
        }
        if help.is_none() && request.reply.explicit() {
            self.set_message(Message::info("No signature here"));
        }
        let mut help = help;
        if let (Some(new), Some(old)) = (&mut help, &self.signature)
            && old.picked
            && let Some(at) = new
                .signatures
                .iter()
                .position(|s| s.label == old.shown().label)
        {
            new.index = at;
            new.picked = true;
        }
        self.signature = help;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help(labels: &[&str]) -> SignatureHelp {
        SignatureHelp {
            signatures: labels
                .iter()
                .map(|label| Signature {
                    label: label.to_string(),
                    active: Some((2, 3)),
                    documentation: Vec::new(),
                })
                .collect(),
            index: 0,
            picked: false,
        }
    }

    fn shown(editor: &Editor) -> &str {
        &editor.signature_help().unwrap().shown().label
    }

    fn answer(editor: &mut Editor, help: SignatureHelp) {
        let request = LspRequest {
            query: LspQuery::SignatureHelp {
                trigger: None,
                retrigger: None,
            },
            document: editor.document.id(),
            position: editor.document.cursor(),
            buffer_id: editor.document.buffer().id(),
            edit_seq: editor.document.buffer().edit_seq(),
            reply: Reply::Typing,
        };
        editor.show_signature_help(&request, Some(help));
    }

    #[test]
    fn ctrl_s_steps_through_the_overloads_and_an_answer_keeps_the_one_picked() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.signature = Some(help(&["f(a)", "f(a, b)", "f()"]));
        editor.request_signature_help().unwrap();
        assert_eq!(shown(&editor), "f(a, b)");
        // The server's answer to the next change has it second, as its
        // active one first: the one picked stays.
        let mut again = help(&["f()", "f(a, b)"]);
        again.index = 0;
        answer(&mut editor, again);
        assert_eq!(shown(&editor), "f(a, b)");
        editor.request_signature_help().unwrap();
        editor.request_signature_help().unwrap();
        assert_eq!(shown(&editor), "f(a, b)", "round to the first and on");
        // An answer without it shows the server's.
        answer(&mut editor, help(&["g()", "g(x)"]));
        assert_eq!(shown(&editor), "g()");
        assert!(editor.lsp_requests.is_empty(), "stepping doesn't ask");
        // With only one, `<C-s>` asks again.
        editor.signature = Some(help(&["f(a)"]));
        let _ = editor.request_signature_help();
        assert_eq!(shown(&editor), "f(a)");
    }

    #[test]
    fn leaving_insert_mode_closes_it() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.signature = Some(help(&["f(a)"]));
        editor.signature_after_key(false);
        assert!(editor.signature_help().is_some(), "a key that didn't type");
        editor.set_mode(Mode::Normal);
        editor.signature_after_key(false);
        assert!(editor.signature_help().is_none());
    }
}
