//! Finding a file named in the text, as Vim's `gf` does: through the
//! directories of the `path` option, trying each ending in `suffixes-add`
//! too.
//!
//! `path` is a comma-separated list of directories, written as in Vim:
//!
//! - `.` is the current file's directory, and `./dir` a directory under it;
//! - an empty entry (the `,,` in the default `.,,`) is the working
//!   directory, and other relative entries are under it;
//! - `**` is any depth of directories below (up to 30, or `**N` for N), and
//!   `*` in a name matches any characters: `src/**`, `crates/*/src`;
//! - `;` after an entry searches each directory above it too, up to and
//!   including the one after the `;` (or the root): `.;` looks upwards from
//!   the current file;
//! - `\,` is a comma in a name, and `~/` the home directory.
//!
//! A name that's absolute or starts with `./` or `../` isn't looked for in
//! `path`: it's beside the current file, else from the working directory.
//! In each directory the name is tried as written, then with each of
//! `suffixes-add`'s endings (with `.rs` there, `gf` on `mod_name` finds
//! `mod_name.rs`). Under `**`, a directory is searched before the ones in
//! it, in name order; symlinked directories aren't followed there (a link
//! back up would go round until the depth runs out).

use std::path::{Component, Path, PathBuf};

/// How deep `**` goes without a number after it (Vim's limit).
const DEFAULT_DEPTH: usize = 30;

/// Where [`find_file`] looks.
#[derive(Debug, Clone, Copy)]
pub struct FileSearch<'a> {
    /// The directories, as Vim's `path` option writes them.
    pub path: &'a str,
    /// Endings to try after the name as written, comma-separated.
    pub suffixes: &'a str,
    /// The current file's directory, if the buffer has a file.
    pub file_dir: Option<&'a Path>,
    /// What relative names are relative to (`""` for the process's
    /// working directory, keeping the paths found relative).
    pub cwd: &'a Path,
}

/// The first file `name` finds (see the module docs).
pub fn find_file(name: &str, search: &FileSearch) -> Option<PathBuf> {
    let name = expand_home(name);
    let dotted = matches!(
        name.components().next(),
        Some(Component::CurDir | Component::ParentDir)
    );
    // `./b.rs` beside `src/a.rs` is `src/b.rs`, not `src/./b.rs`.
    let name = name
        .strip_prefix(".")
        .map_or(name.clone(), Path::to_path_buf);
    let suffixes = split_list(search.suffixes);
    let try_in = |dir: &Path| -> Option<PathBuf> {
        std::iter::once(name.clone())
            .chain(
                suffixes
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|suffix| with_suffix(&name, suffix)),
            )
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    };
    if name.is_absolute() {
        return try_in(Path::new(""));
    }
    if dotted {
        return search
            .file_dir
            .and_then(&try_in)
            .or_else(|| try_in(search.cwd));
    }
    split_list(search.path)
        .iter()
        .find_map(|entry| search_entry(entry, search, &try_in))
}

/// Searches one entry of `path`.
fn search_entry(
    entry: &str,
    search: &FileSearch,
    try_in: &dyn Fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let (entry, upward) = match entry.split_once(';') {
        Some((entry, stop)) => (entry, Some(stop)),
        None => (entry, None),
    };
    let (base, pattern) = base_and_pattern(entry, search);
    let found = walk(&base, &pattern, try_in);
    let Some(stop) = upward else {
        return found;
    };
    if found.is_some() {
        return found;
    }
    let stop = if stop.is_empty() {
        None
    } else {
        let (stop, rest) = base_and_pattern(stop, search);
        canonical(&stop.join(rest.join("/")))
    };
    let mut dir = canonical(&base)?;
    loop {
        if Some(&dir) == stop.as_ref() {
            return None;
        }
        dir = dir.parent()?.to_path_buf();
        if let Some(found) = walk(&dir, &pattern, try_in) {
            return Some(found);
        }
    }
}

/// The directory an entry starts from, and its parts from the first
/// wildcard on.
fn base_and_pattern<'e>(entry: &'e str, search: &FileSearch) -> (PathBuf, Vec<&'e str>) {
    let file_dir = || search.file_dir.unwrap_or(search.cwd).to_path_buf();
    let (mut base, rest) = if entry == "." {
        (file_dir(), "")
    } else if let Some(rest) = entry.strip_prefix("./") {
        (file_dir(), rest)
    } else if let Some((rest, home)) = entry.strip_prefix("~/").zip(home()) {
        (home, rest)
    } else if let Some(rest) = entry.strip_prefix('/') {
        (PathBuf::from("/"), rest)
    } else {
        (search.cwd.to_path_buf(), entry)
    };
    let mut parts = rest.split('/').filter(|part| !part.is_empty()).peekable();
    while let Some(part) = parts.next_if(|part| !part.contains('*')) {
        base.push(part);
    }
    (base, parts.collect())
}

