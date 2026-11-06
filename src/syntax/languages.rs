//! The languages the editor can highlight: each one's tree-sitter grammar
//! and queries, by name. Which files are in which language is config
//! (`[languages]`, see `config::Languages`).
//!
//! Adding a language is one entry here, its grammar crate, and a
//! `[languages.name]` table in the default config.

/// A built-in language: its grammar and queries.
pub struct LanguageDef {
    pub name: &'static str,
    pub grammar: fn() -> tree_sitter::Language,
    /// Highlight queries, joined in order. A language built on another
    /// (TypeScript on JavaScript, C++ on C) lists its own first, so its
    /// patterns win, then the base language's.
    pub highlights: &'static [&'static str],
    pub injections: &'static str,
    pub locals: &'static str,
}

impl LanguageDef {
    /// The highlight queries as one query.
    pub fn highlight_query(&self) -> String {
        self.highlights.join("\n")
    }
}

pub const LANGUAGES: &[LanguageDef] = &[
    LanguageDef {
        name: "rust",
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        highlights: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
        injections: tree_sitter_rust::INJECTIONS_QUERY,
        locals: "",
    },
    LanguageDef {
        name: "toml",
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "json",
        grammar: || tree_sitter_json::LANGUAGE.into(),
        highlights: &[tree_sitter_json::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "python",
        grammar: || tree_sitter_python::LANGUAGE.into(),
        highlights: &[tree_sitter_python::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "javascript",
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        highlights: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        injections: tree_sitter_javascript::INJECTIONS_QUERY,
        locals: tree_sitter_javascript::LOCALS_QUERY,
    },
    LanguageDef {
        name: "typescript",
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlights: &[
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        ],
        injections: "",
        locals: tree_sitter_typescript::LOCALS_QUERY,
    },
    LanguageDef {
        name: "tsx",
        grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlights: &[
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        injections: "",
        locals: tree_sitter_typescript::LOCALS_QUERY,
    },
    LanguageDef {
        name: "go",
        grammar: || tree_sitter_go::LANGUAGE.into(),
        highlights: &[tree_sitter_go::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "c",
        grammar: || tree_sitter_c::LANGUAGE.into(),
        highlights: &[tree_sitter_c::HIGHLIGHT_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "cpp",
        grammar: || tree_sitter_cpp::LANGUAGE.into(),
        highlights: &[
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            tree_sitter_c::HIGHLIGHT_QUERY,
        ],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "bash",
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        highlights: &[tree_sitter_bash::HIGHLIGHT_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        // Block structure only (headings, lists, code fences, quotes):
        // emphasis and links are a second grammar reached by injection,
        // which is off for now.
        name: "markdown",
        grammar: || tree_sitter_md::LANGUAGE.into(),
        highlights: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "yaml",
        grammar: || tree_sitter_yaml::LANGUAGE.into(),
        highlights: &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "html",
        grammar: || tree_sitter_html::LANGUAGE.into(),
        highlights: &[tree_sitter_html::HIGHLIGHTS_QUERY],
        injections: tree_sitter_html::INJECTIONS_QUERY,
        locals: "",
    },
    LanguageDef {
        name: "css",
        grammar: || tree_sitter_css::LANGUAGE.into(),
        highlights: &[tree_sitter_css::HIGHLIGHTS_QUERY],
        injections: "",
        locals: "",
    },
];

/// The built-in language called `name`.
pub fn language(name: &str) -> Option<&'static LanguageDef> {
    LANGUAGES.iter().find(|l| l.name == name)
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
    fn the_default_config_and_the_grammars_list_the_same_languages() {
        let config = Config::default();
        let mut configured: Vec<&str> = config.languages.names().collect();
        let mut built_in: Vec<&str> = LANGUAGES.iter().map(|l| l.name).collect();
        configured.sort_unstable();
        built_in.sort_unstable();
        assert_eq!(configured, built_in);
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
