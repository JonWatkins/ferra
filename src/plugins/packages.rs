//! Plugin packages: git repositories with a `ferra-plugin.toml` at the
//! root, cloned into the data folder (`~/.local/share/ferra/plugins/<name>`).
//!
//! What's installed is kept in two files in the config folder, so dotfiles
//! reproduce a setup:
//!
//! ```toml
//! # plugins.toml: what you asked for
//! [plugins.todos]
//! source = "someone/ferra-todos"
//! tag = "v0.2.0"                  # if you asked for one
//!
//! # plugins.lock: the exact commit of each
//! [plugins.todos]
//! commit = "4f1c…"
//! ```
//!
//! Everything here is plain file and git work with no editor state, so the
//! clone can run on a thread.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// The manifest at a plugin's root.
pub const MANIFEST: &str = "ferra-plugin.toml";
const LIST_FILE: &str = "plugins.toml";
const LOCK_FILE: &str = "plugins.lock";

/// Where plugin packages and their records live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPaths {
    /// The config folder: `plugins.toml` and `plugins.lock`.
    pub config: PathBuf,
    /// Where the clones go, one folder per plugin.
    pub data: PathBuf,
}

impl PluginPaths {
    /// The usual places: ferra's config folder, and
    /// `$XDG_DATA_HOME/ferra/plugins` (or `~/.local/share/ferra/plugins`).
    pub fn standard() -> Option<Self> {
        let config = crate::config::config_dir()?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })?
            .join("ferra")
            .join("plugins");
        Some(Self { config, data })
    }

    /// Where plugin `name` is cloned.
    pub fn dir(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }
}

/// `ferra-plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// The ferra versions it works with (`">=0.1"`); any if left out.
    #[serde(default)]
    pub ferra: Option<String>,
    /// The file loaded first, relative to the plugin's root.
    #[serde(default = "default_main")]
    pub main: String,
}

fn default_main() -> String {
    "init.lua".into()
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self =
            toml::from_str(text).map_err(|e| format!("{MANIFEST}: {}", e.message()))?;
        check_name(&manifest.name).map_err(|e| format!("{MANIFEST}: {e}"))?;
        let main = Path::new(&manifest.main);
        let inside = main.components().all(|c| matches!(c, Component::Normal(_)));
        if manifest.main.is_empty() || !inside {
            return Err(format!(
                "{MANIFEST}: main should be a file inside the plugin, not {}",
                manifest.main
            ));
        }
        Ok(manifest)
    }

    /// Reads the manifest of the plugin in `dir`.
    pub fn read(dir: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("no {MANIFEST}: not a ferra plugin")
            } else {
                format!("{MANIFEST}: {e}")
            }
        })?;
        Self::parse(&text)
    }

    /// Fails if it doesn't work with ferra `version`.
    pub fn check_ferra(&self, version: &str) -> Result<(), String> {
        let Some(range) = &self.ferra else {
            return Ok(());
        };
        match version_matches(range, version) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!(
                "{} needs ferra {range}, this is {version}",
                self.name
            )),
            Err(e) => Err(format!("{MANIFEST}: ferra = \"{range}\": {e}")),
        }
    }
}

/// Plugin names: lowercase letters, digits and `-`, starting with a
/// letter or digit (they're folder names).
pub fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid plugin name '{name}' (lowercase letters, digits and -)"
        ))
    }
}

/// A version as numbers; missing parts are 0 (`0.1` is `0.1.0`).
fn parse_version(text: &str) -> Result<[u64; 3], String> {
    // Pre-release and build parts (`1.0.0-beta`) are ignored.
    let core = text.split(['-', '+']).next().unwrap_or_default();
    let mut parts = [0; 3];
    let pieces: Vec<&str> = core.split('.').collect();
    if pieces.is_empty() || pieces.len() > 3 {
        return Err(format!("bad version '{text}'"));
    }
    for (slot, piece) in parts.iter_mut().zip(&pieces) {
        *slot = piece
            .trim()
            .parse()
            .map_err(|_| format!("bad version '{text}'"))?;
    }
    Ok(parts)
}

