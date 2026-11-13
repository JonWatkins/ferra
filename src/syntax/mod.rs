//! Syntax highlighting with tree-sitter.
//!
//! [`SyntaxTrees`] keeps a syntax tree per document, fed the buffer's
//! edits so that after a change tree-sitter re-parses only what it touched;
//! the editor holds it, for indentation (`indent.rs`) and highlighting.
//! [`SyntaxEngine`] picks a language for a document (by the `[languages]`
//! config: file name, extension or `#!` line) and highlights just the rows
//! a window shows (see `highlight.rs`), cached until the text changes. The UI styles each [`Span`] with the
//! theme scope named by its highlight (`keyword`, `function.method`, ...).

pub mod compat;
mod highlight;
pub mod indent;
mod languages;
mod trees;

pub use highlight::{HIGHLIGHT_NAMES, Highlights, LanguageConfig, Span};
pub use languages::{
    Grammar, INJECTION_ONLY, LANGUAGES, LanguageDef, is_built_in, language, register,
};
pub use trees::SyntaxTrees;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::ops::Range;

use crate::config::Languages;
use crate::editor::{Document, DocumentId};

/// Documents larger than this (in chars) aren't parsed, so opening a huge
/// file stays responsive: no highlighting, and indentation by heuristics.
pub const MAX_HIGHLIGHT_CHARS: usize = 4 * 1024 * 1024;

/// Highlighted row ranges kept per document while its text is unchanged
/// (one per window showing it, say).
const CACHED_RANGES: usize = 4;

/// A document's recent highlights, for its text as of `edit_seq`.
struct DocumentHighlights {
    language: usize,
    buffer_id: u64,
    edit_seq: u64,
    /// Most recent last.
    highlights: Vec<Highlights>,
}

/// Highlights documents from the trees in [`SyntaxTrees`].
pub struct SyntaxEngine {
    /// For injected languages, which parse only parts of the text.
    injection_parser: tree_sitter::Parser,
    /// Compiled queries, one per language (by its registry position),
    /// built on first use; grows as plugins add languages.
    configs: Vec<OnceCell<Result<LanguageConfig, String>>>,
    documents: HashMap<DocumentId, DocumentHighlights>,
    /// Trees for `highlights_default`, which has no editor's to use.
    #[cfg(test)]
    trees: SyntaxTrees,
}

impl Default for SyntaxEngine {
    fn default() -> Self {
        Self {
            injection_parser: tree_sitter::Parser::new(),
            configs: LANGUAGES.iter().map(|_| OnceCell::new()).collect(),
            documents: HashMap::new(),
            #[cfg(test)]
            trees: SyntaxTrees::default(),
        }
    }
}

impl SyntaxEngine {
    /// Highlights for `rows` of `document`, from its tree in `trees`
    /// (brought up to date). `None` if the language is unknown, the
    /// document is too large, or parsing failed.
    pub fn highlight(
        &mut self,
        document: &Document,
        languages: &Languages,
        trees: &SyntaxTrees,
        rows: Range<usize>,
    ) -> Option<&Highlights> {
        let name = languages.detect(document.path(), &document.buffer().line(0));
        self.highlight_as(document, name, languages, trees, rows)
    }

