//! Listing the files in a project, for the file picker.

use std::path::{Path, PathBuf};

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

/// Lists the files under `root` (recursively), relative to it.
///
/// The `.git` directory is always skipped. Unreadable entries are skipped
/// rather than failing the whole listing.
pub fn list_files(root: &Path, options: &WalkOptions) -> FileList {
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
        .build();

    let mut files = Vec::new();
    let mut truncated = false;
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if files.len() == options.max_files {
            truncated = true;
            break;
        }
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        files.push(relative.to_path_buf());
    }
    files.sort();
    FileList { files, truncated }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

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
}
