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
use std::thread;
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

/// Switches to branch `name`.
pub fn switch(root: &Path, name: &str) -> Result<(), String> {
    run(root, &["switch", "--quiet", name]).map(drop)
}

/// Makes branch `name` where HEAD is, and switches to it.
pub fn create_branch(root: &Path, name: &str) -> Result<(), String> {
    run(root, &["switch", "--quiet", "-c", name]).map(drop)
}

/// Deletes branch `name` (`force`: even if it isn't merged).
pub fn delete_branch(root: &Path, name: &str, force: bool) -> Result<(), String> {
    run(
        root,
        &["branch", if force { "-D" } else { "-d" }, "--", name],
    )
    .map(drop)
}

/// How many commits the current branch is ahead of and behind its
/// upstream, if it has one.
pub fn ahead_behind(root: &Path) -> Option<(usize, usize)> {
    let bytes = git(
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    )?;
    let text = String::from_utf8_lossy(&bytes);
    let mut counts = text.split_whitespace().map(|n| n.parse().ok());
    Some((counts.next()??, counts.next()??))
}

/// The repository's remotes (`origin`...).
pub fn remotes(root: &Path) -> Vec<String> {
    git(root, &["remote"])
        .map(|b| {
            String::from_utf8_lossy(&b)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Fetching, pulling or pushing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOp {
    Fetch,
    Pull,
    Push,
}

impl RemoteOp {
    /// `Fetching`, for progress.
    pub fn doing(self) -> &'static str {
        match self {
            RemoteOp::Fetch => "Fetching",
            RemoteOp::Pull => "Pulling",
            RemoteOp::Push => "Pushing",
        }
    }

    pub fn done(self) -> &'static str {
        match self {
            RemoteOp::Fetch => "Fetched",
            RemoteOp::Pull => "Pulled",
            RemoteOp::Push => "Pushed",
        }
    }
}

/// What a remote job reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteUpdate {
    /// A line of git's progress (`Writing objects:  45% (9/20)`).
    Progress(String),
    /// It finished: what it said, or what went wrong.
    Done(Result<String, String>),
}

/// Fetching, pulling or pushing on a thread, its progress and end sent
/// through the channel returned. Nothing waits for a password: git that
/// would need one typed fails instead (`GIT_TERMINAL_PROMPT=0`), and a
/// merge that would want a message takes git's default.
pub fn start_remote(
    root: &Path,
    op: RemoteOp,
    branch: Option<(String, bool)>,
) -> std::sync::mpsc::Receiver<RemoteUpdate> {
    use std::io::Read as _;
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut args: Vec<String> = match op {
        RemoteOp::Fetch => vec!["fetch".into(), "--all".into(), "--prune".into()],
        RemoteOp::Pull => vec!["pull".into(), "--no-edit".into()],
        RemoteOp::Push => vec!["push".into()],
    };
    args.push("--progress".into());
    // The first push of a branch sets where it goes, as `-u` does.
    if let (RemoteOp::Push, Some((name, false))) = (op, &branch) {
        let remote = remotes(root).into_iter().next();
        match remote {
            Some(remote) => args.extend(["--set-upstream".into(), remote, name.clone()]),
            None => {
                let _ = sender.send(RemoteUpdate::Done(Err(
                    "No remote to push to (git remote add ...)".into(),
                )));
                return receiver;
            }
        }
    }
    // ssh would ask for a passphrase or a new host's key on the terminal
    // ferra's drawing on: in batch mode it fails instead, unless you've
    // chosen your own ssh command.
    let own_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
        || std::env::var_os("GIT_SSH").is_some()
        || git(root, &["config", "--get", "core.sshCommand"]).is_some();
    let root = root.to_path_buf();
    thread::spawn(move || {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&root)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true");
        if !own_ssh {
            command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        let child = command.spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                let _ = sender.send(RemoteUpdate::Done(Err(format!("git: {e}"))));
                return;
            }
        };
        // Progress comes on stderr, a line at a time, redrawn with `\r`.
        let mut said = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let mut buf = [0u8; 4096];
            let mut line = String::new();
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for c in String::from_utf8_lossy(&buf[..n]).chars() {
                    if c == '\r' || c == '\n' {
                        let text = line.trim().to_string();
                        if !text.is_empty() {
                            let _ = sender.send(RemoteUpdate::Progress(text.clone()));
                            if c == '\n' {
                                said.push_str(&text);
                                said.push('\n');
                            }
                        }
                        line.clear();
                    } else {
                        line.push(c);
                    }
                }
            }
        }
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        let status = child.wait();
        let ok = status.as_ref().is_ok_and(|s| s.success());
        let result = if ok {
            Ok(out.trim().to_string())
        } else {
            let error = said
                .lines()
                .rev()
                .map(|l| {
                    l.trim_start_matches("error: ")
                        .trim_start_matches("fatal: ")
                        .trim()
                })
                .find(|l| !l.is_empty() && !l.starts_with("hint:"))
                .unwrap_or("git failed")
                .to_string();
            Err(error)
        };
        let _ = sender.send(RemoteUpdate::Done(result));
    });
    receiver
}