/// Whether `version` is in `range`: comparisons joined by commas
/// (`">=0.1, <0.3"`). `^0.2` (or a bare `0.2`) means compatible versions,
/// as in Cargo: `>=0.2.0, <0.3.0`.
pub fn version_matches(range: &str, version: &str) -> Result<bool, String> {
    let version = parse_version(version)?;
    let mut all = true;
    for part in range.split(',') {
        let part = part.trim();
        let (op, rest) = ["<=", ">=", "<", ">", "=", "^"]
            .iter()
            .find_map(|op| part.strip_prefix(op).map(|rest| (*op, rest)))
            .unwrap_or(("^", part));
        let want = parse_version(rest.trim())?;
        let ok = match op {
            "<=" => version <= want,
            ">=" => version >= want,
            "<" => version < want,
            ">" => version > want,
            "=" => version == want,
            _ => {
                // The first non-zero part (of those written) must match.
                let written = rest.trim().split('.').count();
                let fixed = want
                    .iter()
                    .position(|&n| n != 0)
                    .unwrap_or(written - 1)
                    .min(written - 1);
                version >= want && version[..=fixed] == want[..=fixed]
            }
        };
        all &= ok;
    }
    Ok(all)
}

/// What to install: a git URL (or path) and maybe a tag or branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// As typed (`owner/repo`, a URL or a path), without the `@tag`.
    pub spec: String,
    pub tag: Option<String>,
}

impl Source {
    /// `owner/repo`, `owner/repo@tag`, or any git URL or local path,
    /// optionally with `@tag`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        // `@tag` at the end, where it can't be part of a URL's user
        // (`git@host:...`) or a path.
        let (spec, tag) = match text.rsplit_once('@') {
            Some((left, tag))
                if left.contains('/') && !tag.contains(['/', ':']) && !tag.is_empty() =>
            {
                (left, Some(tag.to_string()))
            }
            _ => (text, None),
        };
        if spec.is_empty() || spec.chars().any(char::is_whitespace) || spec.starts_with('-') {
            return Err(format!(
                "Not a plugin to install: '{text}' (owner/repo, or a git URL)"
            ));
        }
        if !is_shorthand(spec) && !looks_like_url_or_path(spec) {
            return Err(format!(
                "Not a plugin to install: '{text}' (owner/repo, or a git URL)"
            ));
        }
        Ok(Self {
            spec: spec.to_string(),
            tag,
        })
    }

    /// What `git clone` is given.
    pub fn url(&self) -> String {
        if is_shorthand(&self.spec) {
            format!("https://github.com/{}.git", self.spec)
        } else if let Some(rest) = self.spec.strip_prefix("~/")
            && let Some(home) = std::env::var_os("HOME")
        {
            PathBuf::from(home).join(rest).display().to_string()
        } else {
            self.spec.clone()
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.tag {
            Some(tag) => write!(f, "{}@{tag}", self.spec),
            None => f.write_str(&self.spec),
        }
    }
}

/// `owner/repo` on GitHub.
fn is_shorthand(spec: &str) -> bool {
    let part = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    matches!(spec.split_once('/'), Some((owner, repo)) if part(owner) && part(repo))
}

fn looks_like_url_or_path(spec: &str) -> bool {
    spec.contains("://")
        || spec.starts_with("git@")
        || spec.starts_with('/')
        || spec.starts_with("~/")
        || spec.starts_with("./")
        || spec.starts_with("../")
}

/// One entry of `plugins.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wanted {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locked {
    pub commit: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileOf<T> {
    #[serde(default = "BTreeMap::new")]
    plugins: BTreeMap<String, T>,
}

fn read_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<BTreeMap<String, T>, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str::<FileOf<T>>(&text)
            .map(|f| f.plugins)
            .map_err(|e| format!("{name}: {}", e.message())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("{name}: {e}")),
    }
}

fn write_file<T: Serialize>(
    path: &Path,
    header: &str,
    plugins: BTreeMap<String, T>,
) -> Result<(), String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let body = toml::to_string(&FileOf { plugins }).map_err(|e| format!("{name}: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{name}: {e}"))?;
    }
    std::fs::write(path, format!("{header}\n{body}")).map_err(|e| format!("{name}: {e}"))
}

