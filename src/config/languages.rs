//! Which language a file is in: the `[languages]` config table.
//!
//! Like Helix's `languages.toml`, each language lists the file extensions,
//! exact file names and `#!` interpreters that mean it. The grammars
//! themselves are built in (`syntax::LANGUAGES`); this table only decides
//! which one a file gets, so users can add an extension or a file name.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// How to recognise one language's files, and how to edit them.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct FileType {
    /// `rs`, `py`: without the dot.
    pub extensions: Vec<String>,
    /// Whole file names, for files whose extension doesn't tell
    /// (`Makefile`, `.bashrc`).
    pub file_names: Vec<String>,
    /// Interpreters in a `#!` first line (`bash`, `python`). Version
    /// suffixes match too: `python` covers `python3.12`.
    pub shebangs: Vec<String>,
    /// What starts a line comment (`//`, `#`), for `gc`.
    pub comment: Option<String>,
    /// What wraps a block comment (`["<!--", "-->"]`), for `gc` in
    /// languages without line comments.
    pub block_comment: Option<[String; 2]>,
    /// Options for this language's buffers (`tab-width`, `expand-tab`,
    /// `auto-indent`, `text-width`), over the `[editor]` ones; a buffer's
    /// own `:setlocal` values still win.
    pub options: toml::Table,
    /// The language server for this language's files, if any.
    pub language_server: Option<ServerConfig>,
}

/// How to run a language server (`language-server` in a `[languages]`
/// entry).
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ServerConfig {
    /// The program, found on `PATH` (`rust-analyzer`).
    pub command: String,
    pub args: Vec<String>,
    /// Files whose directory is the project root (`Cargo.toml`): the
    /// nearest one above the file. Without one, the file's own directory.
    pub roots: Vec<String>,
    /// The name the server knows the language by, if not the language's
    /// (`typescriptreact` for `tsx`).
    pub language_id: Option<String>,
    /// The server's settings (`{ json = { validate = { enable = true } } }`),
    /// sent when it starts and when it asks for a section of them.
    pub settings: toml::Table,
    /// `false` turns it off.
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

/// How `gc` comments a line: a prefix, and for block comments a suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentTokens {
    pub start: String,
    pub end: Option<String>,
}

/// Language name to how its files are recognised. Sorted, so when two
/// languages claim the same file the choice is always the same.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct Languages(BTreeMap<String, FileType>);

/// The built-in table, so an editor made without a config still knows its
/// languages.
impl Default for Languages {
    fn default() -> Self {
        super::Config::default().languages
    }
}

impl Languages {
    /// The language of a file at `path` whose first line is `first_line`:
    /// by exact file name, then extension, then `#!` interpreter.
    pub fn detect(&self, path: Option<&Path>, first_line: &str) -> Option<&str> {
        let file_name = path.and_then(Path::file_name).and_then(|n| n.to_str());
        if let Some(name) = file_name
            && let Some(language) = self.find(|t| t.file_names.iter().any(|f| f == name))
        {
            return Some(language);
        }
        let extension = path.and_then(Path::extension).and_then(|e| e.to_str());
        if let Some(extension) = extension
            && let Some(language) = self.find(|t| t.extensions.iter().any(|e| e == extension))
        {
            return Some(language);
        }
        let interpreter = shebang_interpreter(first_line)?;
        self.find(|t| {
            t.shebangs.iter().any(|s| {
                s == interpreter
                    || s == interpreter.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')
            })
        })
    }

    /// The language server `language`'s files use, if it has one that's on.
    pub fn server(&self, language: &str) -> Option<&ServerConfig> {
        self.0
            .get(language)?
            .language_server
            .as_ref()
            .filter(|s| s.enabled && !s.command.is_empty())
    }

    /// The options `language`'s buffers get (see [`FileType::options`]).
    pub fn options(&self, language: &str) -> Option<&toml::Table> {
        self.0
            .get(language)
            .map(|t| &t.options)
            .filter(|t| !t.is_empty())
    }