/// The last commit's message.
pub fn last_message(root: &Path) -> Option<String> {
    git(root, &["log", "-1", "--format=%B"])
        .map(|b| String::from_utf8_lossy(&b).trim_end().to_string())
}

/// Folds what's staged into the last commit, with `message`. Returns its
/// new short id.
pub fn amend(root: &Path, message: &str) -> Result<String, String> {
    run(root, &["commit", "-q", "--amend", "-m", message])?;
    Ok(run(root, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_string())
}

/// A stash, as `git stash list` gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// `stash@{0}`.
    pub name: String,
    /// `On main: message`.
    pub subject: String,
}

/// Puts every change (untracked files too) away in a stash.
pub fn stash_push(root: &Path, message: &str) -> Result<(), String> {
    let mut args = vec!["stash", "push", "--include-untracked", "--quiet"];
    if !message.trim().is_empty() {
        args.extend(["-m", message]);
    }
    run(root, &args).map(drop)
}

/// The stashes, newest first.
pub fn stashes(root: &Path) -> Vec<Stash> {
    let Some(bytes) = git(root, &["stash", "list", "--format=%gd%x00%gs"]) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| {
            let (name, subject) = line.split_once('\0')?;
            Some(Stash {
                name: name.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

/// Puts stash `name`'s changes back and drops it.
pub fn stash_pop(root: &Path, name: &str) -> Result<(), String> {
    run(root, &["stash", "pop", "--quiet", name]).map(drop)
}

/// Who last changed a line, and in which commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blame {
    pub author: String,
    /// Seconds since 1970 (`author-time`).
    pub time: i64,
    pub summary: String,
    /// A line changed since the last commit (git says "Not Committed Yet").
    pub uncommitted: bool,
}

impl Blame {
    /// `Jon, 3 days ago • Parse the command line`, or `You • not committed
    /// yet`.
    pub fn describe(&self, now: i64) -> String {
        if self.uncommitted {
            return "You • not committed yet".into();
        }
        format!(
            "{}, {} • {}",
            self.author,
            time_ago(now - self.time),
            self.summary
        )
    }
}

/// A short age, for a column: `5m`, `3h`, `2d`, `4w`, `6mo`, `1y`, `now`.
pub fn short_age(seconds: i64) -> String {
    let units = [
        (60 * 60 * 24 * 365, "y"),
        (60 * 60 * 24 * 30, "mo"),
        (60 * 60 * 24 * 7, "w"),
        (60 * 60 * 24, "d"),
        (60 * 60, "h"),
        (60, "m"),
    ];
    for (size, name) in units {
        let n = seconds / size;
        if n >= 1 {
            return format!("{n}{name}");
        }
    }
    "now".into()
}

/// `5 minutes ago`, `3 days ago`, `2 years ago`.
pub fn time_ago(seconds: i64) -> String {
    let units = [
        (60 * 60 * 24 * 365, "year"),
        (60 * 60 * 24 * 30, "month"),
        (60 * 60 * 24 * 7, "week"),
        (60 * 60 * 24, "day"),
        (60 * 60, "hour"),
        (60, "minute"),
    ];
    for (size, name) in units {
        let n = seconds / size;
        if n >= 1 {
            let plural = if n == 1 { "" } else { "s" };
            return format!("{n} {name}{plural} ago");
        }
    }
    "just now".into()
}

/// Who last changed line `row` (0-based) of the file at `path`, given the
/// buffer's text (`contents`, so unsaved edits count: their lines are
/// uncommitted). `None` outside a repository or for an untracked file.
pub fn blame_line(path: &Path, row: usize, contents: &str) -> Option<Blame> {
    use std::io::Write as _;
    let dir = dir_of(path)?;
    let name = path.file_name()?.to_str()?.to_string();
    let line = format!("{},{}", row + 1, row + 1);
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args([
            "blame",
            "--porcelain",
            "-L",
            &line,
            "--contents",
            "-",
            "--",
            &name,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        // Written on its own thread, so a big file can't fill the pipe
        // while git's output waits to be read.
        let contents = contents.to_string();
        thread::spawn(move || {
            let _ = stdin.write_all(contents.as_bytes());
        });
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let hash = lines.next()?.split_whitespace().next()?.to_string();
    let mut blame = Blame {
        author: String::new(),
        time: 0,
        summary: String::new(),
        uncommitted: hash.chars().all(|c| c == '0'),
    };
    for line in lines {
        if let Some(author) = line.strip_prefix("author ") {
            blame.author = author.to_string();
        } else if let Some(time) = line.strip_prefix("author-time ") {
            blame.time = time.parse().unwrap_or(0);
        } else if let Some(summary) = line.strip_prefix("summary ") {
            blame.summary = summary.to_string();
        }
    }
    Some(blame)
}

/// What a side of a side-by-side row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideKind {
    /// The same on both sides.
    Same,
    /// Changed: removed from the left, or added on the right.
    Changed,
}

/// A row of a side-by-side diff: the left (old) and right (new) line
/// numbers shown on it, either missing where the other side has lines
/// this one hasn't (a filler row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideRow {
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub kind: SideKind,
}

/// `old` and `new` lined up side by side: unchanged lines level, a
/// changed run's old and new lines next to each other, and filler rows
/// on the side that has fewer.
pub fn side_by_side(old: &str, new: &str) -> Vec<SideRow> {
    let old = lines(old);
    let new = lines(new);
    let deadline = Instant::now() + Duration::from_millis(500);
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &old, &new, Some(deadline));
    let mut rows = Vec::new();
    // Removed lines waiting for added ones to sit beside.
    let mut removed: Vec<usize> = Vec::new();
    let flush = |rows: &mut Vec<SideRow>, removed: &mut Vec<usize>, added: &[usize]| {
        let n = removed.len().max(added.len());
        for i in 0..n {
            rows.push(SideRow {
                left: removed.get(i).copied(),
                right: added.get(i).copied(),
                kind: SideKind::Changed,
            });
        }
        removed.clear();
    };
    for op in ops {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                flush(&mut rows, &mut removed, &[]);
                rows.extend((0..len).map(|i| SideRow {
                    left: Some(old_index + i),
                    right: Some(new_index + i),
                    kind: SideKind::Same,
                }));
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => removed.extend(old_index..old_index + old_len),
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                let added: Vec<usize> = (new_index..new_index + new_len).collect();
                flush(&mut rows, &mut removed, &added);
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                removed.extend(old_index..old_index + old_len);
                let added: Vec<usize> = (new_index..new_index + new_len).collect();
                flush(&mut rows, &mut removed, &added);
            }
        }
    }
    flush(&mut rows, &mut removed, &[]);
    rows
}