/// Tries `dir` and the directories under it that `pattern` matches.
fn walk(
    dir: &Path,
    pattern: &[&str],
    try_in: &dyn Fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let Some((first, rest)) = pattern.split_first() else {
        return try_in(dir);
    };
    if let Some(depth) = depth_of(first) {
        return deep(dir, depth, rest, try_in);
    }
    if first.contains('*') {
        return subdirs(dir, true)
            .into_iter()
            .filter(|sub| {
                sub.file_name()
                    .is_some_and(|n| glob(first, &n.to_string_lossy()))
            })
            .find_map(|sub| walk(&sub, rest, try_in));
    }
    walk(&dir.join(first), rest, try_in)
}

/// `**`: `dir`, then the directories in it to `depth` levels down.
fn deep(
    dir: &Path,
    depth: usize,
    rest: &[&str],
    try_in: &dyn Fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    walk(dir, rest, try_in).or_else(|| {
        if depth == 0 {
            return None;
        }
        subdirs(dir, false)
            .into_iter()
            .find_map(|sub| deep(&sub, depth - 1, rest, try_in))
    })
}

/// `**` or `**N`: how deep it goes.
fn depth_of(part: &str) -> Option<usize> {
    let digits = part.strip_prefix("**")?;
    if digits.is_empty() {
        return Some(DEFAULT_DEPTH);
    }
    digits.parse().ok()
}

/// The directories in `dir`, in name order (symlinked ones only with
/// `follow_links`). Kept relative when `dir` is.
fn subdirs(dir: &Path, follow_links: bool) -> Vec<PathBuf> {
    let listed = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = std::fs::read_dir(listed) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|t| {
                t.is_dir() || (follow_links && t.is_symlink() && entry.path().is_dir())
            })
        })
        .map(|entry| dir.join(entry.file_name()))
        .collect();
    dirs.sort();
    dirs
}

