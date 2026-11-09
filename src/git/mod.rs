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

/// A branch, as `git branch` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    /// The branch checked out.
    pub current: bool,
    /// The branch it tracks, and how far ahead and behind (`[ahead 1]`).
    pub upstream: String,
    pub track: String,
}

/// The local branches, the current one first.
pub fn branches(root: &Path) -> Vec<Branch> {
    let format = "--format=%(HEAD)%00%(refname:short)%00%(upstream:short)%00%(upstream:track)";
    let Some(bytes) = git(root, &["branch", format]) else {
        return Vec::new();
    };
    let mut list: Vec<Branch> = String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let head = parts.next()?;
            Some(Branch {
                current: head == "*",
                name: parts.next()?.to_string(),
                upstream: parts.next().unwrap_or("").to_string(),
                track: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect();
    list.sort_by_key(|b| !b.current);
    list
}

/// A commit, as the log shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    /// How long ago (`2 hours ago`).
    pub when: String,
}

/// The last `count` commits on `rev` (`HEAD` for the current branch).
pub fn log(root: &Path, rev: &str, count: usize) -> Vec<Commit> {
    let count = format!("-n{count}");
    let format = "--format=%H%x00%h%x00%s%x00%an%x00%ar";
    let Some(bytes) = git(root, &["log", &count, format, rev, "--"]) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            Some(Commit {
                hash: parts.next()?.to_string(),
                short: parts.next()?.to_string(),
                subject: parts.next().unwrap_or("").to_string(),
                author: parts.next().unwrap_or("").to_string(),
                when: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// A file's diff: its staged changes (against `HEAD`), or its unstaged
/// ones (against what's staged); an untracked file shows as all added.
pub fn file_diff(root: &Path, file: &FileStatus, staged: bool) -> String {
    let args: Vec<&str> = if staged {
        vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--cached",
            "--",
            &file.path,
        ]
    } else if file.unstaged == Some('U') {
        vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            &file.path,
        ]
    } else {
        vec!["diff", "--no-color", "--no-ext-diff", "--", &file.path]
    };
    // `--no-index` exits 1 when the files differ, which they do.
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    output
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// A commit's message, files changed and diff.
pub fn show_commit(root: &Path, hash: &str) -> String {
    git(
        root,
        &[
            "show",
            "--no-color",
            "--no-ext-diff",
            "--stat",
            "--patch",
            "--format=commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%n%w(0,4,4)%B",
            hash,
        ],
    )
    .map(|b| String::from_utf8_lossy(&b).into_owned())
    .unwrap_or_default()
}

/// Applies `patch` to what's staged (`reverse`: takes it back out).
pub fn apply_to_index(root: &Path, patch: &str, reverse: bool) -> Result<(), String> {
    use std::io::Write as _;
    let mut args = vec!["apply", "--cached", "--whitespace=nowarn"];
    if reverse {
        args.push("--reverse");
    }
    args.push("-");
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(error
            .lines()
            .map(|l| l.trim_start_matches("error: ").trim())
            .find(|l| !l.is_empty())
            .unwrap_or("git apply failed")
            .to_string())
    }
}

/// A line of a diff, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// `diff --git`, `index`, `---`, `+++`, a commit's header...
    Header,
    /// `@@ -1,3 +1,4 @@`
    HunkHeader,
    Added,
    Removed,
    Context,
}

/// A diff split into lines, and where its hunks are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<(DiffLineKind, String)>,
    /// Each hunk's lines (`start..end`, from its `@@` line), and the file
    /// header lines before it (for making a patch of just that hunk).
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub start: usize,
    pub end: usize,
    pub file_header: std::ops::Range<usize>,
}