/// A file's text at `spec` (`HEAD:src/main.rs`, `:src/main.rs` for the
/// staged one), if it's there.
pub fn blob(root: &Path, spec: &str) -> Option<String> {
    git(root, &["show", spec]).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// One line of a whole file's blame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    /// The commit that last changed it (all `0`s if it's not committed).
    pub hash: String,
    pub author: String,
    /// Seconds since 1970 (`author-time`).
    pub time: i64,
    pub summary: String,
    pub text: String,
}

impl BlameLine {
    pub fn uncommitted(&self) -> bool {
        self.hash.chars().all(|c| c == '0')
    }

    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(7)]
    }
}

/// Who last changed each line of the file at `path`, given its text as
/// it is now (`contents`, so it matches what's being edited); `None`
/// if git can't say (an untracked file, outside a repository).
pub fn blame_file(path: &Path, contents: &str) -> Option<Vec<BlameLine>> {
    use std::io::Write as _;
    let dir = dir_of(path)?;
    let name = path.file_name()?.to_str()?.to_string();
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["blame", "--porcelain", "--contents", "-", "--", &name])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let contents = contents.to_string();
        thread::spawn(move || {
            let _ = stdin.write_all(contents.as_bytes());
        });
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_blame(&String::from_utf8_lossy(&output.stdout)))
}

