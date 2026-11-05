//! Syntax highlighting with tree-sitter.
//!
//! [`SyntaxEngine`] picks a language from a document's path, highlights the
//! whole document, and caches the result until the text changes. The UI
//! styles each [`Span`] with the theme scope named by its highlight
//! (`keyword`, `function.method`, ...).
//!
//! The document is re-highlighted from scratch when its text changes. That
//! is simple and fast enough for typical source files; incremental parsing
//! (editing the previous tree) is the upgrade path for very large ones.

mod highlight;
mod languages;

pub use highlight::{HIGHLIGHT_NAMES, Highlights, Span};
pub use languages::{LANGUAGES, LanguageDef, detect};

use std::cell::OnceCell;
use std::collections::HashMap;

use tree_sitter_highlight::{HighlightConfiguration, Highlighter};

use crate::editor::{Document, DocumentId};

/// Documents larger than this (in chars) aren't highlighted, so opening a
/// huge file stays responsive.
pub const MAX_HIGHLIGHT_CHARS: usize = 4 * 1024 * 1024;

struct Cached {
    version: u64,
    language: &'static str,
    highlights: Option<Highlights>,
}

pub struct SyntaxEngine {
    highlighter: Highlighter,
    /// Compiled queries, one per entry in `LANGUAGES`, built on first use.
    configs: Vec<OnceCell<Result<HighlightConfiguration, String>>>,
    cache: HashMap<DocumentId, Cached>,
}

impl Default for SyntaxEngine {
    fn default() -> Self {
        Self {
            highlighter: Highlighter::new(),
            configs: LANGUAGES.iter().map(|_| OnceCell::new()).collect(),
            cache: HashMap::new(),
        }
    }
}

impl SyntaxEngine {
    /// Highlights for `document`, recomputed only if its text or language
    /// changed. `None` if the language is unknown, the document is too
    /// large, or highlighting failed.
    pub fn highlights(&mut self, document: &Document) -> Option<&Highlights> {
        let (index, language) = LANGUAGES.iter().enumerate().find(|(_, l)| {
            document
                .path()
                .and_then(detect)
                .is_some_and(|d| d.name == l.name)
        })?;

        let id = document.id();
        let version = document.text_version();
        let stale = self
            .cache
            .get(&id)
            .is_none_or(|c| c.version != version || c.language != language.name);
        if stale {
            let highlights = if document.buffer().len_chars() > MAX_HIGHLIGHT_CHARS {
                None
            } else {
                let config = self.configs[index]
                    .get_or_init(|| load(language))
                    .as_ref()
                    .ok()?;
                highlight::highlight(&mut self.highlighter, config, &document.buffer().text()).ok()
            };
            self.cache.insert(
                id,
                Cached {
                    version,
                    language: language.name,
                    highlights,
                },
            );
        }
        self.cache.get(&id)?.highlights.as_ref()
    }

    /// Drops cached highlights for documents that are no longer open.
    pub fn retain(&mut self, open: &[DocumentId]) {
        self.cache.retain(|id, _| open.contains(id));
    }

    /// How many documents have cached highlights.
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    /// Compiles every language's queries, reporting any that fail. Used by
    /// tests to check the bundled grammars and queries agree.
    pub fn load_all(&self) -> Vec<(&'static str, Result<(), String>)> {
        LANGUAGES
            .iter()
            .enumerate()
            .map(|(i, language)| {
                let loaded = self.configs[i].get_or_init(|| load(language));
                (
                    language.name,
                    loaded.as_ref().map(|_| ()).map_err(Clone::clone),
                )
            })
            .collect()
    }
}

fn load(language: &LanguageDef) -> Result<HighlightConfiguration, String> {
    let mut config = HighlightConfiguration::new(
        (language.grammar)(),
        language.name,
        language.highlights,
        language.injections,
        language.locals,
    )
    .map_err(|e| format!("{}: invalid highlight query: {e}", language.name))?;
    config.configure(HIGHLIGHT_NAMES);
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(name: &str, text: &str) -> Document {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        let (doc, _) = Document::open(&path).unwrap();
        doc
    }

    #[test]
    fn every_bundled_language_loads() {
        for (name, result) in SyntaxEngine::default().load_all() {
            assert!(result.is_ok(), "{name}: {result:?}");
        }
    }

    #[test]
    fn highlights_known_languages_only() {
        let mut engine = SyntaxEngine::default();
        assert!(
            engine
                .highlights(&document("a.rs", "fn main() {}"))
                .is_some()
        );
        assert!(
            engine
                .highlights(&document("a.txt", "fn main() {}"))
                .is_none()
        );
        assert!(
            engine
                .highlights(&Document::with_text("fn main() {}"))
                .is_none(),
            "no path"
        );
    }

    #[test]
    fn each_language_produces_highlights() {
        let samples = [
            ("a.rs", "fn main() { let x = 1; }"),
            ("a.toml", "[package]\nname = \"x\""),
            ("a.json", "{\"a\": [1, true, null]}"),
            ("a.py", "def f(x):\n    return \"s\""),
            ("a.js", "function f() { return 'x'; }"),
        ];
        let mut engine = SyntaxEngine::default();
        for (name, text) in samples {
            let doc = document(name, text);
            let highlights = engine.highlights(&doc).unwrap();
            assert!(!highlights.line(0).is_empty(), "{name} has no highlights");
        }
    }

    #[test]
    fn retain_drops_closed_documents() {
        let mut engine = SyntaxEngine::default();
        let doc = document("a.rs", "fn x() {}");
        engine.highlights(&doc);
        assert_eq!(engine.cached_count(), 1);
        engine.retain(&[doc.id()]);
        assert_eq!(engine.cached_count(), 1);
        engine.retain(&[]);
        assert_eq!(engine.cached_count(), 0);
    }

    #[test]
    fn cache_follows_text_changes() {
        let mut engine = SyntaxEngine::default();
        let mut doc = document("a.rs", "x");
        let before = engine.highlights(&doc).unwrap().line(0).to_vec();
        doc.insert(crate::core::Position::new(0, 0), "fn ");
        let after = engine.highlights(&doc).unwrap().line(0).to_vec();
        assert_ne!(before, after);
        doc.undo();
        assert_eq!(engine.highlights(&doc).unwrap().line(0), before);
    }
}
