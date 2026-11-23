//! The agent's side of LSP queries: its answers, as text for it. The
//! screen's side is in `lsp.rs`; `lsp/` hands both the same answers and
//! never knows who asked (see docs/adr/0001-lsp-answers-are-structured.md).

use super::Editor;
use super::lsp::{Answer, LspFeature, LspRequest, NoAnswer};

impl Editor {
    /// The answer to the agent's tool call `call`, or why there's none.
    pub(super) fn agent_lsp_answered(
        &mut self,
        call: u64,
        request: &LspRequest,
        answer: Result<Answer, NoAnswer>,
    ) {
        let why = match answer {
            Ok(answer) => match answer {},
            Err(why) => why,
        };
        self.agent_lsp_answer(call, Err(no_answer_text(request.feature, &why)));
    }
}

/// Why there's no answer, for the agent: what to do about it, too.
fn no_answer_text(feature: LspFeature, why: &NoAnswer) -> String {
    match why {
        NoAnswer::NoServer => "No language server for this file".into(),
        NoAnswer::Starting(_) if feature == LspFeature::MoveFile => {
            "The language server is still starting (it'd update the imports): try again in a moment"
                .into()
        }
        NoAnswer::Starting(_) => {
            "The language server is still starting (waited 30 s): try again in a moment".into()
        }
        NoAnswer::Unsupported(name) => format!("{name} doesn't offer {}", feature.noun()),
        NoAnswer::Stopped(name) => format!("{name} stopped before answering"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_is_told_why_and_what_to_do() {
        let starting = NoAnswer::Starting("rust-analyzer".into());
        assert!(no_answer_text(LspFeature::Hover, &starting).contains("try again"));
        assert!(no_answer_text(LspFeature::MoveFile, &starting).contains("imports"));
        assert_eq!(
            no_answer_text(LspFeature::Callers, &NoAnswer::Unsupported("x".into())),
            "x doesn't offer callers"
        );
        assert_eq!(
            no_answer_text(LspFeature::Hover, &NoAnswer::Stopped("x".into())),
            "x stopped before answering"
        );
    }
}
