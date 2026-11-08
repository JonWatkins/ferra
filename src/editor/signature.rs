//! Signature help: while typing a call's arguments, the function's
//! signature in a popup above the line, the parameter being typed picked
//! out. The server is asked when a character it names is typed (`(`, `,`),
//! and again on each change while it's showing, so the parameter follows
//! the cursor; it goes when the server has none, or insert mode ends.

use super::lsp::{LspFeature, LspRequest};
use super::{Editor, Message, Mode};

/// One signature to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    /// `fn area(&self, scale: f64) -> f64`.
    pub label: String,
    /// The parameter being typed: its chars in `label`.
    pub active: Option<(usize, usize)>,
    /// Which of the server's signatures (overloads) this is, of how many.
    pub index: usize,
    pub total: usize,
    /// The parameter's documentation, or the signature's: a few lines.
    pub documentation: Vec<String>,
}

impl Editor {
    /// The signature being shown, if any.
    pub fn signature_help(&self) -> Option<&SignatureHelp> {
        self.signature.as_ref()
    }

    /// `<C-s>` in insert mode: asks for the signature of the call the
    /// cursor is in.
    pub fn request_signature_help(&mut self) -> Result<(), String> {
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
            feature: LspFeature::SignatureHelp,
            document: self.document.id(),
            position: cursor,
            buffer_id: buffer.id(),
            edit_seq: buffer.edit_seq(),
            manual,
            trigger: typed,
            word_start: cursor,
            item: None,
            text: None,
            end: cursor,
        });
        Ok(())
    }

    /// Shows the server's answer to `request`: a signature, or none (which
    /// closes it), if the editor is still typing in that buffer.
    pub fn show_signature_help(&mut self, request: &LspRequest, help: Option<SignatureHelp>) {
        if self.document.id() != request.document || self.mode() != Mode::Insert {
            return;
        }
        if help.is_none() && request.manual {
            self.set_message(Message::info("No signature here"));
        }
        self.signature = help;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help() -> SignatureHelp {
        SignatureHelp {
            label: "f(a)".into(),
            active: Some((2, 3)),
            index: 0,
            total: 1,
            documentation: Vec::new(),
        }
    }

    #[test]
    fn leaving_insert_mode_closes_it() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        editor.signature = Some(help());
        editor.signature_after_key(false);
        assert!(editor.signature_help().is_some(), "a key that didn't type");
        editor.set_mode(Mode::Normal);
        editor.signature_after_key(false);
        assert!(editor.signature_help().is_none());
    }
}
