//! Git, read by running `git` (so it behaves exactly as git does, hooks,
//! config and all, with nothing to link) and diffed in-process (so the
//! gutter keeps up with typing).
//!
//! A file's *base* is its staged version (the index, `git show :./file`),
//! as VS Code's gutter uses: lines you've staged stop showing as changes.
//! `line_changes` diffs the buffer's lines against it into hunks of added,
//! changed and removed lines.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};

/// A diff that takes longer than this gives up with what it has.
const DIFF_DEADLINE: Duration = Duration::from_millis(50);

/// Runs `git` in `dir`. Its stdout, if it succeeded.
fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        // A repository's config can't make reading it ask for anything.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Runs `git` in `dir` for an action: its stdout, or what it said went
/// wrong (its error output, else its exit status).
fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    let error = error
        .lines()
        .map(|l| {
            l.trim_start_matches("error: ")
                .trim_start_matches("fatal: ")
                .trim()
        })
        .find(|l| !l.is_empty())
        .map(str::to_string);
    Err(error.unwrap_or_else(|| format!("git {} failed ({})", args[0], output.status)))
}

/// The folder a file's in (for running git there).
fn dir_of(path: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    path.parent().map(Path::to_path_buf)
}

/// The staged text of the file at `path`, if it's in a repository and
/// git knows it (not untracked or ignored). Line breaks as in the file.
pub fn staged_text(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let bytes = git(&dir_of(path)?, &["show", &format!(":./{name}")])?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The current branch of the repository `path` is in (or the short commit,
/// when HEAD isn't on a branch).
pub fn branch(path: &Path) -> Option<String> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        dir_of(path)?
    };
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).trim().to_string();
    git(&dir, &["symbolic-ref", "--short", "-q", "HEAD"])
        .map(text)
        .filter(|b| !b.is_empty())
        .or_else(|| git(&dir, &["rev-parse", "--short", "HEAD"]).map(text))
        .filter(|b| !b.is_empty())
}

/// The top folder of the repository `dir` is in.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let bytes = git(dir, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
    Some(std::fs::canonicalize(&root).unwrap_or(root))
}

/// A changed file, as `git status` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Relative to the repository's top folder, with `/`.
    pub path: String,
    /// What's staged: `M`odified, `A`dded, `D`eleted, `R`enamed..., if
    /// anything.
    pub staged: Option<char>,
    /// What's changed but not staged: `M`, `D`, or `U` for untracked (and
    /// conflicts), if anything.
    pub unstaged: Option<char>,
}

/// A repository's changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStatus {
    pub root: PathBuf,
    pub files: Vec<FileStatus>,
}

impl RepoStatus {
    pub fn staged(&self) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(|f| f.staged.is_some())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(|f| f.unstaged.is_some())
    }
}

/// The changes in the repository `dir` is in (untracked files listed one by
/// one), or `None` outside a repository.
pub fn status(dir: &Path) -> Option<RepoStatus> {
    let root = repo_root(dir)?;
    let bytes = git(&root, &["status", "--porcelain=v1", "-z", "-uall"])?;
    let text = String::from_utf8_lossy(&bytes);
    let mut entries = text.split('\0');
    let mut files = Vec::new();
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let mut code = code.chars();
        let (x, y) = (code.next().unwrap_or(' '), code.next().unwrap_or(' '));
        // A rename or copy is followed by where it came from.
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            entries.next();
        }
        let conflict = x == 'U' || y == 'U' || (x == y && matches!(x, 'A' | 'D'));
        let (staged, unstaged) = match (x, y) {
            _ if conflict => (None, Some('!')),
            ('?', '?') => (None, Some('U')),
            (x, y) => ((x != ' ').then_some(x), (y != ' ').then_some(y)),
        };
        files.push(FileStatus {
            path: path.to_string(),
            staged,
            unstaged,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Some(RepoStatus { root, files })
}

/// `git add` (new, changed and deleted files alike).
pub fn stage(root: &Path, paths: &[&str]) -> Result<(), String> {
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths);
    run(root, &args).map(drop)
}

/// Takes files out of what's staged, keeping their changes.
pub fn unstage(root: &Path, paths: &[&str]) -> Result<(), String> {
    let has_head = git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some();
    let mut args = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        // Before the first commit there's nothing to restore from.
        vec!["rm", "--cached", "-r", "-q", "--"]
    };
    args.extend(paths);
    run(root, &args).map(drop)
}