    /// Like [`highlight`](Self::highlight), in `language` whatever the
    /// document's name (a hover's Markdown).
    pub fn highlight_as(
        &mut self,
        document: &Document,
        name: Option<&str>,
        languages: &Languages,
        trees: &SyntaxTrees,
        rows: Range<usize>,
    ) -> Option<&Highlights> {
        let id = document.id();
        let buffer = document.buffer();
        let Some(index) = name.and_then(languages::position) else {
            self.documents.remove(&id);
            return None;
        };
        // Room for languages added since.
        let count = languages::count();
        if self.configs.len() < count {
            self.configs.resize_with(count, OnceCell::new);
        }
        let definition = languages::get(index)?;
        let config = self.configs[index]
            .get_or_init(|| load(definition))
            .as_ref()
            .ok()?;

        let current = self.documents.get(&id).is_some_and(|d| {
            d.language == index && d.buffer_id == buffer.id() && d.edit_seq == buffer.edit_seq()
        });
        if !current {
            self.documents.insert(
                id,
                DocumentHighlights {
                    language: index,
                    buffer_id: buffer.id(),
                    edit_seq: buffer.edit_seq(),
                    highlights: Vec::new(),
                },
            );
        }
        let state = self.documents.get_mut(&id).expect("just made current");
        let rows = rows.start.min(buffer.line_count())..rows.end.min(buffer.line_count());
        let cached = state.highlights.iter().position(|h| {
            let have = h.rows();
            have.start <= rows.start && rows.end <= have.end
        });
        if let Some(at) = cached {
            return state.highlights.get(at);
        }

        let tree = trees.tree(document, definition.name)?;
        let configs = &self.configs;
        // Languages inside this one (a code block's, `<script>`'s), by name
        // or file extension (`js`, `py`), loaded on first use.
        let inject = move |name: &str| {
            let index = injected_language(name, languages)?;
            let definition = languages::get(index)?;
            configs
                .get(index)?
                .get_or_init(|| load(definition))
                .as_ref()
                .ok()
        };
        let highlights = highlight::highlight_rows(
            buffer,
            &tree,
            config,
            rows,
            &inject,
            &mut self.injection_parser,
        );
        if state.highlights.len() == CACHED_RANGES {
            state.highlights.remove(0);
        }
        state.highlights.push(highlights);
        state.highlights.last()
    }

    /// Highlights for all of `document`, with the default `[languages]`.
    #[cfg(test)]
    pub fn highlights_default(&mut self, document: &Document) -> Option<&Highlights> {
        let languages = crate::config::Config::default().languages;
        let rows = 0..document.buffer().line_count();
        let trees = std::mem::take(&mut self.trees);
        let found = self.highlight(document, &languages, &trees, rows).is_some();
        self.trees = trees;
        if !found {
            return None;
        }
        self.documents.get(&document.id())?.highlights.last()
    }

    /// Drops what's kept for documents that are no longer open.
    pub fn retain(&mut self, open: &[DocumentId]) {
        self.documents.retain(|id, _| open.contains(id));
        #[cfg(test)]
        self.trees.retain(open);
    }