/// Whether `name` matches `pattern`, where `*` is any run of characters.
fn glob(pattern: &str, name: &str) -> bool {
    let mut pieces = pattern.split('*');
    let first = pieces.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let pieces: Vec<&str> = pieces.collect();
    let Some((last, middle)) = pieces.split_last() else {
        return rest.is_empty();
    };
    for piece in middle {
        match rest.find(piece) {
            Some(at) => rest = &rest[at + piece.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// A comma-separated option's items, with `\,` a comma in one. Empty
/// text has none; `a,,b` has an empty one.
pub fn split_list(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut items = vec![String::new()];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                chars.next();
                items.last_mut().unwrap().push(',');
            }
            ',' => items.push(String::new()),
            c => items.last_mut().unwrap().push(c),
        }
    }
    items
}

fn with_suffix(name: &Path, suffix: &str) -> PathBuf {
    let mut name = name.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn expand_home(name: &str) -> PathBuf {
    match name.strip_prefix("~/").zip(home()) {
        Some((rest, home)) => home.join(rest),
        None => PathBuf::from(name),
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn canonical(dir: &Path) -> Option<PathBuf> {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    std::fs::canonicalize(dir).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A tree of files (directories made as needed) under a temp dir.
    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            let path = dir.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        dir
    }

    /// What `name` finds from the file `from` (relative to `root`, which
    /// is also the working directory), relative to `root`.
    fn find(root: &Path, from: &str, path: &str, suffixes: &str, name: &str) -> Option<String> {
        let file_dir = root.join(from);
        let file_dir = file_dir.parent();
        find_file(
            name,
            &FileSearch {
                path,
                suffixes,
                file_dir,
                cwd: root,
            },
        )
        .map(|found| {
            found
                .strip_prefix(root)
                .unwrap_or(&found)
                .to_string_lossy()
                .into_owned()
        })
    }

    #[test]
    fn the_default_path_is_beside_the_file_then_the_working_directory() {
        let dir = tree(&["src/a.rs", "src/b.txt", "b.txt", "c.txt"]);
        let root = dir.path();
        assert_eq!(
            find(root, "src/a.rs", ".,,", "", "b.txt").as_deref(),
            Some("src/b.txt")
        );
        assert_eq!(
            find(root, "src/a.rs", ".,,", "", "c.txt").as_deref(),
            Some("c.txt")
        );
        assert_eq!(find(root, "src/a.rs", ".,,", "", "d.txt"), None);
        assert_eq!(
            find(root, "src/a.rs", "", "", "c.txt"),
            None,
            "an empty path has nowhere"
        );
    }

    #[test]
    fn suffixes_are_tried_in_each_directory_after_the_name() {
        let dir = tree(&["src/util.rs", "src/both", "src/both.rs", "lib.py"]);
        let root = dir.path();
        assert_eq!(
            find(root, "src/a.rs", ".,,", ".rs", "util").as_deref(),
            Some("src/util.rs")
        );
        assert_eq!(
            find(root, "src/a.rs", ".,,", ".rs", "both").as_deref(),
            Some("src/both")
        );
        assert_eq!(
            find(root, "src/a.rs", ".,,", ".rs,.py", "lib").as_deref(),
            Some("lib.py"),
            "every suffix beside the file, then the next directory"
        );
    }

    #[test]
    fn stars_search_below() {
        let dir = tree(&["a/deep/er/x.h", "a/y.h", "crates/one/src/z.rs", "b/x.h"]);
        let root = dir.path();
        assert_eq!(
            find(root, "m.c", "**", "", "x.h").as_deref(),
            Some("a/deep/er/x.h")
        );
        assert_eq!(
            find(root, "m.c", "b/**,**", "", "x.h").as_deref(),
            Some("b/x.h")
        );
        assert_eq!(
            find(root, "m.c", "**1", "", "x.h").as_deref(),
            Some("b/x.h")
        );
        assert_eq!(
            find(root, "m.c", "a/**1", "", "x.h"),
            None,
            "two levels down"
        );
        assert_eq!(find(root, "m.c", "**", "", "y.h").as_deref(), Some("a/y.h"));
        assert_eq!(
            find(root, "m.c", "crates/*/src", "", "z.rs").as_deref(),
            Some("crates/one/src/z.rs")
        );
        assert_eq!(
            find(root, "m.c", "cr*s/o*/src", "", "z.rs").as_deref(),
            Some("crates/one/src/z.rs")
        );
        assert_eq!(find(root, "m.c", "crates/x*/src", "", "z.rs"), None);
    }

    #[test]
    fn a_semicolon_searches_upwards() {
        let dir = tree(&["top.txt", "a/mid.txt", "a/b/c/file.rs"]);
        let root = dir.path();
        assert_eq!(find(root, "a/b/c/file.rs", ".", "", "mid.txt"), None);
        assert_eq!(
            find(root, "a/b/c/file.rs", ".;", "", "mid.txt").map(|p| p.ends_with("a/mid.txt")),
            Some(true)
        );
        let stop = root.join("a/b");
        let path = format!(".;{}", stop.display());
        assert_eq!(
            find(root, "a/b/c/file.rs", &path, "", "mid.txt"),
            None,
            "stops at a/b"
        );
        let stop = root.join("a");
        let path = format!(".;{}", stop.display());
        assert!(
            find(root, "a/b/c/file.rs", &path, "", "mid.txt").is_some(),
            "a itself is searched"
        );
        assert_eq!(find(root, "a/b/c/file.rs", &path, "", "top.txt"), None);
    }

    #[test]
    fn dotted_and_absolute_names_skip_the_path() {
        let dir = tree(&["src/a.rs", "src/b.rs", "lib/b.rs", "c.rs"]);
        let root = dir.path();
        assert_eq!(
            find(root, "src/a.rs", "lib", "", "./b.rs").as_deref(),
            Some("src/b.rs")
        );
        assert_eq!(
            find(root, "src/a.rs", "lib", "", "../c.rs").as_deref(),
            Some("src/../c.rs")
        );
        assert_eq!(
            find(root, "src/a.rs", "lib", "", "b.rs").as_deref(),
            Some("lib/b.rs")
        );
        let absolute = root.join("lib/b").display().to_string();
        assert_eq!(
            find(root, "src/a.rs", "", ".rs", &absolute).as_deref(),
            Some("lib/b.rs")
        );
    }

    #[test]
    fn splits_lists_with_escaped_commas() {
        assert_eq!(split_list(".,,"), [".", "", ""]);
        assert_eq!(split_list(""), Vec::<String>::new());
        assert_eq!(split_list(r"a\,b,c"), ["a,b", "c"]);
    }

    #[test]
    fn globs() {
        assert!(glob("*", "anything"));
        assert!(glob("a*", "abc"));
        assert!(glob("*c", "abc"));
        assert!(glob("a*c", "ac"));
        assert!(glob("a*b*c", "aXbYc"));
        assert!(!glob("a*c", "ab"));
        assert!(!glob("ab*b", "ab"), "pieces don't overlap");
        assert!(!glob("abc", "abcd"));
    }
}
