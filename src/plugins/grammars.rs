//! Language packs' tree-sitter grammars: compiled from the C source the
//! plugin carries, with the system's C compiler, and loaded while ferra
//! runs.
//!
//! ```toml
//! # ferra-plugin.toml
//! [grammars.zig]
//! path = "tree-sitter-zig"      # the folder with src/parser.c ("." if left out)
//! symbol = "tree_sitter_zig"    # (the default: tree_sitter_<language>)
//! queries = "queries/zig"       # highlights.scm, injections.scm, locals.scm
//! ```
//!
//! A build goes in the clone's `.ferra-build/`, named after the language
//! and the commit (`zig-4f1c2a…​.dylib`), so an updated plugin's grammar is
//! a different file: a process can't load a second library from a path it
//! already loaded. Building happens when a plugin is installed, updated or
//! restored (on a thread); loading builds first if the library is missing
//! (a C compiler installed since, say).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use serde::Deserialize;

use super::packages::Manifest;
use crate::syntax::{self, Grammar, LanguageDef};

/// Where builds go, inside the plugin's clone.
const BUILD_DIR: &str = ".ferra-build";

/// One `[grammars.<language>]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrammarSpec {
    /// The folder holding the grammar's `src/parser.c`, from the plugin's
    /// root.
    #[serde(default = "here")]
    pub path: String,
    /// The C function returning the language (`tree_sitter_<language>`).
    #[serde(default)]
    pub symbol: Option<String>,
    /// The folder with the `.scm` queries (`queries/<language>`).
    #[serde(default)]
    pub queries: Option<String>,
}

fn here() -> String {
    ".".into()
}

impl GrammarSpec {
    /// Fails if a path leaves the plugin's folder.
    pub fn check(&self, language: &str) -> Result<(), String> {
        for path in [Some(&self.path), self.queries.as_ref()]
            .into_iter()
            .flatten()
        {
            let inside = Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
            if !inside {
                return Err(format!(
                    "grammars.{language}: {path} should be a folder inside the plugin"
                ));
            }
        }
        if let Some(symbol) = &self.symbol
            && !symbol
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!("grammars.{language}: invalid symbol {symbol}"));
        }
        Ok(())
    }

    fn symbol(&self, language: &str) -> String {
        self.symbol
            .clone()
            .unwrap_or_else(|| format!("tree_sitter_{}", language.replace('-', "_")))
    }

    fn queries(&self, language: &str) -> String {
        self.queries
            .clone()
            .unwrap_or_else(|| format!("queries/{language}"))
    }
}

fn library_extension() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    }
}

fn head_commit(dir: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err("not a git clone".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Where `language`'s build for the clone's current commit goes.
fn library_path(dir: &Path, language: &str) -> Result<PathBuf, String> {
    let commit = head_commit(dir)?;
    let commit = &commit[..commit.len().min(12)];
    Ok(dir
        .join(BUILD_DIR)
        .join(format!("{language}-{commit}.{}", library_extension())))
}

/// Builds every grammar the plugin in `dir` carries, if not built for
/// its commit yet. Returns a problem per grammar that didn't build.
/// Blocking: it runs the C compiler.
pub fn build_all(dir: &Path, manifest: &Manifest) -> Vec<String> {
    manifest
        .grammars
        .iter()
        .filter_map(|(language, spec)| build(dir, language, spec).err())
        .map(|e| format!("{}: {e}", manifest.name))
        .collect()
}

/// Builds `language`'s grammar for the clone's current commit, unless
/// that's done. Returns the library.
pub fn build(dir: &Path, language: &str, spec: &GrammarSpec) -> Result<PathBuf, String> {
    let library = library_path(dir, language)?;
    if library.is_file() {
        return Ok(library);
    }
    let src = dir.join(&spec.path).join("src");
    let parser = src.join("parser.c");
    if !parser.is_file() {
        return Err(format!(
            "grammar {language}: no {}",
            Path::new(&spec.path).join("src/parser.c").display()
        ));
    }
    let build = dir.join(BUILD_DIR);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;
    keep_out_of_git(dir);
    // Older builds of this grammar aren't needed (a library already loaded
    // stays usable after its file is gone).
    if let Ok(entries) = std::fs::read_dir(&build) {
        let prefix = format!("{language}-");
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    let mut sources = vec![parser];
    let mut cpp = false;
    for scanner in ["scanner.c", "scanner.cc", "scanner.cpp"] {
        let path = src.join(scanner);
        if path.is_file() {
            cpp = scanner != "scanner.c";
            sources.push(path);
            break;
        }
    }
    let temp = build.join(format!("{language}.building"));
    let mut objects = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let is_cpp = source.extension().is_some_and(|e| e != "c");
        let object = build.join(format!("{language}.{i}.o"));
        let mut command = compiler(is_cpp);
        command.args(["-c", "-fPIC", "-O2"]);
        if !is_cpp {
            command.arg("-std=c11");
        }
        command
            .arg("-I")
            .arg(&src)
            .arg("-o")
            .arg(&object)
            .arg(source);
        run(command, is_cpp).inspect_err(|_| remove_all(&objects))?;
        objects.push(object);
    }
    let mut link = compiler(cpp);
    if cfg!(target_os = "macos") {
        link.args(["-dynamiclib", "-undefined", "dynamic_lookup"]);
    } else {
        link.arg("-shared");
    }
    link.arg("-o").arg(&temp).args(&objects);
    let linked = run(link, cpp);
    remove_all(&objects);
    linked?;
    std::fs::rename(&temp, &library).map_err(|e| format!("{}: {e}", library.display()))?;
    Ok(library)
}

/// `$CC` (or `cc`), or for C++ `$CXX` (or `c++`).
fn compiler(cpp: bool) -> Command {
    let (variable, default) = if cpp { ("CXX", "c++") } else { ("CC", "cc") };
    let program = std::env::var(variable)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.into());
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    command
}

fn run(mut command: Command, cpp: bool) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            let what = if cpp { "C++" } else { "C" };
            let hint = if cfg!(target_os = "macos") {
                "xcode-select --install"
            } else {
                "install gcc or clang"
            };
            format!("no {what} compiler ({program}) to build the grammar with ({hint})")
        } else {
            format!("{program}: {e}")
        }
    })?;
    if output.status.success() {
        return Ok(());
    }
    let errors = String::from_utf8_lossy(&output.stderr);
    let first = errors
        .lines()
        .find(|l| l.contains("error"))
        .or_else(|| errors.lines().next())
        .unwrap_or("failed");
    Err(format!("the grammar didn't compile: {}", first.trim()))
}