    /// How many documents have highlights kept.
    pub fn cached_count(&self) -> usize {
        self.documents.len()
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

/// The built-in language an injection names: by name (`rust`,
/// `markdown_inline`), or by one of the extensions in `[languages]` (`js`,
/// `py`, `sh`), ignoring case.
fn injected_language(name: &str, languages: &Languages) -> Option<usize> {
    let name = name.trim().to_ascii_lowercase();
    if let Some(index) = languages::position(&name) {
        return Some(index);
    }
    let path = std::path::PathBuf::from(format!("x.{name}"));
    let language = languages.detect(Some(&path), "")?;
    languages::position(language)
}

fn load(language: &LanguageDef) -> Result<LanguageConfig, String> {
    LanguageConfig::new(
        language.name,
        language.grammar(),
        &language.highlight_query(),
        language.injections,
        language.locals,
    )
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
            LANGUAGES.len() - INJECTION_ONLY.len() + 2,
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

    /// The highlight names on `row` of `doc`, in order.
    fn names_on(engine: &mut SyntaxEngine, doc: &Document, row: usize) -> Vec<&'static str> {
        engine
            .highlights_default(doc)
            .unwrap()
            .line(row)
            .iter()
            .map(Span::name)
            .collect()
    }

    #[test]
    fn markdown_inline_syntax_comes_from_the_injected_grammar() {
        let mut engine = SyntaxEngine::default();
        let doc = document("a.md", "# Title\n\nSome **bold**, *it* and `code`.\n");
        assert!(names_on(&mut engine, &doc, 0).contains(&"text.title"));
        let prose = names_on(&mut engine, &doc, 2);
        for name in ["text.strong", "text.emphasis", "text.literal"] {
            assert!(prose.contains(&name), "{name} in {prose:?}");
        }
    }

    #[test]
    fn an_injected_span_wins_over_its_hosts_from_the_same_start() {
        // The title runs from the `` ` ``, as the code span does; the
        // code span's delimiter (from the inline grammar) is what shows.
        let mut engine = SyntaxEngine::default();
        let doc = document("a.md", "### `area` here\n");
        let spans = engine.highlights_default(&doc).unwrap().line(0).to_vec();
        let at = |col: usize| {
            spans
                .iter()
                .find(|s| s.start <= col && col < s.end)
                .map(Span::name)
        };
        assert_eq!(at(4), Some("punctuation.delimiter"), "{spans:?}");
        assert_eq!(at(9), Some("punctuation.delimiter"));
        assert_eq!(at(11), Some("text.title"));
    }

    #[test]
    fn code_blocks_are_highlighted_in_their_language() {
        let mut engine = SyntaxEngine::default();
        let doc = document(
            "a.md",
            "```rust\nfn main() {}\n```\n\n```py\ndef f(): pass\n```\n",
        );
        assert!(
            names_on(&mut engine, &doc, 1).contains(&"keyword"),
            "rust `fn`"
        );
        assert!(
            names_on(&mut engine, &doc, 5).contains(&"keyword"),
            "`py` by extension"
        );
    }

    #[test]
    fn html_script_and_style_are_highlighted() {
        let mut engine = SyntaxEngine::default();
        let doc = document(
            "a.html",
            "<script>\nconst x = 1;\n</script>\n<style>\np { color: red; }\n</style>\n",
        );
        assert!(
            names_on(&mut engine, &doc, 1).contains(&"keyword"),
            "js `const`"
        );
        assert!(
            names_on(&mut engine, &doc, 4).contains(&"property"),
            "css `color`"
        );
    }

    #[test]
    fn macro_calls_are_highlighted_as_rust() {
        let mut engine = SyntaxEngine::default();
        let doc = document(
            "a.rs",
            "fn f() {\n    assert!(x.is_empty() && Some(1).is_some());\n}\n",
        );
        let names = names_on(&mut engine, &doc, 1);
        // Without the injection, a macro's contents are just tokens.
        assert!(names.contains(&"function.method"), "{names:?}");
    }

    #[test]
    fn unknown_injected_languages_are_left_plain() {
        let mut engine = SyntaxEngine::default();
        let doc = document("a.md", "```brainfunk\n+++.\n```\n");
        assert!(engine.highlights_default(&doc).is_some(), "no failure");
    }

    // --- Checks against tree-sitter-highlight and fresh parses ------------

    /// Highlights `text` the way the editor did before keeping trees: with
    /// `tree-sitter-highlight` over the whole text.
    fn reference(name: &str, text: &str) -> Vec<Vec<Span>> {
        use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};
        static CONFIGS: std::sync::OnceLock<Vec<HighlightConfiguration>> =
            std::sync::OnceLock::new();
        let languages = crate::config::Config::default().languages;
        let configs = CONFIGS.get_or_init(|| {
            LANGUAGES
                .iter()
                .map(|l| {
                    let mut config = HighlightConfiguration::new(
                        l.grammar(),
                        l.name,
                        &l.highlight_query(),
                        l.injections,
                        l.locals,
                    )
                    .unwrap();
                    config.configure(HIGHLIGHT_NAMES);
                    config
                })
                .collect()
        });
        let index = LANGUAGES.iter().position(|l| l.name == name).unwrap();
        let mut highlighter = Highlighter::new();
        let events = highlighter
            .highlight(&configs[index], text.as_bytes(), None, None, |injected| {
                injected_language(injected, &languages).map(|i| &configs[i])
            })
            .unwrap();
        let mut lines: Vec<Vec<Span>> = vec![Vec::new()];
        let mut stack: Vec<usize> = Vec::new();
        let mut col = 0;
        for event in events {
            match event.unwrap() {
                HighlightEvent::HighlightStart(h) => stack.push(h.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    for c in text[start..end].chars() {
                        if c == '\n' {
                            lines.push(Vec::new());
                            col = 0;
                            continue;
                        }
                        if let Some(&highlight) = stack.last() {
                            let line = lines.last_mut().unwrap();
                            match line.last_mut() {
                                Some(s) if s.end == col && s.highlight == highlight => s.end += 1,
                                _ => line.push(Span {
                                    start: col,
                                    end: col + 1,
                                    highlight,
                                }),
                            }
                        }
                        col += 1;
                    }
                }
            }
        }
        lines
    }

