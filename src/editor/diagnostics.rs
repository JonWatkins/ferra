//! Diagnostics: errors and warnings a language server reports for a
//! buffer, shown in the text and stepped through with `]d`/`[d`.

use super::{DocumentId, Editor, Message};
use crate::core::Position;

/// How serious a diagnostic is, most serious first (LSP's numbering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    /// From LSP's `DiagnosticSeverity` (1 to 4); errors when it's missing.
    pub fn from_lsp(value: Option<u64>) -> Severity {
        match value {
            Some(2) => Severity::Warning,
            Some(3) => Severity::Info,
            Some(4) => Severity::Hint,
            _ => Severity::Error,
        }
    }

    /// The theme scope's last part: `diagnostic.error`...
    pub fn id(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }
}

/// One diagnostic: chars `start..end` (positions in the buffer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub start: Position,
    pub end: Position,
    pub severity: Severity,
    pub message: String,
    /// What reported it (`rustc`, `clippy`, `Pyright`).
    pub source: Option<String>,
}

impl Diagnostic {
    /// `error: message (source)`, on one line.
    pub fn describe(&self) -> String {
        let message = self.message.lines().next().unwrap_or_default();
        match &self.source {
            Some(source) => format!("{}: {message} ({source})", self.severity.id()),
            None => format!("{}: {message}", self.severity.id()),
        }
    }
}

impl Editor {
    /// Replaces document `id`'s diagnostics.
    pub fn set_diagnostics(&mut self, id: DocumentId, mut diagnostics: Vec<Diagnostic>) {
        diagnostics.sort_by_key(|d| (d.start, d.severity));
        if diagnostics.is_empty() {
            self.diagnostics.remove(&id);
        } else {
            self.diagnostics.insert(id, diagnostics);
        }
    }

    /// Document `id`'s diagnostics, in order.
    pub fn diagnostics(&self, id: DocumentId) -> &[Diagnostic] {
        self.diagnostics.get(&id).map_or(&[], Vec::as_slice)
    }

    /// `]d` / `[d`: moves to the `count`th next (or previous) diagnostic in
    /// this buffer, wrapping, and shows its message.
    pub fn goto_diagnostic(&mut self, forward: bool, count: usize) -> Result<(), String> {
        let id = self.document.id();
        let all = self.diagnostics(id);
        if all.is_empty() {
            return Err("No diagnostics in this buffer".into());
        }
        let cursor = self.document.cursor();
        let mut at = cursor;
        let mut found = None;
        for _ in 0..count.max(1) {
            let next = if forward {
                all.iter().find(|d| d.start > at).or_else(|| all.first())
            } else {
                all.iter()
                    .rev()
                    .find(|d| d.start < at)
                    .or_else(|| all.last())
            };
            if let Some(d) = next {
                at = d.start;
                found = Some(d.clone());
            }
        }
        let found = found.expect("not empty");
        self.push_jump();
        self.document.set_cursor(found.start);
        self.reveal_cursor();
        self.set_message(match found.severity {
            Severity::Error => Message::error(found.describe()),
            _ => Message::info(found.describe()),
        });
        Ok(())
    }

    /// The most serious diagnostic on `row` of document `id`.
    pub fn line_diagnostic(&self, id: DocumentId, row: usize) -> Option<&Diagnostic> {
        self.diagnostics(id)
            .iter()
            .filter(|d| d.start.row <= row && row <= d.end.row)
            .min_by_key(|d| d.severity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    fn diagnostic(row: usize, col: usize, severity: Severity) -> Diagnostic {
        Diagnostic {
            start: Position::new(row, col),
            end: Position::new(row, col + 1),
            severity,
            message: format!("at {row}:{col}"),
            source: Some("test".into()),
        }
    }

    #[test]
    fn stepping_through_diagnostics_wraps() {
        let mut e = Editor::with_document(Document::with_text("a\nb\nc\nd"));
        let id = e.document.id();
        e.set_diagnostics(
            id,
            vec![
                diagnostic(2, 0, Severity::Warning),
                diagnostic(1, 0, Severity::Error),
            ],
        );
        e.goto_diagnostic(true, 1).unwrap();
        assert_eq!(e.document.cursor(), Position::new(1, 0));
        assert_eq!(e.message().unwrap().text, "error: at 1:0 (test)");
        e.goto_diagnostic(true, 1).unwrap();
        assert_eq!(e.document.cursor(), Position::new(2, 0));
        e.goto_diagnostic(true, 1).unwrap();
        assert_eq!(e.document.cursor(), Position::new(1, 0), "wraps");
        e.goto_diagnostic(false, 1).unwrap();
        assert_eq!(e.document.cursor(), Position::new(2, 0), "back, wrapping");
        assert_eq!(e.line_diagnostic(id, 1).unwrap().severity, Severity::Error);
        e.set_diagnostics(id, Vec::new());
        assert!(e.goto_diagnostic(true, 1).is_err());
    }
}