fn remove_all(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// Lists the build folder in the clone's `.git/info/exclude`, so the
/// clone stays clean for `git status` and updates.
fn keep_out_of_git(dir: &Path) {
    let exclude = dir.join(".git/info/exclude");
    let text = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !text.lines().any(|l| l.trim() == BUILD_DIR) {
        if let Some(parent) = exclude.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&exclude, format!("{text}{BUILD_DIR}\n"));
    }
}

/// What a loaded grammar is registered under, so loading the same build
/// with the same queries again (`:plugin_reload`) reuses it.
type Key = (PathBuf, String, String, String);

static LOADED: Mutex<Option<HashMap<Key, &'static LanguageDef>>> = Mutex::new(None);

/// Builds (if need be) and loads `language`'s grammar from the plugin in
/// `dir`, with its queries, and adds it to the languages ferra highlights.
pub fn load(dir: &Path, language: &str, spec: &GrammarSpec) -> Result<(), String> {
    if syntax::is_built_in(language) {
        return Err(format!(
            "grammars.{language}: ferra has a {language} grammar built in"
        ));
    }
    let library = build(dir, language, spec)?;
    let read = |name: &str| {
        let path = dir.join(spec.queries(language)).join(name);
        std::fs::read_to_string(path).unwrap_or_default()
    };
    let (highlights, injections, locals) = (
        read("highlights.scm"),
        read("injections.scm"),
        read("locals.scm"),
    );
    let key = (library.clone(), highlights, injections, locals);
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    let loaded = loaded.get_or_insert_with(HashMap::new);
    if let Some(definition) = loaded.get(&key) {
        // Registered already; make it the one this name means again.
        let again = LanguageDef {
            name: definition.name,
            grammar: definition.grammar,
            highlights: definition.highlights,
            injections: definition.injections,
            locals: definition.locals,
        };
        if syntax::language(language).is_none_or(|l| !std::ptr::eq(l, *definition)) {
            let definition = syntax::register(again);
            loaded.insert(key, definition);
        }
        return Ok(());
    }

    let grammar = open(&library, &spec.symbol(language))?;
    let leak = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
    let definition = LanguageDef {
        name: leak(language),
        grammar: Grammar::Loaded(grammar),
        highlights: Box::leak(vec![leak(&key.1)].into_boxed_slice()),
        injections: leak(&key.2),
        locals: leak(&key.3),
    };
    // The queries must fit the grammar: say so now, not on first draw.
    syntax::LanguageConfig::new(
        definition.name,
        definition.grammar(),
        &definition.highlight_query(),
        definition.injections,
        definition.locals,
    )?;
    let definition = syntax::register(definition);
    loaded.insert(key, definition);
    Ok(())
}

/// Opens the library and finds the language function, checking tree-sitter
/// can use it. The library stays loaded for good.
fn open(library: &Path, symbol: &str) -> Result<tree_sitter_language::LanguageFn, String> {
    // SAFETY: loading a library runs its initialisers: the grammar is code
    // the user chose to install, like a plugin's Lua.
    let lib = unsafe { libloading::Library::new(library) }
        .map_err(|e| format!("couldn't load {}: {e}", library.display()))?;
    // SAFETY: a tree-sitter grammar's language function has this type.
    let function = unsafe {
        lib.get::<unsafe extern "C" fn() -> *const ()>(symbol.as_bytes())
            .map(|f| *f)
    }
    .map_err(|_| format!("{} has no function {symbol}", library.display()))?;
    // Trees and queries point into the library: it's never unloaded.
    std::mem::forget(lib);
    // SAFETY: `function` is the grammar's language function, from a
    // library that stays loaded.
    let grammar = unsafe { tree_sitter_language::LanguageFn::from_raw(function) };
    let version = tree_sitter::Language::new(grammar).abi_version();
    let supported = tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION;
    if !supported.contains(&version) {
        return Err(format!(
            "the grammar is tree-sitter ABI {version}; this ferra reads {}-{}",
            supported.start(),
            supported.end()
        ));
    }
    Ok(grammar)
}
