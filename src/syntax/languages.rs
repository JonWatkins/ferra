//! The languages the editor can highlight: each one's tree-sitter grammar
//! and queries, by name. Which files are in which language is config
//! (`[languages]`, see `config::Languages`).
//!
//! Adding a language is one entry here, its grammar crate, and a
//! `[languages.name]` table in the default config.
//!
//! Plugins' language packs add grammars while ferra runs ([`register`]):
//! the built-ins come first, then those, in the order they were added. A
//! language's position never changes, so it can index caches; a grammar
//! registered again under the same name (an updated plugin) gets a new
//! position, and wins.

use std::sync::RwLock;

/// Where a language's grammar comes from.
#[derive(Clone, Copy)]
pub enum Grammar {
    /// A grammar crate built into ferra.
    BuiltIn(fn() -> tree_sitter::Language),
    /// A plugin's compiled grammar, loaded from a shared library that's
    /// never unloaded.
    Loaded(tree_sitter_language::LanguageFn),
}

/// A language: its grammar and queries.
pub struct LanguageDef {
    pub name: &'static str,
    pub grammar: Grammar,
    /// Highlight queries, joined in order. A language built on another
    /// (TypeScript on JavaScript, C++ on C) lists its own first, so its
    /// patterns win, then the base language's.
    pub highlights: &'static [&'static str],
    pub injections: &'static str,
    pub locals: &'static str,
    /// Indentation rules (`@indent`, `@outdent`; see `indent.rs`) for a
    /// plugin's language; the built-ins' are in `indent.rs`.
    pub indents: &'static str,
}

impl LanguageDef {
    pub fn grammar(&self) -> tree_sitter::Language {
        match self.grammar {
            Grammar::BuiltIn(grammar) => grammar(),
            Grammar::Loaded(grammar) => tree_sitter::Language::new(grammar),
        }
    }

    /// The highlight queries as one query.
    pub fn highlight_query(&self) -> String {
        self.highlights.join("\n")
    }
}

/// The upstream Markdown injections, with `include-children` on the inline
/// one: the `inline` node has hidden children, and without it the inline
/// grammar is given nothing to parse.
const MARKDOWN_INJECTIONS: &str = r#"
(fenced_code_block
  (info_string (language) @injection.language)
  (code_fence_content) @injection.content)