const LIST_HEADER: &str = "# The plugins ferra loads, kept by :plugin_install and :plugin_remove.";
const LOCK_HEADER: &str = "# The exact commit of each plugin. Written by ferra: don't edit.";

/// `plugins.toml`: the plugins asked for, by name.
pub fn wanted(paths: &PluginPaths) -> Result<BTreeMap<String, Wanted>, String> {
    read_file(&paths.config.join(LIST_FILE))
}

/// `plugins.lock`: each plugin's commit, by name.
pub fn locked(paths: &PluginPaths) -> Result<BTreeMap<String, Locked>, String> {
    read_file(&paths.config.join(LOCK_FILE))
}

/// A plugin as installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub manifest: Manifest,
    pub source: Source,
    pub commit: String,
}

impl Installed {
    pub fn short_commit(&self) -> &str {
        &self.commit[..self.commit.len().min(7)]
    }
}

/// Adds (or replaces) a plugin in `plugins.toml` and `plugins.lock`.
pub fn record(paths: &PluginPaths, installed: &Installed) -> Result<(), String> {
    let name = installed.manifest.name.clone();
    let mut list = wanted(paths)?;
    let mut lock = locked(paths)?;
    list.insert(
        name.clone(),
        Wanted {
            source: installed.source.spec.clone(),
            tag: installed.source.tag.clone(),
        },
    );
    lock.insert(
        name,
        Locked {
            commit: installed.commit.clone(),
        },
    );
    write_file(&paths.config.join(LIST_FILE), LIST_HEADER, list)?;
    write_file(&paths.config.join(LOCK_FILE), LOCK_HEADER, lock)
}

/// Removes plugin `name`: its clone, and its lines in `plugins.toml` and
/// `plugins.lock`.
pub fn remove(paths: &PluginPaths, name: &str) -> Result<(), String> {
    check_name(name)?;
    let mut list = wanted(paths)?;
    let mut lock = locked(paths)?;
    let dir = paths.dir(name);
    let listed = list.remove(name).is_some();
    lock.remove(name);
    if !listed && !dir.exists() {
        return Err(format!("No plugin named {name} is installed"));
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    write_file(&paths.config.join(LIST_FILE), LIST_HEADER, list)?;
    write_file(&paths.config.join(LOCK_FILE), LOCK_HEADER, lock)
}

/// What `:plugin_list` shows about one plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub source: String,
    /// From its manifest, if it's installed and readable.
    pub version: Option<String>,
    pub description: String,
    pub commit: Option<String>,
    /// Why it can't be loaded (not cloned, a broken manifest).
    pub problem: Option<String>,
}

/// Every plugin in `plugins.toml`, by name.
pub fn list(paths: &PluginPaths) -> Result<Vec<Listed>, String> {
    let lock = locked(paths)?;
    Ok(wanted(paths)?
        .into_iter()
        .map(|(name, wanted)| {
            let source = Source {
                spec: wanted.source,
                tag: wanted.tag,
            };
            let manifest = Manifest::read(&paths.dir(&name));
            let commit = lock.get(&name).map(|l| l.commit.clone());
            let (version, description, problem) = match manifest {
                Ok(m) => (Some(m.version), m.description, None),
                Err(_) if !paths.dir(&name).is_dir() => {
                    (None, String::new(), Some("not installed".to_string()))
                }
                Err(e) => (None, String::new(), Some(e)),
            };
            Listed {
                name,
                source: source.to_string(),
                version,
                description,
                commit,
                problem,
            }
        })
        .collect())
}

/// Runs git with no terminal prompts: ferra owns the terminal, so a
/// password or host-key question would hang. It fails instead.
fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let output = command.output().map_err(|e| format!("git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        let error = error
            .lines()
            .map(|l| l.trim().trim_start_matches("fatal: "))
            .rfind(|l| !l.is_empty())
            .unwrap_or("git failed")
            .to_string();
        Err(error)
    }
}

