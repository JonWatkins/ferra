//! File and folder icons, for a Nerd Font (`icons = "nerd"`): the Seti UI
//! set VS Code's file icons come from, coloured as Neovim's
//! nvim-web-devicons colours them. Code points are checked against Nerd
//! Fonts' `glyphnames.json` (`seti-*`, `custom-*`, `dev-*`, `md-*`).
//!
//! A file is matched by its exact name first (`Cargo.toml`, `Dockerfile`),
//! then its extension; then a language's own icon (`icon` in its
//! `[languages]` entry, which language packs can set); else a plain file.

use std::path::Path;
use std::str::FromStr;

use ratatui::style::Color;

use crate::config::Languages;

/// An icon: its glyph and colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    pub glyph: &'static str,
    pub color: Color,
}

const fn icon(glyph: &'static str, rgb: u32) -> Icon {
    Icon {
        glyph,
        color: Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
    }
}

/// `custom-folder` and `custom-folder_open`.
pub const FOLDER: &str = "\u{e5ff}";
pub const FOLDER_OPEN: &str = "\u{e5fe}";

/// `seti-default`.
const FILE: Icon = icon("\u{e64e}", 0x6d8086);

const RUST: Icon = icon("\u{e68b}", 0xdea584);
const PYTHON: Icon = icon("\u{e606}", 0xffbc03);
const JAVASCRIPT: Icon = icon("\u{e60c}", 0xcbcb41);
const TYPESCRIPT: Icon = icon("\u{e628}", 0x519aba);
const REACT: Icon = icon("\u{e7ba}", 0x20c2e3);
const GO: Icon = icon("\u{e627}", 0x00add8);
const C: Icon = icon("\u{e649}", 0x599eff);
const HEADER: Icon = icon("\u{e649}", 0xa074c4);
const CPP: Icon = icon("\u{e646}", 0x519aba);
const HTML: Icon = icon("\u{e60e}", 0xe44d26);
const CSS: Icon = icon("\u{e614}", 0x42a5f5);
const SASS: Icon = icon("\u{e603}", 0xf55385);
const JSON: Icon = icon("\u{e60b}", 0xcbcb41);
const TOML: Icon = icon("\u{e6b2}", 0x9c4221);
const YAML: Icon = icon("\u{e6a8}", 0x6d8086);
const MARKDOWN: Icon = icon("\u{e609}", 0xdddddd);
const LUA: Icon = icon("\u{e620}", 0x51a0cf);
const RUBY: Icon = icon("\u{e605}", 0xcc3e44);
const JAVA: Icon = icon("\u{e66d}", 0xcc3e44);
const PHP: Icon = icon("\u{e608}", 0xa074c4);
const ELIXIR: Icon = icon("\u{e62d}", 0xa074c4);
const NIX: Icon = icon("\u{f1105}", 0x7ebae4);
const ZIG: Icon = icon("\u{e6a9}", 0xf69a1b);
const SHELL: Icon = icon("\u{e691}", 0x89e051);
const DOCKER: Icon = icon("\u{e650}", 0x458ee6);
const GIT: Icon = icon("\u{e65d}", 0xf14c28);
const LOCK: Icon = icon("\u{e672}", 0xbbbbbb);
const LICENSE: Icon = icon("\u{e60a}", 0xd0bf41);
const MAKEFILE: Icon = icon("\u{e673}", 0x6d8086);
const SVG: Icon = icon("\u{e698}", 0xffb13b);
const IMAGE: Icon = icon("\u{e60d}", 0xa074c4);
const XML: Icon = icon("\u{e619}", 0xe37933);
const CSV: Icon = icon("\u{e64a}", 0x89e051);
const VUE: Icon = icon("\u{e6a0}", 0x8dc149);
const SVELTE: Icon = icon("\u{e697}", 0xff3e00);
const KOTLIN: Icon = icon("\u{e634}", 0x7f52ff);
const SWIFT: Icon = icon("\u{e699}", 0xe37933);
const HASKELL: Icon = icon("\u{e61f}", 0xa074c4);
const OCAML: Icon = icon("\u{e67a}", 0xe37933);
const SCALA: Icon = icon("\u{e68e}", 0xcc3e44);
const CLOJURE: Icon = icon("\u{e642}", 0x8dc149);
const DART: Icon = icon("\u{e64c}", 0x03589c);
const JULIA: Icon = icon("\u{e624}", 0xa270ba);
const R: Icon = icon("\u{e68a}", 0x2266ba);
const PERL: Icon = icon("\u{e67e}", 0x519aba);
const ELM: Icon = icon("\u{e62c}", 0x519aba);
const TEX: Icon = icon("\u{e69b}", 0x89e051);
const PDF: Icon = icon("\u{e67d}", 0xb30b00);
const ZIP: Icon = icon("\u{e6aa}", 0xeca517);
const VIDEO: Icon = icon("\u{e69f}", 0xfd971f);
const AUDIO: Icon = icon("\u{e638}", 0x00afff);
const FONT: Icon = icon("\u{e659}", 0xececec);
const NPM: Icon = icon("\u{e616}", 0xe8274b);
const GRADLE: Icon = icon("\u{e660}", 0x02303a);
const MAVEN: Icon = icon("\u{e674}", 0xe37933);
const EDITORCONFIG: Icon = icon("\u{e652}", 0xfff2f2);
const TERRAFORM: Icon = icon("\u{e69a}", 0x5f43e9);
const GRAPHQL: Icon = icon("\u{e662}", 0xe535ab);
const PRISMA: Icon = icon("\u{e684}", 0x5a67d8);
const WASM: Icon = icon("\u{e6a1}", 0x5c4cdb);
const DATABASE: Icon = icon("\u{e706}", 0xdad8d8);
const CONFIG: Icon = icon("\u{e615}", 0x6d8086);
const TEXT: Icon = icon("\u{f15c}", 0x89e051);
const TODO: Icon = icon("\u{e69c}", 0x7fb2ff);