    fn ours(file: &str, text: &str) -> Vec<Vec<Span>> {
        // Opening a file takes off one final newline.
        let doc = document(file, &format!("{text}\n"));
        with_engine(|engine| {
            let highlights = engine.highlights_default(&doc).unwrap();
            (0..doc.buffer().line_count())
                .map(|row| highlights.line(row).to_vec())
                .collect()
        })
    }

    /// Runs `f` with an engine whose queries are already compiled (slow in
    /// debug builds), holding nothing for other documents.
    fn with_engine<T>(f: impl FnOnce(&mut SyntaxEngine) -> T) -> T {
        thread_local! {
            static ENGINE: std::cell::RefCell<SyntaxEngine> = Default::default();
        }
        ENGINE.with_borrow_mut(|engine| {
            engine.retain(&[]);
            f(engine)
        })
    }

    /// Reports the first row where the two disagree.
    fn assert_same_as_reference(file: &str, language: &str, text: &str) {
        let expected = reference(language, text);
        let found = ours(file, text);
        for (row, line) in text.split('\n').enumerate() {
            let want = expected.get(row).cloned().unwrap_or_default();
            let got = found.get(row).cloned().unwrap_or_default();
            let show = |spans: &[Span]| -> Vec<(String, &str)> {
                let chars: Vec<char> = line.chars().collect();
                spans
                    .iter()
                    .map(|s| (chars[s.start..s.end].iter().collect(), s.name()))
                    .collect()
            };
            assert_eq!(show(&got), show(&want), "{file} row {row}: {line:?}");
        }
    }

    const SAMPLES: &[(&str, &str, &str)] = &[
        (
            "a.rs",
            "rust",
            "use std::fmt;\n/// Doc.\n#[derive(Debug)]\npub struct A<'a> { x: &'a str }\nimpl A<'_> {\n    fn f(&self, n: u32) -> Option<u32> {\n        let s = \"hi\\n\"; // c\n        println!(\"{s}\");\n        match n { 0 => None, _ => Some(n + 1) }\n    }\n}\n/* block\n comment */\n",
        ),
        (
            "a.toml",
            "toml",
            "[package]\nname = \"x\" # c\nv = [1, 2.5, true]\n[a.b]\nd = 1979-05-27\n",
        ),
        (
            "a.json",
            "json",
            "{\n  \"a\": [1, true, null, \"s\"],\n  \"b\": {\"c\": -1.5e3}\n}\n",
        ),
        (
            "a.py",
            "python",
            "import os\n@dec\nclass A(B):\n    def f(self, x=1, *a, **k):\n        \"\"\"Doc.\"\"\"\n        return f\"{x}\" if x else None  # c\n",
        ),
        (
            "a.js",
            "javascript",
            "import x from 'y';\nfunction f(a, b) {\n  const c = a + b;\n  return c.map((d) => d * 2);\n}\nclass K extends L { m() { return this.n; } }\nconst s = css`color: red;`;\nconst e = <div className=\"x\">{a}</div>;\n// c\n",
        ),
        (
            "a.ts",
            "typescript",
            "interface I { a: number }\nfunction f<T>(x: T, y: string): T {\n  const z: I = { a: 1 };\n  return x;\n}\nenum E { A, B }\ntype U = string | number;\n",
        ),
        (
            "a.tsx",
            "tsx",
            "const A = (p: { n: number }) => <b className=\"x\">{p.n}</b>;\nexport default A;\n",
        ),
        (
            "a.go",
            "go",
            "package main\nimport \"fmt\"\ntype T struct { A int `json:\"a\"` }\nfunc (t *T) F(n int) error {\n\tfor i := range n { fmt.Println(i) }\n\treturn nil // c\n}\n",
        ),
        (
            "a.c",
            "c",
            "#include <stdio.h>\n#define N 3\nstatic int f(const char *s) {\n  /* c */ return printf(\"%s\\n\", s);\n}\n",
        ),
        (
            "a.cpp",
            "cpp",
            "namespace n {\ntemplate <typename T> class A : public B {\npublic:\n  auto f() const -> T { return this->x; }\n  T x = nullptr;\n};\n}\n",
        ),
        (
            "a.sh",
            "bash",
            "#!/bin/bash\nf() { local x=\"$1\"; echo \"${x:-y}\" | grep -q z; }\nif [ -n \"$x\" ]; then f a; fi # c\n",
        ),
        (
            "a.md",
            "markdown",
            "# Title\n\nSome **bold**, *it*, `code` and [a link](http://x).\n\n- item\n\n```rust\nfn main() {}\n```\n\n<div>html</div>\n\n> quote\n",
        ),
        (
            "a.yml",
            "yaml",
            "a: 1\nb:\n  - \"s\"\n  - true # c\nc: &x {d: null}\n",
        ),
        (
            "a.html",
            "html",
            "<!DOCTYPE html>\n<html>\n<head><style>p { color: red; }</style></head>\n<body class=\"x\">\n<!-- c -->\n<script>\nfunction f(a) { return a; }\n</script>\n</body>\n</html>\n",
        ),
        (
            "a.css",
            "css",
            "@media (max-width: 1px) {\n  .a > #b:hover::after { color: #fff; margin: 0 1px !important; }\n}\n/* c */\n",
        ),
    ];