/// Clones `source` into the data folder and checks it's a plugin this
/// ferra can load. Blocking: run it on a thread. `installed` are the names
/// already in use.
pub fn install(
    paths: &PluginPaths,
    source: &Source,
    installed: &[String],
) -> Result<Installed, String> {
    in_temp_clone(paths, |temp| {
        let (manifest, commit) = clone_and_check(source, None, temp)?;
        let name = &manifest.name;
        if installed.contains(name) || paths.dir(name).exists() {
            return Err(format!(
                "A plugin named {name} is already installed (:plugin_remove {name} first)"
            ));
        }
        std::fs::rename(temp, paths.dir(name)).map_err(|e| format!("{name}: {e}"))?;
        Ok(Installed {
            manifest,
            source: source.clone(),
            commit,
        })
    })
}

/// The plugins in `plugins.toml` that aren't cloned (dotfiles brought to a
/// new machine, say).
pub fn missing(paths: &PluginPaths) -> Result<Vec<String>, String> {
    Ok(wanted(paths)?
        .into_keys()
        .filter(|name| !paths.dir(name).exists())
        .collect())
}

/// Clones plugin `name` from its `plugins.toml` source, at the commit in
/// `plugins.lock` if there is one. Blocking.
pub fn restore(paths: &PluginPaths, name: &str) -> Result<Installed, String> {
    let wanted = wanted(paths)?
        .remove(name)
        .ok_or_else(|| format!("{name} isn't in plugins.toml"))?;
    let commit = locked(paths)?.remove(name).map(|l| l.commit);
    let source = Source {
        spec: wanted.source,
        tag: wanted.tag,
    };
    in_temp_clone(paths, |temp| {
        let (manifest, commit) = clone_and_check(&source, commit.as_deref(), temp)?;
        if manifest.name != name {
            return Err(format!(
                "{source} is now called {}, not {name}",
                manifest.name
            ));
        }
        if paths.dir(name).exists() {
            return Err(format!("{name} is already installed"));
        }
        std::fs::rename(temp, paths.dir(name)).map_err(|e| format!("{name}: {e}"))?;
        Ok(Installed {
            manifest,
            source: source.clone(),
            commit,
        })
    })
    .map_err(|e| format!("{name}: {e}"))
}

/// Runs `work` with a fresh folder beside the plugins, under a name that
/// can't be a plugin's, removing it if `work` fails (on success `work`
/// has renamed it into place).
fn in_temp_clone<T>(
    paths: &PluginPaths,
    work: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    std::fs::create_dir_all(&paths.data).map_err(|e| format!("{}: {e}", paths.data.display()))?;
    let temp = paths
        .data
        .join(format!(".installing-{}", std::process::id()));
    if temp.exists() {
        let _ = std::fs::remove_dir_all(&temp);
    }
    let result = work(&temp);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
    }
    result
}

/// Clones `source` into `temp` (at `commit`, if given) and checks the
/// plugin there. Returns its manifest and commit.
fn clone_and_check(
    source: &Source,
    commit: Option<&str>,
    temp: &Path,
) -> Result<(Manifest, String), String> {
    let url = source.url();
    let temp_arg = temp.to_string_lossy();
    let mut args = vec!["clone", "--quiet"];
    if let Some(tag) = &source.tag {
        args.extend(["--branch", tag]);
    }
    args.extend(["--", &url, &temp_arg]);
    git(None, &args).map_err(|e| format!("Couldn't clone {url}: {e}"))?;
    if let Some(commit) = commit {
        // `reset` rather than `checkout`, so the branch (and updating) stays.
        git(Some(temp), &["reset", "--quiet", "--hard", commit])
            .map_err(|e| format!("Couldn't go to commit {commit}: {e}"))?;
    }
    let manifest = check_plugin(temp).map_err(|e| format!("{source}: {e}"))?;
    let commit = git(Some(temp), &["rev-parse", "HEAD"])?;
    Ok((manifest, commit))
}

/// Reads and checks the plugin in `dir`: its manifest, that it works with
/// this ferra, and that its main file is there.
fn check_plugin(dir: &Path) -> Result<Manifest, String> {
    let manifest = Manifest::read(dir)?;
    manifest.check_ferra(env!("CARGO_PKG_VERSION"))?;
    if !dir.join(&manifest.main).is_file() {
        return Err(format!("its main file {} is missing", manifest.main));
    }
    Ok(manifest)
}

