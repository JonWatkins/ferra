//! Language packs' tree-sitter grammars: compiled from the C source the
//! plugin carries, with the system's C compiler, and loaded while ferra
//! runs.
//!
//! ```toml
//! # ferra-plugin.toml
//! [grammars.zig]
//! git = "https://github.com/tree-sitter-grammars/tree-sitter-zig"
//! rev = "6479aa13f32f701c383083d8b28360ebd682fb7d"  # the commit to build
//! path = "."                    # the folder with src/parser.c, in the grammar's repo
//! symbol = "tree_sitter_zig"    # (the default: tree_sitter_<language>)
//! queries = "queries/zig"       # highlights, injections, locals, indents (.scm)
//! ```
//!
//! With `git` and `rev` (as Helix's grammars are given), the grammar's
//! source is fetched at that commit when it's built, into
//! `.ferra-build/src/`, so a pack is only its settings and queries; without
//! them, `path` is a folder of the plugin itself. Queries come from the
//! plugin's `queries/<language>/`, else from the grammar's own `queries/`.
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
    /// The grammar's repository, fetched at `rev` when it's built.
    #[serde(default)]
    pub git: Option<String>,
    /// The commit of `git` to build (a full hash, so a build can't change
    /// under you).
    #[serde(default)]
    pub rev: Option<String>,
    /// The folder holding the grammar's `src/parser.c`: in its repository
    /// (`git`), else in the plugin.
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
        match (&self.git, &self.rev) {
            (Some(git), Some(rev)) => {
                let ok = |s: &str| {
                    !s.is_empty() && !s.starts_with('-') && !s.contains(char::is_whitespace)
                };
                if !ok(git) || !ok(rev) {
                    return Err(format!("grammars.{language}: invalid git or rev"));
                }
            }
            (Some(_), None) => {
                return Err(format!(
                    "grammars.{language}: git needs a rev (the commit to build)"
                ));
            }
            (None, Some(_)) => {
                return Err(format!("grammars.{language}: rev needs a git repository"));
            }
            (None, None) => {}
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
    let build = dir.join(BUILD_DIR);
    std::fs::create_dir_all(&build).map_err(|e| format!("{}: {e}", build.display()))?;
    keep_out_of_git(dir);
    let src = grammar_root(dir, language, spec)?
        .join(&spec.path)
        .join("src");
    let parser = src.join("parser.c");
    if !parser.is_file() {
        return Err(format!(
            "grammar {language}: no {}",
            Path::new(&spec.path).join("src/parser.c").display()
        ));
    }
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

/// Where the grammar's repository is: the plugin itself, or (with `git`)
/// a checkout of `rev`, fetched now if it isn't there.
fn grammar_root(dir: &Path, language: &str, spec: &GrammarSpec) -> Result<PathBuf, String> {
    let (Some(git), Some(rev)) = (&spec.git, &spec.rev) else {
        return Ok(dir.to_path_buf());
    };
    let checkout = source_checkout(dir, language, spec).expect("has git and rev");
    if checkout.join(".git").is_dir() {
        return Ok(checkout);
    }
    // Other revisions' sources aren't needed now.
    let sources = dir.join(BUILD_DIR).join("src");
    if let Ok(entries) = std::fs::read_dir(&sources) {
        let prefix = format!("{language}-");
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let temp = sources.join(format!("{language}.fetching"));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
    // Just that commit if the server allows it (GitHub does), else all
    // of it.
    let shallow = git_in(&temp, &["init", "--quiet"])
        .and_then(|_| git_in(&temp, &["fetch", "--quiet", "--depth", "1", "--", git, rev]))
        .and_then(|_| git_in(&temp, &["checkout", "--quiet", "FETCH_HEAD"]));
    if shallow.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
        let temp_arg = temp.to_string_lossy().into_owned();
        git_in(&sources, &["clone", "--quiet", "--", git, &temp_arg])
            .and_then(|_| git_in(&temp, &["checkout", "--quiet", rev]))
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&temp);
                format!("grammar {language}: couldn't get {git} at {rev}: {e}")
            })?;
    }
    std::fs::rename(&temp, &checkout).map_err(|e| format!("{}: {e}", checkout.display()))?;
    Ok(checkout)
}

/// Where a `git` grammar's source is checked out, by its revision.
fn source_checkout(dir: &Path, language: &str, spec: &GrammarSpec) -> Option<PathBuf> {
    spec.git.as_ref()?;
    let rev = spec.rev.as_ref()?;
    let short: String = rev
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect();
    Some(
        dir.join(BUILD_DIR)
            .join("src")
            .join(format!("{language}-{short}")),
    )
}

/// Runs git in `dir`, without prompts. Its last error line on failure.
fn git_in(dir: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let errors = String::from_utf8_lossy(&output.stderr);
    Err(errors
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("git failed")
        .trim()
        .trim_start_matches("fatal: ")
        .to_string())
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
    let what = if cpp { "C++" } else { "C" };
    let missing = || {
        let hint = match compiler_install(std::env::consts::OS, &os_release()) {
            Some(install) => install.command,
            None => "install gcc or clang".into(),
        };
        format!("no {what} compiler ({program}) to build the grammar with ({hint})")
    };
    let output = command.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            missing()
        } else {
            format!("{program}: {e}")
        }
    })?;
    if output.status.success() {
        return Ok(());
    }
    let errors = String::from_utf8_lossy(&output.stderr);
    // macOS's `cc` without the command line tools is a stub that says so.
    if errors.contains("xcrun: error") || errors.contains("invalid active developer path") {
        return Err(missing());
    }
    let first = errors
        .lines()
        .find(|l| l.contains("error"))
        .or_else(|| errors.lines().next())
        .unwrap_or("failed");
    Err(format!("the grammar didn't compile: {}", first.trim()))
}

