//! Syntax highlighting with tree-sitter.
//!
//! [`SyntaxEngine`] picks a language for a document (by the `[languages]`
//! config: file name, extension or `#!` line), highlights the
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
pub use languages::{LANGUAGES, LanguageDef, language};

use std::cell::OnceCell;
use std::collections::HashMap;

use tree_sitter_highlight::{HighlightConfiguration, Highlighter};

use crate::config::Languages;
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
    pub fn highlights(
        &mut self,
        document: &Document,
        languages: &Languages,
    ) -> Option<&Highlights> {
        self.refresh(document, languages);
        self.get(document.id())
    }

    /// Brings `document`'s cached highlights up to date. Call this for each
    /// document on screen, then `get` them all (several windows can show
    /// different documents at once).
    pub fn refresh(&mut self, document: &Document, languages: &Languages) {
        let id = document.id();
        let name = languages.detect(document.path(), &document.buffer().line(0));
        let Some((index, language)) = LANGUAGES
            .iter()
            .enumerate()
            .find(|(_, l)| Some(l.name) == name)
        else {
            self.cache.remove(&id);
            return;
        };

        let version = document.text_version();
        let fresh = self
            .cache
            .get(&id)
            .is_some_and(|c| c.version == version && c.language == language.name);
        if fresh {
            return;
        }
        let highlights = if document.buffer().len_chars() > MAX_HIGHLIGHT_CHARS {
            None
        } else {
            self.configs[index]
                .get_or_init(|| load(language))
                .as_ref()
                .ok()
                .and_then(|config| {
                    highlight::highlight(&mut self.highlighter, config, &document.buffer().text())
                        .ok()
                })
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

    /// `highlights` with the default `[languages]` table.
    #[cfg(test)]
    pub fn highlights_default(&mut self, document: &Document) -> Option<&Highlights> {
        let languages = crate::config::Config::default().languages;
        self.highlights(document, &languages)
    }

    /// The cached highlights for document `id` (see `refresh`).
    pub fn get(&self, id: DocumentId) -> Option<&Highlights> {
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
        &language.highlight_query(),
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
                .highlights_default(&document("a.rs", "fn main() {}"))
                .is_some()
        );
        assert!(
            engine
                .highlights_default(&document("a.txt", "fn main() {}"))
                .is_none()
        );
        assert!(
            engine
                .highlights_default(&Document::with_text("fn main() {}"))
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
            ("a.jsx", "const a = <div className=\"x\">hi</div>;"),
            ("a.ts", "function f(x: number): string { return 'x'; }"),
            ("a.tsx", "const a = (x: number) => <b>{x}</b>;"),
            ("a.go", "package main\nfunc main() {}"),
            ("a.c", "int main(void) { return 0; }"),
            ("a.cpp", "class A { public: int x; };"),
            ("a.sh", "if [ -n \"$x\" ]; then echo hi; fi"),
            ("a.md", "# Heading\n\nText"),
            ("a.yml", "key: value"),
            ("a.html", "<p class=\"x\">hi</p>"),
            ("a.css", "body { color: red; }"),
            ("script", "#!/bin/bash\necho hi"),
        ];
        assert_eq!(
            samples.len(),
            LANGUAGES.len() + 2,
            "a sample per language (plus jsx and a #! script)"
        );
        let mut engine = SyntaxEngine::default();
        for (name, text) in samples {
            let doc = document(name, text);
            let highlights = engine.highlights_default(&doc).unwrap();
            assert!(!highlights.line(0).is_empty(), "{name} has no highlights");
        }
    }

    #[test]
    fn retain_drops_closed_documents() {
        let mut engine = SyntaxEngine::default();
        let doc = document("a.rs", "fn x() {}");
        engine.highlights_default(&doc);
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
        let before = engine.highlights_default(&doc).unwrap().line(0).to_vec();
        doc.insert(crate::core::Position::new(0, 0), "fn ");
        let after = engine.highlights_default(&doc).unwrap().line(0).to_vec();
        assert_ne!(before, after);
        doc.undo();
        assert_eq!(engine.highlights_default(&doc).unwrap().line(0), before);
    }
}