/// Whole file names (matched before extensions, case as on disk).
fn by_name(name: &str) -> Option<Icon> {
    Some(match name {
        "Cargo.toml" | "Cargo.lock" => RUST,
        "Dockerfile"
        | "Containerfile"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yml"
        | "compose.yaml"
        | ".dockerignore" => DOCKER,
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" | ".git" => GIT,
        "Makefile" | "makefile" | "GNUmakefile" | "CMakeLists.txt" => MAKEFILE,
        "package.json" | "package-lock.json" | ".npmrc" | ".npmignore" => NPM,
        "LICENSE" | "LICENSE.md" | "LICENSE.txt" | "COPYING" => LICENSE,
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => GRADLE,
        "pom.xml" => MAVEN,
        ".editorconfig" => EDITORCONFIG,
        "TODO" | "TODO.md" => TODO,
        "flake.lock" => NIX,
        ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile" | ".profile" => SHELL,
        "Gemfile" | "Rakefile" => RUBY,
        _ if name.ends_with(".lock") || name.ends_with("-lock.json") => LOCK,
        _ => return None,
    })
}

/// File extensions (lowercased).
fn by_extension(extension: &str) -> Option<Icon> {
    Some(match extension {
        "rs" => RUST,
        "py" | "pyi" | "pyw" => PYTHON,
        "js" | "mjs" | "cjs" => JAVASCRIPT,
        "ts" | "mts" | "cts" => TYPESCRIPT,
        "jsx" | "tsx" => REACT,
        "go" => GO,
        "c" => C,
        "h" => HEADER,
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" => CPP,
        "html" | "htm" => HTML,
        "css" => CSS,
        "scss" | "sass" => SASS,
        "json" | "jsonc" | "json5" => JSON,
        "toml" => TOML,
        "yaml" | "yml" => YAML,
        "md" | "markdown" => MARKDOWN,
        "lua" => LUA,
        "rb" | "rake" | "gemspec" | "ru" => RUBY,
        "java" => JAVA,
        "php" | "phtml" => PHP,
        "ex" | "exs" | "heex" => ELIXIR,
        "nix" => NIX,
        "zig" | "zon" => ZIG,
        "sh" | "bash" | "zsh" | "fish" => SHELL,
        "dockerfile" => DOCKER,
        "lock" => LOCK,
        "svg" => SVG,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => IMAGE,
        "xml" => XML,
        "csv" | "tsv" => CSV,
        "vue" => VUE,
        "svelte" => SVELTE,
        "kt" | "kts" => KOTLIN,
        "swift" => SWIFT,
        "hs" | "lhs" => HASKELL,
        "ml" | "mli" => OCAML,
        "scala" | "sc" => SCALA,
        "clj" | "cljs" | "cljc" | "edn" => CLOJURE,
        "dart" => DART,
        "jl" => JULIA,
        "r" => R,
        "pl" | "pm" => PERL,
        "elm" => ELM,
        "tex" | "bib" => TEX,
        "pdf" => PDF,
        "zip" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "tar" | "7z" | "rar" => ZIP,
        "mp4" | "mov" | "mkv" | "webm" | "avi" => VIDEO,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => AUDIO,
        "ttf" | "otf" | "woff" | "woff2" => FONT,
        "tf" | "tfvars" => TERRAFORM,
        "graphql" | "gql" => GRAPHQL,
        "prisma" => PRISMA,
        "wasm" | "wat" => WASM,
        "sql" | "db" | "sqlite" | "sqlite3" => DATABASE,
        "ini" | "cfg" | "conf" | "env" | "properties" => CONFIG,
        "txt" | "log" => TEXT,
        _ => return None,
    })
}

/// The icon for a file at `path` (its name and extension), else its
/// language's own (from `languages`), else a plain file's. The glyph is
/// owned because a language's comes from config.
pub fn for_file(path: &Path, languages: &Languages) -> (String, Color) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if let Some(icon) = by_name(&name).or_else(|| by_extension(&extension)) {
        return (icon.glyph.to_string(), icon.color);
    }
    if let Some((glyph, color)) = languages
        .detect(Some(path), "")
        .and_then(|language| languages.icon(language))
    {
        let color = color
            .and_then(|c| Color::from_str(c).ok())
            .unwrap_or(FILE.color);
        return (glyph.to_string(), color);
    }
    (FILE.glyph.to_string(), FILE.color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(name: &str) -> String {
        for_file(Path::new(name), &Languages::default()).0
    }

    #[test]
    fn files_by_name_then_extension() {
        assert_eq!(glyph("src/main.rs"), RUST.glyph);
        assert_eq!(glyph("Cargo.toml"), RUST.glyph, "by name before .toml");
        assert_eq!(glyph("config.toml"), TOML.glyph);
        assert_eq!(glyph("App.TSX"), REACT.glyph, "extensions ignore case");
        assert_eq!(glyph("Dockerfile"), DOCKER.glyph);
        assert_eq!(glyph("yarn.lock"), LOCK.glyph);
        assert_eq!(glyph("notes"), FILE.glyph);
    }

    #[test]
    fn a_languages_own_icon_for_files_the_table_doesnt_know() {
        let languages = Languages::from_map(
            toml::from_str(
                r##"
                [gleam]
                extensions = ["gleam"]
                icon = "★"
                icon-color = "#ffaff3"
                "##,
            )
            .unwrap(),
        );
        let (glyph, color) = for_file(Path::new("a.gleam"), &languages);
        assert_eq!((glyph.as_str(), color), ("★", Color::Rgb(0xff, 0xaf, 0xf3)));
    }
}