/// Throws away a file's unstaged changes (an untracked file is deleted).
pub fn discard(root: &Path, file: &FileStatus) -> Result<(), String> {
    match file.unstaged {
        Some('U') => run(root, &["clean", "-f", "-q", "--", &file.path]).map(drop),
        Some(_) => run(root, &["restore", "--", &file.path]).map(drop),
        None => Ok(()),
    }
}

/// Commits what's staged with `message`. Returns the new commit's short id.
pub fn commit(root: &Path, message: &str) -> Result<String, String> {
    run(root, &["commit", "-q", "-m", message])?;
    Ok(run(root, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// What happened to some lines, compared with the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    /// Lines that were here are gone: `start` is the row after them (so the
    /// mark goes between `start - 1` and `start`), and the hunk has no rows.
    Removed,
}

/// A run of changed lines, in the buffer's rows (`start..end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub kind: ChangeKind,
    pub start: usize,
    pub end: usize,
    /// How many of the base's lines it replaces.
    pub removed: usize,
}

impl Hunk {
    /// The row to put the cursor on for this hunk.
    pub fn first_row(&self) -> usize {
        match self.kind {
            ChangeKind::Removed => self.start.saturating_sub(1),
            _ => self.start,
        }
    }
}

/// Counts of changed lines (the status line's `+3 ~1 -2`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeCounts {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

pub fn counts(hunks: &[Hunk]) -> ChangeCounts {
    let mut counts = ChangeCounts::default();
    for hunk in hunks {
        match hunk.kind {
            ChangeKind::Added => counts.added += hunk.end - hunk.start,
            ChangeKind::Changed => {
                let rows = hunk.end - hunk.start;
                counts.changed += rows.min(hunk.removed);
                counts.added += rows.saturating_sub(hunk.removed);
                counts.removed += hunk.removed.saturating_sub(rows);
            }
            ChangeKind::Removed => counts.removed += hunk.removed,
        }
    }
    counts
}

/// The lines of `text` (without their line breaks; a final line break
/// doesn't make an empty last line).
fn lines(text: &str) -> Vec<&str> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// How `current` differs from `base`, line by line.
pub fn line_changes(base: &str, current: &str) -> Vec<Hunk> {
    let old = lines(base);
    let new = lines(current);
    let deadline = Instant::now() + DIFF_DEADLINE;
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &old, &new, Some(deadline));
    let mut hunks: Vec<Hunk> = Vec::new();
    for op in ops {
        let hunk = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Insert {
                new_index, new_len, ..
            } => Hunk {
                kind: ChangeKind::Added,
                start: new_index,
                end: new_index + new_len,
                removed: 0,
            },
            DiffOp::Delete {
                new_index, old_len, ..
            } => Hunk {
                kind: ChangeKind::Removed,
                start: new_index,
                end: new_index,
                removed: old_len,
            },
            DiffOp::Replace {
                old_len,
                new_index,
                new_len,
                ..
            } => Hunk {
                kind: ChangeKind::Changed,
                start: new_index,
                end: new_index + new_len,
                removed: old_len,
            },
        };
        // A delete next to an insert is one change.
        if let Some(last) = hunks.last_mut()
            && last.end == hunk.start
            && (last.kind == ChangeKind::Removed || hunk.kind == ChangeKind::Removed)
            && last.kind != hunk.kind
        {
            last.kind = ChangeKind::Changed;
            last.end = hunk.end;
            last.removed += hunk.removed;
            continue;
        }
        hunks.push(hunk);
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(hunks: &[Hunk]) -> Vec<(ChangeKind, usize, usize)> {
        hunks.iter().map(|h| (h.kind, h.start, h.end)).collect()
    }

    #[test]
    fn lines_added_changed_and_removed() {
        let base = "a\nb\nc\nd\ne\n";
        assert_eq!(line_changes(base, base), []);
        assert_eq!(
            kinds(&line_changes(base, "a\nnew\nb\nc\nd\ne\n")),
            [(ChangeKind::Added, 1, 2)]
        );
        assert_eq!(
            kinds(&line_changes(base, "a\nB\nc\nd\ne\n")),
            [(ChangeKind::Changed, 1, 2)]
        );
        let removed = line_changes(base, "a\nd\ne\n");
        assert_eq!(kinds(&removed), [(ChangeKind::Removed, 1, 1)]);
        assert_eq!(removed[0].removed, 2);
        assert_eq!(removed[0].first_row(), 0, "the line above the gap");
        assert_eq!(
            kinds(&line_changes(base, "a\nb\nc\nd\ne\nf")),
            [(ChangeKind::Added, 5, 6)],
            "a missing last line break isn't a change"
        );
    }

    #[test]
    fn counts_lines() {
        let hunks = line_changes("a\nb\nc\n", "a\nB\nnew\n");
        assert_eq!(
            counts(&hunks),
            ChangeCounts {
                added: 0,
                changed: 2,
                removed: 0
            }
        );
        let hunks = line_changes("a\nb\nc\nd\n", "x\nd\nnew\n");
        assert_eq!(
            counts(&hunks),
            ChangeCounts {
                added: 1,
                changed: 1,
                removed: 2
            }
        );
    }

    use super::tests_support::repo;

    #[test]
    fn status_stage_unstage_discard_and_commit() {
        let Some(dir) = repo() else {
            return;
        };
        let root = dir.path();
        std::fs::write(root.join("file.txt"), "one\nchanged\n").unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/new.txt"), "new\n").unwrap();
        let status = status(&root.join("sub")).unwrap();
        let canonical = std::fs::canonicalize(root).unwrap();
        assert_eq!(status.root, canonical);
        let summary = |s: &RepoStatus| -> Vec<(String, Option<char>, Option<char>)> {
            s.files
                .iter()
                .map(|f| (f.path.clone(), f.staged, f.unstaged))
                .collect()
        };
        assert_eq!(
            summary(&status),
            [
                ("file.txt".into(), None, Some('M')),
                ("sub/new.txt".into(), None, Some('U')),
            ]
        );

        stage(&canonical, &["file.txt", "sub/new.txt"]).unwrap();
        let status = super::status(root).unwrap();
        assert_eq!(
            summary(&status),
            [
                ("file.txt".into(), Some('M'), None),
                ("sub/new.txt".into(), Some('A'), None),
            ]
        );
        unstage(&canonical, &["sub/new.txt"]).unwrap();
        let status = super::status(root).unwrap();
        assert_eq!(status.unstaged().count(), 1);

        let untracked = status
            .files
            .iter()
            .find(|f| f.path == "sub/new.txt")
            .unwrap();
        discard(&canonical, untracked).unwrap();
        assert!(!root.join("sub/new.txt").exists(), "untracked: deleted");

        let id = commit(&canonical, "second").unwrap();
        assert!(!id.is_empty());
        assert_eq!(super::status(root).unwrap().files, []);
        assert!(!commit(&canonical, "nothing").unwrap_err().is_empty());
    }

    #[test]
    fn reads_the_staged_text_and_the_branch() {
        let Some(dir) = repo() else {
            eprintln!("git isn't available: skipped");
            return;
        };
        let file = dir.path().join("file.txt");
        std::fs::write(&file, "one\nchanged\n").unwrap();
        assert_eq!(
            staged_text(&file).as_deref(),
            Some("one\ntwo\n"),
            "not what's on disk"
        );
        assert_eq!(branch(&file).as_deref(), Some("main"));
        assert_eq!(branch(dir.path()).as_deref(), Some("main"));
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        assert_eq!(staged_text(&dir.path().join("new.txt")), None, "untracked");
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), "x").unwrap();
        assert_eq!(staged_text(&outside.path().join("a.txt")), None);
    }
}

/// For tests here and elsewhere: a temp repository.
#[cfg(test)]
pub mod tests_support {
    use std::process::{Command, Stdio};

    /// A temp repository with `file.txt` ("one\ntwo\n") committed on
    /// `main`; `None` if git isn't there to run.
    pub fn repo() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().ok()?;
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if !run(&["init", "-q", "-b", "main"]) {
            return None;
        }
        std::fs::write(dir.path().join("file.txt"), "one\ntwo\n").ok()?;
        let committed = run(&["add", "file.txt"])
            && run(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "first",
            ]);
        committed.then_some(dir)
    }
}