/// New commits for one plugin, found by `check_updates`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub name: String,
    /// The commit it's at, and the one it would move to.
    pub from: String,
    pub to: String,
    /// `abc1234 Subject`, newest first.
    pub commits: Vec<String>,
}

/// What `check_updates` found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    pub incoming: Vec<Incoming>,
    /// Plugins already at their newest commit.
    pub current: Vec<String>,
    /// Plugins that can't be updated, and why (`name: pinned to v1`).
    pub skipped: Vec<String>,
}

/// Fetches plugins `names` (all of `plugins.toml` if empty) and lists the
/// commits each would gain. Changes nothing but the clones' remote
/// branches. Blocking.
pub fn check_updates(paths: &PluginPaths, names: &[String]) -> Result<Checked, String> {
    let wanted = wanted(paths)?;
    let names: Vec<String> = if names.is_empty() {
        wanted.keys().cloned().collect()
    } else {
        for name in names {
            if !wanted.contains_key(name) {
                return Err(format!("No plugin named {name} is installed"));
            }
        }
        names.to_vec()
    };
    let mut checked = Checked::default();
    for name in names {
        let dir = paths.dir(&name);
        if !dir.is_dir() {
            checked.skipped.push(format!(
                "{name}: not installed (:plugin_install restores it)"
            ));
            continue;
        }
        let dir = Some(dir.as_path());
        // A tag clones as a detached HEAD: there's nothing to follow.
        if git(dir, &["symbolic-ref", "--quiet", "HEAD"]).is_err() {
            let at = wanted[&name].tag.as_deref().unwrap_or("a commit");
            checked.skipped.push(format!("{name}: pinned to {at}"));
            continue;
        }
        if let Err(e) = git(dir, &["fetch", "--quiet"]) {
            checked
                .skipped
                .push(format!("{name}: couldn't fetch ({e})"));
            continue;
        }
        let Ok(to) = git(dir, &["rev-parse", "@{upstream}"]) else {
            checked
                .skipped
                .push(format!("{name}: its branch follows nothing"));
            continue;
        };
        let from = git(dir, &["rev-parse", "HEAD"])?;
        if from == to {
            checked.current.push(name);
            continue;
        }
        if git(dir, &["merge-base", "--is-ancestor", &from, &to]).is_err() {
            checked.skipped.push(format!(
                "{name}: has commits of its own, so it can't fast-forward"
            ));
            continue;
        }
        let log = git(dir, &["log", "--format=%h %s", &format!("{from}..{to}")])?;
        checked.incoming.push(Incoming {
            name,
            from,
            to,
            commits: log.lines().map(str::to_string).collect(),
        });
    }
    Ok(checked)
}

/// Moves a plugin to the commit `check_updates` found, going back if the
/// new version isn't one this ferra can load. Returns the new manifest;
/// the caller records the commit. Blocking.
pub fn apply_update(paths: &PluginPaths, incoming: &Incoming) -> Result<Manifest, String> {
    let name = &incoming.name;
    let dir = paths.dir(name);
    let dir = Some(dir.as_path());
    git(dir, &["merge", "--quiet", "--ff-only", &incoming.to])
        .map_err(|e| format!("{name}: {e}"))?;
    match check_plugin(&paths.dir(name)) {
        Ok(manifest) if manifest.name == *name => Ok(manifest),
        result => {
            let _ = git(dir, &["reset", "--quiet", "--hard", &incoming.from]);
            let why = match result {
                Ok(manifest) => format!("it's now called {}", manifest.name),
                Err(e) => e,
            };
            Err(format!("{name}: kept the old version: {why}"))
        }
    }
}