((html_block) @injection.content
  (#set! injection.language "html"))

((minus_metadata) @injection.content
  (#set! injection.language "yaml"))

((plus_metadata) @injection.content
  (#set! injection.language "toml"))

((inline) @injection.content
  (#set! injection.language "markdown_inline")
  (#set! injection.include-children))
"#;

/// The languages built into ferra: few, so it highlights the files it's
/// most often used on (and its own config) with no plugins at all. The
/// rest are language packs.
const CORE: &[LanguageDef] = &[
    LanguageDef {
        name: "toml",
        grammar: Grammar::BuiltIn(|| tree_sitter_toml_ng::LANGUAGE.into()),
        highlights: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "json",
        grammar: Grammar::BuiltIn(|| tree_sitter_json::LANGUAGE.into()),
        highlights: &[tree_sitter_json::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "bash",
        grammar: Grammar::BuiltIn(|| tree_sitter_bash::LANGUAGE.into()),
        highlights: &[tree_sitter_bash::HIGHLIGHT_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        // Block structure only (headings, lists, code fences, quotes):
        // emphasis and links are a second grammar reached by injection,
        // which is off for now.
        name: "markdown",
        grammar: Grammar::BuiltIn(|| tree_sitter_md::LANGUAGE.into()),
        highlights: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
        injections: MARKDOWN_INJECTIONS,
        locals: "",
        indents: "",
    },
    LanguageDef {
        // Markdown's inline syntax (emphasis, links, code spans): a second
        // grammar the block grammar injects into each paragraph.
        name: "markdown_inline",
        grammar: Grammar::BuiltIn(|| tree_sitter_md::INLINE_LANGUAGE.into()),
        highlights: &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
        injections: tree_sitter_md::INJECTION_QUERY_INLINE,
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "yaml",
        grammar: Grammar::BuiltIn(|| tree_sitter_yaml::LANGUAGE.into()),
        highlights: &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
];

/// Languages that were built in and are language packs now. ferra's own
/// tests still use them (the `test-languages` feature).
#[cfg(feature = "test-languages")]
const MOVED: &[LanguageDef] = &[
    LanguageDef {
        name: "rust",
        grammar: Grammar::BuiltIn(|| tree_sitter_rust::LANGUAGE.into()),
        highlights: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
        // Macro calls' and `macro_rules!` bodies are highlighted as Rust.
        injections: tree_sitter_rust::INJECTIONS_QUERY,
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "python",
        grammar: Grammar::BuiltIn(|| tree_sitter_python::LANGUAGE.into()),
        highlights: &[tree_sitter_python::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "javascript",
        grammar: Grammar::BuiltIn(|| tree_sitter_javascript::LANGUAGE.into()),
        highlights: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        injections: tree_sitter_javascript::INJECTIONS_QUERY,
        locals: tree_sitter_javascript::LOCALS_QUERY,
        indents: "",
    },
    LanguageDef {
        name: "typescript",
        grammar: Grammar::BuiltIn(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        highlights: &[
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        ],
        injections: "",
        locals: tree_sitter_typescript::LOCALS_QUERY,
        indents: "",
    },
    LanguageDef {
        name: "tsx",
        grammar: Grammar::BuiltIn(|| tree_sitter_typescript::LANGUAGE_TSX.into()),
        highlights: &[
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        injections: "",
        locals: tree_sitter_typescript::LOCALS_QUERY,
        indents: "",
    },
    LanguageDef {
        name: "go",
        grammar: Grammar::BuiltIn(|| tree_sitter_go::LANGUAGE.into()),
        highlights: &[tree_sitter_go::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "c",
        grammar: Grammar::BuiltIn(|| tree_sitter_c::LANGUAGE.into()),
        highlights: &[tree_sitter_c::HIGHLIGHT_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "cpp",
        grammar: Grammar::BuiltIn(|| tree_sitter_cpp::LANGUAGE.into()),
        highlights: &[
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            tree_sitter_c::HIGHLIGHT_QUERY,
        ],
        injections: "",
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "html",
        grammar: Grammar::BuiltIn(|| tree_sitter_html::LANGUAGE.into()),
        highlights: &[tree_sitter_html::HIGHLIGHTS_QUERY],
        injections: tree_sitter_html::INJECTIONS_QUERY,
        locals: "",
        indents: "",
    },
    LanguageDef {
        name: "css",
        grammar: Grammar::BuiltIn(|| tree_sitter_css::LANGUAGE.into()),
        highlights: &[tree_sitter_css::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
        indents: "",
    },
];

/// The built-in languages (with the moved ones when testing).
pub static LANGUAGES: std::sync::LazyLock<Vec<&'static LanguageDef>> =
    std::sync::LazyLock::new(|| {
        let languages = CORE.iter();
        #[cfg(feature = "test-languages")]
        let languages = languages.chain(MOVED.iter());
        languages.collect()
    });

/// Languages only ever reached by injection from another one, so they have
/// no `[languages]` entry.
pub const INJECTION_ONLY: &[&str] = &["markdown_inline"];

/// Languages plugins added, in order. Leaked: they live as long as ferra
/// (trees and compiled queries point into their grammars).
static ADDED: RwLock<Vec<&'static LanguageDef>> = RwLock::new(Vec::new());

/// Adds a plugin's language, after the built-ins and earlier ones.
pub fn register(definition: LanguageDef) -> &'static LanguageDef {
    let definition: &'static LanguageDef = Box::leak(Box::new(definition));
    ADDED
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(definition);
    definition
}

fn added() -> std::sync::RwLockReadGuard<'static, Vec<&'static LanguageDef>> {
    ADDED.read().unwrap_or_else(|e| e.into_inner())
}

/// How many languages there are, built in and added.
pub fn count() -> usize {
    LANGUAGES.len() + added().len()
}

/// The language at `index` (see [`position`]).
pub fn get(index: usize) -> Option<&'static LanguageDef> {
    LANGUAGES
        .get(index)
        .copied()
        .or_else(|| added().get(index - LANGUAGES.len()).copied())
}

/// Where language `name` is: a built-in's, else the last added under it.
pub fn position(name: &str) -> Option<usize> {
    if let Some(index) = LANGUAGES.iter().position(|l| l.name == name) {
        return Some(index);
    }
    let added = added();
    added
        .iter()
        .rposition(|l| l.name == name)
        .map(|i| LANGUAGES.len() + i)
}

/// The language called `name` (see [`position`]).
pub fn language(name: &str) -> Option<&'static LanguageDef> {
    get(position(name)?)
}

/// Whether ferra has a grammar built in for `name`.
pub fn is_built_in(name: &str) -> bool {
    LANGUAGES.iter().any(|l| l.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = LANGUAGES.iter().map(|l| l.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LANGUAGES.len());
    }

    #[test]
    fn every_grammar_has_a_language_in_the_default_config() {
        // The config can also list languages without a grammar (make,
        // git-commit): recognised for comments and options only.
        let config = Config::default();
        let configured: Vec<&str> = config.languages.names().collect();
        for name in LANGUAGES
            .iter()
            .map(|l| l.name)
            .filter(|name| !INJECTION_ONLY.contains(name))
        {
            assert!(configured.contains(&name), "{name} isn't in [languages]");
        }
    }

    #[test]
    fn default_detection() {
        let config = Config::default();
        let detect = |path: &str, first: &str| {
            config
                .languages
                .detect(Some(std::path::Path::new(path)), first)
                .map(str::to_string)
        };
        for (path, expected) in [
            ("src/main.rs", "rust"),
            ("Cargo.toml", "toml"),
            ("Cargo.lock", "toml"),
            ("app.py", "python"),
            ("index.mjs", "javascript"),
            ("app.ts", "typescript"),
            ("App.tsx", "tsx"),
            ("main.go", "go"),
            ("lib.h", "c"),
            ("lib.hpp", "cpp"),
            ("install.sh", "bash"),
            (".bashrc", "bash"),
            ("README.md", "markdown"),
            ("ci.yml", "yaml"),
            ("index.html", "html"),
            ("site.css", "css"),
        ] {
            assert_eq!(detect(path, "").as_deref(), Some(expected), "{path}");
        }
        assert_eq!(
            detect("deploy", "#!/usr/bin/env bash").as_deref(),
            Some("bash")
        );
        assert_eq!(detect("notes.txt", "").as_deref(), None);
    }
}
