//! The languages the editor can highlight, and how files map to them.
//!
//! Adding a language is one entry here plus its grammar crate.

use std::path::Path;

/// A language: how to recognise its files, and its tree-sitter grammar and
/// queries.
pub struct LanguageDef {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    /// Exact file names, for files without a telling extension.
    pub file_names: &'static [&'static str],
    pub grammar: fn() -> tree_sitter::Language,
    pub highlights: &'static str,
    pub injections: &'static str,
    pub locals: &'static str,
}

pub const LANGUAGES: &[LanguageDef] = &[
    LanguageDef {
        name: "rust",
        extensions: &["rs"],
        file_names: &[],
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        highlights: tree_sitter_rust::HIGHLIGHTS_QUERY,
        injections: tree_sitter_rust::INJECTIONS_QUERY,
        locals: "",
    },
    LanguageDef {
        name: "toml",
        extensions: &["toml"],
        file_names: &["Cargo.lock"],
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights: tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "json",
        extensions: &["json", "jsonc"],
        file_names: &[],
        grammar: || tree_sitter_json::LANGUAGE.into(),
        highlights: tree_sitter_json::HIGHLIGHTS_QUERY,
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "python",
        extensions: &["py", "pyi"],
        file_names: &[],
        grammar: || tree_sitter_python::LANGUAGE.into(),
        highlights: tree_sitter_python::HIGHLIGHTS_QUERY,
        injections: "",
        locals: "",
    },
    LanguageDef {
        name: "javascript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: &[],
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        highlights: tree_sitter_javascript::HIGHLIGHT_QUERY,
        injections: tree_sitter_javascript::INJECTIONS_QUERY,
        locals: tree_sitter_javascript::LOCALS_QUERY,
    },
];

/// The language for `path`, by file name or extension.
pub fn detect(path: &Path) -> Option<&'static LanguageDef> {
    let file_name = path.file_name()?.to_str()?;
    if let Some(language) = LANGUAGES.iter().find(|l| l.file_names.contains(&file_name)) {
        return Some(language);
    }
    let extension = path.extension()?.to_str()?;
    LANGUAGES.iter().find(|l| l.extensions.contains(&extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(path: &str) -> Option<&'static str> {
        detect(Path::new(path)).map(|l| l.name)
    }

    #[test]
    fn detects_by_extension_and_file_name() {
        assert_eq!(name("src/main.rs"), Some("rust"));
        assert_eq!(name("Cargo.toml"), Some("toml"));
        assert_eq!(name("Cargo.lock"), Some("toml"));
        assert_eq!(name("data.json"), Some("json"));
        assert_eq!(name("app.py"), Some("python"));
        assert_eq!(name("index.mjs"), Some("javascript"));
    }

    #[test]
    fn unknown_files_have_no_language() {
        assert_eq!(name("README.md"), None);
        assert_eq!(name("Makefile"), None);
        assert_eq!(name(""), None);
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = LANGUAGES.iter().map(|l| l.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LANGUAGES.len());
    }
}