/// Sets plugin `name`'s commit in `plugins.lock`.
pub fn lock(paths: &PluginPaths, name: &str, commit: &str) -> Result<(), String> {
    let mut lock = locked(paths)?;
    lock.insert(
        name.to_string(),
        Locked {
            commit: commit.to_string(),
        },
    );
    write_file(&paths.config.join(LOCK_FILE), LOCK_HEADER, lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources() {
        let s = Source::parse("owner/repo").unwrap();
        assert_eq!((s.spec.as_str(), s.tag.as_deref()), ("owner/repo", None));
        assert_eq!(s.url(), "https://github.com/owner/repo.git");

        let s = Source::parse("owner/repo@v1.2").unwrap();
        assert_eq!(s.tag.as_deref(), Some("v1.2"));
        assert_eq!(s.to_string(), "owner/repo@v1.2");

        let s = Source::parse("git@github.com:o/r.git").unwrap();
        assert_eq!(
            (s.spec.as_str(), s.tag.as_deref()),
            ("git@github.com:o/r.git", None)
        );
        let s = Source::parse("git@github.com:o/r.git@main").unwrap();
        assert_eq!(s.tag.as_deref(), Some("main"));
        let s = Source::parse("https://example.com/a/b").unwrap();
        assert_eq!(s.url(), "https://example.com/a/b");
        let s = Source::parse("/tmp/x@y/repo").unwrap();
        assert_eq!((s.spec.as_str(), s.tag.as_deref()), ("/tmp/x@y/repo", None));

        for bad in ["", "repo", "a b/c", "-x/y", "owner/", "--upload-pack=x/y"] {
            assert!(Source::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn version_ranges() {
        let m = |range, version| version_matches(range, version).unwrap();
        assert!(m(">=0.1", "0.1.0"));
        assert!(m(">=0.1, <0.3", "0.2.5"));
        assert!(!m(">=0.1, <0.3", "0.3.0"));
        assert!(m("^0.1", "0.1.9"));
        assert!(!m("^0.1", "0.2.0"));
        assert!(m("0.1", "0.1.3"));
        assert!(m("^1.2", "1.9.0"));
        assert!(!m("^1.2", "2.0.0"));
        assert!(!m("^1.2", "1.1.0"));
        assert!(m("=0.1.0", "0.1.0"));
        assert!(m(">0.1", "0.1.1"));
        assert!(version_matches(">=x", "0.1.0").is_err());
    }

    #[test]
    fn manifests() {
        let m = Manifest::parse("name = \"todos\"\nversion = \"0.1.0\"").unwrap();
        assert_eq!(m.main, "init.lua");
        assert!(m.check_ferra("0.1.0").is_ok());

        let m = Manifest::parse("name = \"t\"\nversion = \"1\"\nferra = \">=9\"").unwrap();
        assert_eq!(
            m.check_ferra("0.1.0").unwrap_err(),
            "t needs ferra >=9, this is 0.1.0"
        );

        for bad in [
            "name = \"Todos\"\nversion = \"1\"",
            "name = \"../x\"\nversion = \"1\"",
            "name = \"t\"\nversion = \"1\"\nmain = \"../x.lua\"",
            "name = \"t\"\nversion = \"1\"\nmain = \"/etc/x.lua\"",
            "name = \"t\"\nversion = \"1\"\nextra = 1",
            "name = \"t\"",
        ] {
            assert!(Manifest::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn records_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PluginPaths {
            config: dir.path().join("config"),
            data: dir.path().join("data"),
        };
        let installed = Installed {
            manifest: Manifest::parse("name = \"todos\"\nversion = \"0.1.0\"").unwrap(),
            source: Source::parse("o/todos@v1").unwrap(),
            commit: "0123456789abcdef".into(),
        };
        std::fs::create_dir_all(paths.dir("todos")).unwrap();
        record(&paths, &installed).unwrap();
        let text = std::fs::read_to_string(paths.config.join("plugins.toml")).unwrap();
        assert!(
            text.contains("[plugins.todos]\nsource = \"o/todos\"\ntag = \"v1\""),
            "{text}"
        );
        assert_eq!(locked(&paths).unwrap()["todos"].commit, "0123456789abcdef");

        let listed = list(&paths).unwrap();
        assert_eq!(listed[0].source, "o/todos@v1");
        assert_eq!(
            listed[0].problem.as_deref(),
            Some("no ferra-plugin.toml: not a ferra plugin")
        );

        remove(&paths, "todos").unwrap();
        assert!(wanted(&paths).unwrap().is_empty());
        assert!(!paths.dir("todos").exists());
        assert!(remove(&paths, "todos").is_err());
    }
}