    #[test]
    fn highlights_match_tree_sitter_highlight() {
        for (file, language, text) in SAMPLES {
            assert_same_as_reference(file, language, text);
        }
    }

    #[test]
    fn real_files_match_tree_sitter_highlight() {
        let root = env!("CARGO_MANIFEST_DIR");
        let read = |path: &str| std::fs::read_to_string(format!("{root}/{path}")).unwrap();
        assert_same_as_reference("a.rs", "rust", &read("src/syntax/highlight.rs"));
        assert_same_as_reference("a.rs", "rust", &read("src/editor/transform.rs"));
        assert_same_as_reference("a.md", "markdown", &read("README.md"));
        assert_same_as_reference("a.md", "markdown", &read("docs/ARCHITECTURE.md"));
        assert_same_as_reference("a.toml", "toml", &read("src/config/default.toml"));
    }

    /// Runs `edits` (position, chars to delete, text to insert) on a
    /// document, checking after each that the kept tree is the tree a fresh
    /// parse gives and the highlights match fresh ones.
    fn check_edits(file: &str, text: &str, edits: &[(usize, usize, &str)]) {
        let mut doc = document(file, text);
        let mut engine = SyntaxEngine::default();
        engine.highlights_default(&doc);
        let languages = crate::config::Config::default().languages;
        let language = languages.detect(doc.path(), "").unwrap().to_string();
        let step = |doc: &Document, engine: &mut SyntaxEngine, what: &str| {
            let kept: Vec<Vec<Span>> = {
                let h = engine.highlights_default(doc).unwrap();
                (0..doc.buffer().line_count())
                    .map(|r| h.line(r).to_vec())
                    .collect()
            };
            let kept_tree = engine.trees.tree(doc, &language).unwrap();
            let kept_tree = kept_tree.root_node().to_sexp();
            let fresh_doc = Document::with_text(&doc.buffer().text());
            let tree = trees::parse_fresh(&language, fresh_doc.buffer()).unwrap();
            assert_eq!(kept_tree, tree.root_node().to_sexp(), "tree after {what}");
            assert_eq!(
                kept,
                ours(file, &doc.buffer().text()),
                "highlights after {what}"
            );
        };
        for &(at, delete, insert) in edits {
            let text = doc.buffer().text();
            let at = at.min(text.chars().count());
            let start = position_of(&text, at);
            let end = position_of(&text, (at + delete).min(text.chars().count()));
            doc.delete(start, end);
            doc.insert(start, insert);
            doc.commit();
            step(&doc, &mut engine, &format!("editing at {at}"));
        }
        while doc.undo() {
            step(&doc, &mut engine, "undo");
        }
    }