impl Diff {
    pub fn parse(text: &str) -> Self {
        let mut diff = Diff::default();
        let mut file_header = 0..0;
        let mut in_file_header = false;
        for line in text.lines() {
            let i = diff.lines.len();
            let kind = if line.starts_with("diff --git") || line.starts_with("diff --cc") {
                in_file_header = true;
                file_header = i..i;
                DiffLineKind::Header
            } else if line.starts_with("@@") {
                in_file_header = false;
                diff.hunks.push(DiffHunk {
                    start: i,
                    end: i + 1,
                    file_header: file_header.clone(),
                });
                DiffLineKind::HunkHeader
            } else if in_file_header || diff.hunks.is_empty() {
                DiffLineKind::Header
            } else if line.starts_with('+') {
                DiffLineKind::Added
            } else if line.starts_with('-') {
                DiffLineKind::Removed
            } else if line.starts_with(' ') || line.starts_with('\\') {
                DiffLineKind::Context
            } else {
                // A commit's next message or stat lines: not in a hunk.
                DiffLineKind::Header
            };
            if in_file_header {
                file_header.end = i + 1;
            }
            if matches!(
                kind,
                DiffLineKind::Added | DiffLineKind::Removed | DiffLineKind::Context
            ) && let Some(hunk) = diff.hunks.last_mut()
            {
                hunk.end = i + 1;
            }
            diff.lines.push((kind, line.to_string()));
        }
        diff
    }

    /// A patch of just hunk `index`: its file's header and the hunk.
    pub fn hunk_patch(&self, index: usize) -> Option<String> {
        let hunk = self.hunks.get(index)?;
        let mut patch = String::new();
        for (_, line) in &self.lines[hunk.file_header.clone()] {
            patch.push_str(line);
            patch.push('\n');
        }
        for (_, line) in &self.lines[hunk.start..hunk.end] {
            patch.push_str(line);
            patch.push('\n');
        }
        Some(patch)
    }

    /// The hunk line `line` is in.
    pub fn hunk_at(&self, line: usize) -> Option<usize> {
        self.hunks
            .iter()
            .position(|h| (h.start..h.end).contains(&line))
    }
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
    fn branches_log_diffs_and_staging_one_hunk() {
        let Some(dir) = repo() else {
            return;
        };
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        std::fs::write(root.join("file.txt"), lines.join("\n") + "\n").unwrap();
        stage(&root, &["file.txt"]).unwrap();
        commit(&root, "twenty lines").unwrap();

        let branches = branches(&root);
        assert_eq!(branches.len(), 1);
        assert!(branches[0].current && branches[0].name == "main");
        let log = log(&root, "HEAD", 10);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "twenty lines");
        assert!(show_commit(&root, &log[0].hash).contains("+line 20"));

        // Two changes far apart: two hunks.
        let mut changed = lines.clone();
        changed[1] = "LINE 2".into();
        changed[17] = "LINE 18".into();
        std::fs::write(root.join("file.txt"), changed.join("\n") + "\n").unwrap();
        let file = status(&root).unwrap().files.remove(0);
        let diff = Diff::parse(&file_diff(&root, &file, false));
        assert_eq!(diff.hunks.len(), 2, "{diff:?}");
        let second = diff.hunks[1].start;
        assert_eq!(diff.lines[second].0, DiffLineKind::HunkHeader);
        assert_eq!(diff.hunk_at(second + 1), Some(1));

        // Stage only the second.
        apply_to_index(&root, &diff.hunk_patch(1).unwrap(), false).unwrap();
        let file = status(&root).unwrap().files.remove(0);
        assert_eq!((file.staged, file.unstaged), (Some('M'), Some('M')));
        let staged = Diff::parse(&file_diff(&root, &file, true));
        assert_eq!(staged.hunks.len(), 1);
        assert!(
            staged
                .lines
                .iter()
                .any(|(k, l)| *k == DiffLineKind::Added && l == "+LINE 18")
        );
        // And take it back out.
        apply_to_index(&root, &staged.hunk_patch(0).unwrap(), true).unwrap();
        let file = status(&root).unwrap().files.remove(0);
        assert_eq!(file.staged, None);

        // An untracked file is all added.
        std::fs::write(root.join("new.txt"), "a\nb\n").unwrap();
        let new = status(&root)
            .unwrap()
            .files
            .into_iter()
            .find(|f| f.path == "new.txt")
            .unwrap();
        let diff = Diff::parse(&file_diff(&root, &new, false));
        assert_eq!(
            diff.lines
                .iter()
                .filter(|(k, _)| *k == DiffLineKind::Added)
                .count(),
            2
        );
    }

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