    /// Checks each language's options name buffer options with valid
    /// values, against `editor` (the config's own options: taking the
    /// defaults here would load the config being checked).
    pub fn check_options(&self, editor: &super::EditorOptions) -> Result<(), String> {
        use super::set;
        for (language, file_type) in &self.0 {
            for (name, value) in &file_type.options {
                if set::scope(name) != Some(set::Scope::Buffer) {
                    return Err(format!(
                        "languages.{language}.options: {name} can't be set per language \
                         (tab-width, expand-tab, auto-indent, text-width, auto-wrap and \
                         format-on-save can)"
                    ));
                }
                let mut options = editor.clone();
                set::set_value(&mut options, name, value.clone())
                    .map_err(|e| format!("languages.{language}.options: {e}"))?;
            }
        }
        Ok(())
    }

    fn find(&self, matches: impl Fn(&FileType) -> bool) -> Option<&str> {
        self.0
            .iter()
            .find(|(_, file_type)| matches(file_type))
            .map(|(name, _)| name.as_str())
    }

    /// How `language` comments a line: its line comment, or else its block
    /// comment around the line.
    pub fn comment_tokens(&self, language: &str) -> Option<CommentTokens> {
        let file_type = self.0.get(language)?;
        if let Some(start) = &file_type.comment {
            return Some(CommentTokens {
                start: start.clone(),
                end: None,
            });
        }
        let [start, end] = file_type.block_comment.clone()?;
        Some(CommentTokens {
            start,
            end: Some(end),
        })
    }

    /// Every configured language name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
}

/// The interpreter named by a `#!` line: `#!/bin/bash` and
/// `#!/usr/bin/env -S python3 -u` give `bash` and `python3`.
fn shebang_interpreter(first_line: &str) -> Option<&str> {
    let command = first_line.strip_prefix("#!")?;
    let mut words = command.split_whitespace();
    let program = words.next()?.rsplit('/').next()?;
    if program != "env" {
        return Some(program);
    }
    words
        .find(|word| !word.starts_with('-') && !word.contains('='))
        .and_then(|word| word.rsplit('/').next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn languages() -> Languages {
        let toml = r#"
            [rust]
            extensions = ["rs"]
            [python]
            extensions = ["py"]
            shebangs = ["python"]
            [bash]
            extensions = ["sh"]
            file-names = [".bashrc"]
            shebangs = ["bash", "sh"]
            [make]
            file-names = ["Makefile"]
        "#;
        toml::from_str(toml).unwrap()
    }

    fn detect(path: &str, first_line: &str) -> Option<String> {
        languages()
            .detect(Some(Path::new(path)), first_line)
            .map(str::to_string)
    }

    #[test]
    fn by_extension_and_file_name() {
        assert_eq!(detect("src/main.rs", "").as_deref(), Some("rust"));
        assert_eq!(detect("/home/me/.bashrc", "").as_deref(), Some("bash"));
        assert_eq!(detect("Makefile", "").as_deref(), Some("make"));
        assert_eq!(detect("notes.txt", "").as_deref(), None);
    }

    #[test]
    fn by_shebang() {
        assert_eq!(detect("deploy", "#!/bin/bash").as_deref(), Some("bash"));
        assert_eq!(
            detect("run", "#!/usr/bin/env python3").as_deref(),
            Some("python")
        );
        assert_eq!(
            detect("run", "#!/usr/bin/env -S python3.12 -u").as_deref(),
            Some("python"),
            "env flags and version suffixes"
        );
        assert_eq!(detect("run", "#!/bin/sh -e").as_deref(), Some("bash"));
        assert_eq!(detect("run", "#!/usr/bin/perl").as_deref(), None);
        assert_eq!(detect("run", "# just a comment").as_deref(), None);
    }

    #[test]
    fn the_extension_wins_over_the_shebang() {
        assert_eq!(detect("tool.py", "#!/bin/bash").as_deref(), Some("python"));
    }

    #[test]
    fn no_path_can_still_be_detected_by_shebang() {
        assert_eq!(languages().detect(None, "#!/bin/bash"), Some("bash"));
        assert_eq!(languages().detect(None, "text"), None);
    }

    #[test]
    fn shebang_interpreters() {
        assert_eq!(shebang_interpreter("#!/bin/bash"), Some("bash"));
        assert_eq!(shebang_interpreter("#! /usr/bin/env node"), Some("node"));
        assert_eq!(
            shebang_interpreter("#!/usr/bin/env FOO=1 ruby"),
            Some("ruby")
        );
        assert_eq!(shebang_interpreter("#!"), None);
        assert_eq!(shebang_interpreter("bash"), None);
    }
}