/// Whether a grammar's error is that there's no compiler to build it.
pub fn needs_compiler(error: &str) -> bool {
    error.contains("no C compiler") || error.contains("no C++ compiler")
}

/// How to install a C compiler here: the command, and what it installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerInstall {
    pub command: String,
    pub what: String,
}

/// The command that installs a C compiler on `os` (`std::env::consts::OS`),
/// on Linux by the distribution `/etc/os-release` names (`ID`, then
/// `ID_LIKE`). `None` where it isn't known.
pub fn compiler_install(os: &str, os_release: &str) -> Option<CompilerInstall> {
    let install = |command: &str, what: &str| {
        Some(CompilerInstall {
            command: command.into(),
            what: what.into(),
        })
    };
    match os {
        "macos" => return install("xcode-select --install", "Apple's command line tools"),
        "linux" => {}
        _ => return None,
    }
    let field = |name: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    for id in ids.split_whitespace() {
        let found = match id {
            "debian" | "ubuntu" => {
                install("sudo apt install build-essential", "gcc (build-essential)")
            }
            "fedora" | "rhel" | "centos" => install("sudo dnf install gcc", "gcc"),
            "arch" => install("sudo pacman -S --needed base-devel", "gcc (base-devel)"),
            "alpine" => install("sudo apk add build-base", "gcc (build-base)"),
            "opensuse" | "suse" | "opensuse-tumbleweed" | "opensuse-leap" => {
                install("sudo zypper install gcc", "gcc")
            }
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// `/etc/os-release`, or nothing.
pub fn os_release() -> String {
    std::fs::read_to_string("/etc/os-release").unwrap_or_default()
}

fn remove_all(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// Lists the build folder in the clone's `info/exclude`, so the clone
/// stays clean for `git status` and updates. (`dir` may be a folder inside
/// the clone: git says where the file is.)
fn keep_out_of_git(dir: &Path) {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ])
        .stdin(Stdio::null())
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let exclude = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let text = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !text.lines().any(|l| l.trim() == BUILD_DIR) {
        if let Some(parent) = exclude.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&exclude, format!("{text}{BUILD_DIR}\n"));
    }
}

/// What a loaded grammar is registered under, so loading the same build
/// with the same queries again (`:plugin_reload`) reuses it: the library,
/// then the highlights, injections, locals and indents queries.
type Key = (PathBuf, String, String, String, String);

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
    // Each query from the plugin if it has it, else the grammar's own
    // (beside the grammar, or at the top of its repository, which can
    // hold several grammars).
    let mut folders = vec![dir.join(spec.queries(language))];
    if let Some(checkout) = source_checkout(dir, language, spec) {
        folders.push(checkout.join(&spec.path).join("queries"));
        folders.push(checkout.join("queries"));
    }
    // Written for another editor, most likely: see `syntax::compat`.
    let read = |name: &str| {
        let text = folders
            .iter()
            .find_map(|folder| std::fs::read_to_string(folder.join(name)).ok())
            .unwrap_or_default();
        syntax::compat::adapt(&text)
    };
    let key = (
        library.clone(),
        read("highlights.scm"),
        read("injections.scm"),
        read("locals.scm"),
        read("indents.scm"),
    );
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
            indents: definition.indents,
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
        indents: leak(&key.4),
    };
    // The queries must fit the grammar: say so now, not on first draw.
    syntax::LanguageConfig::new(
        definition.name,
        definition.grammar(),
        &definition.highlight_query(),
        definition.injections,
        definition.locals,
    )?;
    if !definition.indents.trim().is_empty() {
        tree_sitter::Query::new(&definition.grammar(), definition.indents)
            .map_err(|e| format!("{language}: invalid indents.scm: {e}"))?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilers_are_installed_by_platform() {
        let command = |os: &str, release: &str| compiler_install(os, release).map(|i| i.command);
        assert_eq!(
            command("macos", "").as_deref(),
            Some("xcode-select --install")
        );
        assert_eq!(
            command("linux", "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n").as_deref(),
            Some("sudo apt install build-essential")
        );
        assert_eq!(
            command("linux", "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n").as_deref(),
            Some("sudo apt install build-essential")
        );
        assert_eq!(
            command("linux", "ID=fedora\n").as_deref(),
            Some("sudo dnf install gcc")
        );
        assert_eq!(
            command("linux", "ID=endeavouros\nID_LIKE=arch\n").as_deref(),
            Some("sudo pacman -S --needed base-devel")
        );
        assert_eq!(
            command("linux", "ID=alpine\n").as_deref(),
            Some("sudo apk add build-base")
        );
        assert_eq!(command("linux", "ID=gentoo\n"), None);
        assert_eq!(command("linux", ""), None);
        assert_eq!(command("windows", ""), None);
    }

    #[test]
    fn macos_compiler_stubs_count_as_missing() {
        // What /usr/bin/cc does on a Mac without the command line tools.
        let mut stub = Command::new("sh");
        stub.args([
            "-c",
            "echo 'xcrun: error: invalid active developer path (/Library/Developer/CommandLineTools)' >&2; exit 1",
        ]);
        let error = run(stub, false).unwrap_err();
        assert!(needs_compiler(&error), "{error}");
        let mut broken = Command::new("sh");
        broken.args(["-c", "echo 'parser.c:1:1: error: expected' >&2; exit 1"]);
        let error = run(broken, false).unwrap_err();
        assert!(!needs_compiler(&error), "{error}");
    }

    #[test]
    fn missing_compilers_are_told_apart() {
        assert!(needs_compiler(
            "zig: no C compiler (cc) to build the grammar with (x)"
        ));
        assert!(!needs_compiler(
            "zig: the grammar didn't compile: parser.c:1: error"
        ));
    }
}