    fn position_of(text: &str, index: usize) -> crate::core::Position {
        let before: String = text.chars().take(index).collect();
        let row = before.matches('\n').count();
        let col = before.chars().rev().take_while(|&c| c != '\n').count();
        crate::core::Position::new(row, col)
    }

    #[test]
    fn kept_trees_follow_edits() {
        let (file, _, text) = SAMPLES[0];
        check_edits(
            file,
            text,
            &[
                (0, 0, "// é\n"),
                (40, 3, ""),
                (70, 0, "\"unclosed "),
                (70, 10, ""),
                (120, 0, "{\n{\n"),
                (5, 30, "x"),
                (1000, 0, "\nfn g() {}"),
            ],
        );
        let (file, _, text) = SAMPLES[11];
        check_edits(file, text, &[(10, 0, "```py\n"), (30, 2, "**"), (0, 2, "")]);
    }

    #[test]
    fn many_pseudo_random_edits() {
        let (file, _, text) = SAMPLES[4];
        let pieces = [
            "{", "}", "(", "\n", "'", "`", "x", "é ", "/*", "*/", "<a>", "",
        ];
        let mut seed: u64 = 7;
        let mut next = move |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        let edits: Vec<(usize, usize, &str)> = (0..40)
            .map(|_| (next(300), next(4), pieces[next(pieces.len())]))
            .collect();
        check_edits(file, text, &edits);
    }

    #[test]
    fn some_rows_highlight_as_they_do_in_the_whole_file() {
        for (file, _, text) in SAMPLES {
            let doc = document(file, text);
            let languages = crate::config::Config::default().languages;
            let whole = ours(file, text);
            for first in 0..doc.buffer().line_count() {
                with_engine(|engine| {
                    let trees = SyntaxTrees::default();
                    let some = engine
                        .highlight(&doc, &languages, &trees, first..first + 2)
                        .unwrap();
                    for (row, line) in whole.iter().enumerate().skip(first).take(2) {
                        assert_eq!(some.line(row), &line[..], "{file} row {row} from {first}");
                    }
                    assert!(some.line(first + 2).is_empty(), "only the rows asked for");
                });
            }
        }
    }

    #[test]
    fn highlights_are_kept_until_the_text_changes() {
        let mut engine = SyntaxEngine::default();
        let languages = crate::config::Config::default().languages;
        let trees = SyntaxTrees::default();
        let mut doc = document("a.rs", "fn a() {}\nfn b() {}\nfn c() {}\n");
        let first = engine.highlight(&doc, &languages, &trees, 0..3).unwrap() as *const Highlights;
        let again = engine.highlight(&doc, &languages, &trees, 1..2).unwrap() as *const Highlights;
        assert_eq!(first, again, "a cached range covering the rows is reused");
        doc.insert(crate::core::Position::new(0, 0), "pub ");
        let after = engine.highlight(&doc, &languages, &trees, 0..1).unwrap();
        assert_eq!(after.rows(), 0..1, "recomputed after an edit");
        assert_eq!(after.line(0)[0].name(), "keyword", "`pub`");
    }

    #[test]
    fn a_replaced_document_is_parsed_afresh() {
        let mut engine = SyntaxEngine::default();
        let mut doc = document("a.rs", "fn a() {}");
        engine.highlights_default(&doc);
        let replacement = Document::with_text("struct B;");
        // Same id, different buffer (as after a reload that swaps it).
        let text = replacement.buffer().clone();
        doc.delete(crate::core::Position::new(0, 0), doc.buffer().line_end(0));
        doc.insert(crate::core::Position::new(0, 0), &text.text());
        let names = names_on(&mut engine, &doc, 0);
        assert!(names.contains(&"keyword"), "{names:?}");
    }
}
