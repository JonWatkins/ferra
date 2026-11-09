//! Listing the files in a project, for the file picker.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::WalkBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkOptions {
    /// Include hidden files and directories (dotfiles).
    pub hidden: bool,
    /// Skip files matched by `.gitignore`, `.ignore` and git's excludes.
    pub git_ignore: bool,
    /// Stop after this many files, so a huge directory can't stall the UI.
    pub max_files: usize,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            hidden: false,
            git_ignore: true,
            max_files: 20_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileList {
    /// Paths relative to the root, sorted.
    pub files: Vec<PathBuf>,
    /// `max_files` was reached, so some files are missing.
    pub truncated: bool,
}

/// Lists the files under `root` (recursively), relative to it, sorted.
///
/// The `.git` directory is always skipped. Unreadable entries are skipped
/// rather than failing the whole listing.
pub fn list_files(root: &Path, options: &WalkOptions) -> FileList {
    let mut files = Vec::new();
    let truncated = walk_files(root, options, &AtomicBool::new(false), |batch| {
        files.extend(batch);
    });
    files.sort();
    FileList { files, truncated }
}

/// How many paths `walk_files` gathers before handing them over.
const BATCH: usize = 256;

/// Walks the files under `root` like [`list_files`], handing them to
/// `on_batch` a few hundred at a time (relative to `root`, in directory
/// order) so a caller on another thread can show them as they come. Stops
/// early once `cancel` is set. Returns whether `max_files` cut it short.
pub fn walk_files(
    root: &Path,
    options: &WalkOptions,
    cancel: &AtomicBool,
    mut on_batch: impl FnMut(Vec<PathBuf>),
) -> bool {
    let walker = WalkBuilder::new(root)
        .hidden(!options.hidden)
        .git_ignore(options.git_ignore)
        .git_global(options.git_ignore)
        .git_exclude(options.git_ignore)
        .ignore(options.git_ignore)
        .parents(options.git_ignore)
        // Honour .gitignore even outside a git repository.
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    let mut batch = Vec::with_capacity(BATCH);
    let mut found = 0;
    let mut truncated = false;
    for entry in walker.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if found == options.max_files {
            truncated = true;
            break;
        }
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        batch.push(relative.to_path_buf());
        found += 1;
        if batch.len() == BATCH {
            on_batch(std::mem::replace(&mut batch, Vec::with_capacity(BATCH)));
        }
    }
    if !batch.is_empty() {
        on_batch(batch);
    }
    truncated
}

/// One entry of a directory listing (for the file tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// What's in `dir` (not its subdirectories), folders first, then by name
/// (ignoring case), leaving out what `.gitignore` and the like ignore and
/// `.git` itself, as the file picker does; dotfiles are kept (`.github`,
/// `.env`), as in VS Code.
pub fn list_dir(dir: &Path) -> Vec<DirEntry> {
    let walker = WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let mut entries: Vec<DirEntry> = walker
        .flatten()
        .filter(|entry| entry.depth() == 1)
        .map(|entry| DirEntry {
            path: entry.path().to_path_buf(),
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: entry.file_type().is_some_and(|t| t.is_dir()),
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn listing_a_folder_puts_folders_first_and_leaves_out_what_git_ignores() {
        let dir = project();
        let names: Vec<(String, bool)> = list_dir(dir.path())
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(
            names,
            [
                ("src".to_string(), true),
                (".gitignore".to_string(), false),
                (".hidden".to_string(), false),
                ("README.md".to_string(), false),
            ],
            "no .git or ignored target/; dotfiles kept"
        );
    }

    /// A temp project: `src/main.rs`, `README.md`, `.hidden`, a git-ignored
    /// `target/out.bin` and a `.git/HEAD`.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, contents) in [
            ("src/main.rs", "fn main() {}"),
            ("README.md", "# hi"),
            (".hidden", "secret"),
            ("target/out.bin", "binary"),
            (".git/HEAD", "ref"),
            (".gitignore", "target/\n"),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    fn names(list: &FileList) -> Vec<String> {
        list.files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[test]
    fn lists_project_files_relative_and_sorted() {
        let dir = project();
        let list = list_files(dir.path(), &WalkOptions::default());
        assert_eq!(names(&list), ["README.md", "src/main.rs"]);
        assert!(!list.truncated);
    }

    #[test]
    fn hidden_files_can_be_included_but_never_git() {
        let dir = project();
        let options = WalkOptions {
            hidden: true,
            ..WalkOptions::default()
        };
        let list = list_files(dir.path(), &options);
        assert_eq!(
            names(&list),
            [".gitignore", ".hidden", "README.md", "src/main.rs"]
        );
    }

    #[test]
    fn git_ignore_can_be_turned_off() {
        let dir = project();
        let options = WalkOptions {
            git_ignore: false,
            ..WalkOptions::default()
        };
        let list = list_files(dir.path(), &options);
        assert!(names(&list).contains(&"target/out.bin".to_string()));
    }

    #[test]
    fn stops_at_the_limit() {
        let dir = project();
        let options = WalkOptions {
            max_files: 1,
            ..WalkOptions::default()
        };
        let list = list_files(dir.path(), &options);
        assert_eq!(list.files.len(), 1);
        assert!(list.truncated);
    }

    #[test]
    fn missing_root_gives_an_empty_list() {
        let list = list_files(Path::new("/definitely/not/here"), &WalkOptions::default());
        assert!(list.files.is_empty());
    }

    #[test]
    fn walking_hands_over_batches_and_stops_when_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..600 {
            fs::write(dir.path().join(format!("f{i:03}")), "").unwrap();
        }
        let mut sizes = Vec::new();
        let truncated = walk_files(
            dir.path(),
            &WalkOptions::default(),
            &AtomicBool::new(false),
            |b| {
                sizes.push(b.len());
            },
        );
        assert_eq!(sizes, [256, 256, 88]);
        assert!(!truncated);

        let cancel = AtomicBool::new(false);
        let mut seen = 0;
        walk_files(dir.path(), &WalkOptions::default(), &cancel, |b| {
            seen += b.len();
            cancel.store(true, Ordering::Relaxed);
        });
        assert_eq!(seen, 256, "nothing after the cancel");
    }
}