/// `git blame --porcelain`'s output: a header per line (the commit
/// first), the commit's details the first time it's seen, then the line
/// after a tab.
fn parse_blame(text: &str) -> Vec<BlameLine> {
    use std::collections::HashMap;
    #[derive(Default, Clone)]
    struct Info {
        author: String,
        time: i64,
        summary: String,
    }
    let mut commits: HashMap<String, Info> = HashMap::new();
    let mut lines = Vec::new();
    let mut hash: Option<String> = None;
    for line in text.lines() {
        if let Some(code) = line.strip_prefix('\t') {
            let Some(hash) = hash.take() else {
                continue;
            };
            // Not committed: git's made-up author and summary are left out.
            let info = commits
                .get(&hash)
                .filter(|_| !hash.chars().all(|c| c == '0'))
                .cloned()
                .unwrap_or_default();
            lines.push(BlameLine {
                hash,
                author: info.author,
                time: info.time,
                summary: info.summary,
                text: code.to_string(),
            });
        } else if hash.is_none() {
            let id = line.split_whitespace().next().unwrap_or("").to_string();
            commits.entry(id.clone()).or_default();
            hash = Some(id);
        } else if let Some(id) = &hash {
            let info = commits.entry(id.clone()).or_default();
            if let Some(author) = line.strip_prefix("author ") {
                info.author = author.to_string();
            } else if let Some(time) = line.strip_prefix("author-time ") {
                info.time = time.parse().unwrap_or(0);
            } else if let Some(summary) = line.strip_prefix("summary ") {
                info.summary = summary.to_string();
            }
        }
    }
    lines
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

/// The last `count` commits that changed `path` (relative to `root`),
/// following it back through renames: each with the file's name in it.
pub fn file_log(root: &Path, path: &str, count: usize) -> Vec<(Commit, String)> {
    let count = format!("-n{count}");
    let format = "--format=%x1e%H%x00%h%x00%s%x00%an%x00%ar";
    let args = [
        "log",
        "--follow",
        "--name-only",
        "--no-color",
        &count,
        format,
        "--",
        path,
    ];
    let Some(bytes) = git(root, &args) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .split('\x1e')
        .filter_map(|entry| {
            let mut lines = entry.lines();
            let mut parts = lines.next()?.split('\0');
            let commit = Commit {
                hash: parts.next()?.to_string(),
                short: parts.next()?.to_string(),
                subject: parts.next().unwrap_or("").to_string(),
                author: parts.next().unwrap_or("").to_string(),
                when: parts.next().unwrap_or("").to_string(),
            };
            let name = lines
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or(path)
                .to_string();
            Some((commit, name))
        })
        .collect()
}

/// A commit's message and what it did to the file at `paths` (its name
/// then, and before it if it was renamed, so the rename shows as one).
pub fn show_commit_file(root: &Path, hash: &str, paths: &[&str]) -> String {
    let mut args = vec![
        "show",
        "--no-color",
        "--no-ext-diff",
        "--find-renames",
        "--patch",
        "--format=commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%n%w(0,4,4)%B",
        hash,
        "--",
    ];
    args.extend_from_slice(paths);
    git(root, &args)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
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
    fn amending_and_stashing() {
        let Some(dir) = repo() else {
            return;
        };
        let root = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(last_message(&root).as_deref(), Some("first"));
        std::fs::write(root.join("file.txt"), "one\ntwo\nthree\n").unwrap();
        stage(&root, &["file.txt"]).unwrap();
        amend(&root, "first, with three").unwrap();
        assert_eq!(log(&root, "HEAD", 10).len(), 1, "still one commit");
        assert_eq!(last_message(&root).as_deref(), Some("first, with three"));
        assert!(
            status(&root).unwrap().files.is_empty(),
            "the change went in"
        );

        std::fs::write(root.join("file.txt"), "changed\n").unwrap();
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        stash_push(&root, "wip").unwrap();
        assert!(
            status(&root).unwrap().files.is_empty(),
            "put away, untracked too"
        );
        let list = stashes(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "stash@{0}");
        assert!(list[0].subject.ends_with("wip"), "{}", list[0].subject);
        stash_pop(&root, "stash@{0}").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("file.txt")).unwrap(),
            "changed\n"
        );
        assert!(root.join("new.txt").exists());
        assert!(stashes(&root).is_empty());
        assert!(stash_pop(&root, "stash@{0}").is_err());
    }

    #[test]
    fn blames_a_line_counting_unsaved_edits() {
        let Some(dir) = repo() else {
            return;
        };
        let path = dir.path().join("file.txt");
        let blame = blame_line(&path, 1, "one\ntwo\n").unwrap();
        assert!(!blame.uncommitted);
        assert_eq!(
            (blame.author.as_str(), blame.summary.as_str()),
            ("t", "first")
        );
        assert!(
            blame
                .describe(blame.time + 3 * 86400)
                .starts_with("t, 3 days ago • first")
        );
        // A line added in the buffer, not saved: not committed.
        let blame = blame_line(&path, 1, "one\nnew\ntwo\n").unwrap();
        assert!(blame.uncommitted);
        assert_eq!(blame.describe(0), "You • not committed yet");
        assert!(
            !blame_line(&path, 2, "one\nnew\ntwo\n").unwrap().uncommitted,
            "two moved down"
        );
        std::fs::write(dir.path().join("new.txt"), "x\n").unwrap();
        assert_eq!(
            blame_line(&dir.path().join("new.txt"), 0, "x\n"),
            None,
            "untracked"
        );
    }

    #[test]
    fn times_ago() {
        assert_eq!(time_ago(30), "just now");
        assert_eq!(time_ago(60), "1 minute ago");
        assert_eq!(time_ago(7200), "2 hours ago");
        assert_eq!(time_ago(86400 * 400), "1 year ago");
    }

    /// Waits for a remote job to finish.
    fn finish(receiver: std::sync::mpsc::Receiver<RemoteUpdate>) -> Result<String, String> {
        loop {
            match receiver.recv_timeout(Duration::from_secs(20)) {
                Ok(RemoteUpdate::Done(result)) => return result,
                Ok(RemoteUpdate::Progress(_)) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    #[test]
    fn branches_and_a_remote() {
        let Some(dir) = repo() else {
            return;
        };
        let root = std::fs::canonicalize(dir.path()).unwrap();
        create_branch(&root, "feature").unwrap();
        assert_eq!(branch(&root).as_deref(), Some("feature"));
        switch(&root, "main").unwrap();
        assert_eq!(
            branches(&root)
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["main", "feature"]
        );
        delete_branch(&root, "feature", false).unwrap();
        assert_eq!(branches(&root).len(), 1);
        assert!(switch(&root, "nope").is_err());
        assert_eq!(ahead_behind(&root), None, "no upstream");

        // A bare repository as the remote.
        let remote = tempfile::tempdir().unwrap();
        let bare = remote.path().join("origin.git");
        assert!(
            Command::new("git")
                .args(["init", "-q", "--bare", "-b", "main"])
                .arg(&bare)
                .status()
                .unwrap()
                .success()
        );
        run(&root, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();
        assert_eq!(remotes(&root), ["origin"]);
        finish(start_remote(
            &root,
            RemoteOp::Push,
            Some(("main".into(), false)),
        ))
        .unwrap();
        assert_eq!(ahead_behind(&root), Some((0, 0)), "tracking now");

        std::fs::write(root.join("file.txt"), "pushed\n").unwrap();
        stage(&root, &["file.txt"]).unwrap();
        commit(&root, "second").unwrap();
        assert_eq!(ahead_behind(&root), Some((1, 0)));
        finish(start_remote(
            &root,
            RemoteOp::Push,
            Some(("main".into(), true)),
        ))
        .unwrap();
        assert_eq!(ahead_behind(&root), Some((0, 0)));
        finish(start_remote(&root, RemoteOp::Fetch, None)).unwrap();
        finish(start_remote(&root, RemoteOp::Pull, None)).unwrap();

        // A push that can't go reports why.
        run(
            &root,
            &["remote", "set-url", "origin", "/nowhere/at/all.git"],
        )
        .unwrap();
        let error = finish(start_remote(
            &root,
            RemoteOp::Push,
            Some(("main".into(), true)),
        ))
        .unwrap_err();
        assert!(!error.is_empty());
    }

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

    #[test]
    fn lines_up_two_versions_side_by_side() {
        let rows = side_by_side("a\nb\nc\nd\n", "a\nB\nc\nnew\nd\n");
        let pairs: Vec<(Option<usize>, Option<usize>, SideKind)> =
            rows.iter().map(|r| (r.left, r.right, r.kind)).collect();
        use SideKind::*;
        assert_eq!(
            pairs,
            [
                (Some(0), Some(0), Same),
                (Some(1), Some(1), Changed),
                (Some(2), Some(2), Same),
                (None, Some(3), Changed),
                (Some(3), Some(4), Same),
            ]
        );
        let rows = side_by_side("a\nb\nc\n", "a\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[1].left, rows[1].right),
            (Some(1), None),
            "filler on the right"
        );
        assert_eq!(side_by_side("", "x\n").len(), 1, "a new file");
    }

    #[test]
    fn blames_every_line_including_ones_not_committed() {
        let Some(dir) = repo() else {
            return;
        };
        let file = dir.path().join("file.txt");
        let lines = blame_file(&file, "one\ntwo\nthree\n").unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "one");
        assert_eq!(lines[0].author, "t");
        assert_eq!(lines[0].summary, "first");
        assert!(!lines[0].uncommitted());
        assert_eq!(lines[1].hash, lines[0].hash, "details shown once, kept");
        assert_eq!(lines[1].summary, "first");
        assert!(lines[2].uncommitted(), "typed, not saved");
        assert_eq!(lines[0].short().len(), 7);
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        assert_eq!(blame_file(&dir.path().join("new.txt"), "x"), None);
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
