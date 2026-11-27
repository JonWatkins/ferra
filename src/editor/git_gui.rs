//! The full-screen git view (`<Space>g`, `:git`), in the style of lazygit:
//! panels for the changed files, the branches and the commits on the
//! left, and a diff of what's selected on the right. It has focus in
//! `Mode::Git`, with `[keys.git]`.
//!
//! In the files panel `s` stages or unstages the file; `<CR>` goes into
//! its diff, where `j`/`k` move between hunks and `s`/`u` stage or
//! unstage just the hunk (a patch of it applied to the index). Committing
//! and discarding ask in an input box and a popup, which come back here.
//!
//! `H` on a file (or `<Space>l` / `:git_history` in a buffer) turns the
//! commits panel into the file's history: the commits that changed it,
//! following renames, each showing just what it did to the file. `H` or
//! `<Esc>` there goes back to the whole log.
//!
//! `B` on a file (or `<Space>B` / `:git_blame_view` in a buffer) shows
//! the whole file with who last changed each line, over the panels;
//! `<CR>` on a line opens its commit, `H` the file's history, `<Esc>`
//! goes back.
//!
//! `<CR>` (or a click) on a file in the sidebar's Git tab, `v` on one
//! here, or `:git_diff` in a buffer shows its change side by side, as VS
//! Code does: the old version on the left, the new on the right, lines
//! level, changes coloured. It's read-only for now; `]c`/`[c` jump
//! between changes and `<CR>` opens the file at the line.
//!
//! The git view (`editor/git_gui.rs`, `ui/git_gui.rs`) is `Mode::Git` with
//! `[keys.git]`: a `GitGui` with the focused panel, the branches
//! (`git::branches`), the last 300 commits (`git::log`) and the diff panel's
//! `Diff`. The files panel uses Source Control's rows and selection, so the two
//! agree. Whenever the selection or focus changes, the diff is made for what
//! the focused list has selected: `git::file_diff` (`diff --cached` for a
//! staged row, `diff --no-index /dev/null` for an untracked file),
//! `show_commit`, or a branch's log. `Diff::parse` splits it into lines by kind
//! and finds the hunks, each with its file header, so `hunk_patch` can make a
//! patch of one hunk; staging it is `git apply --cached` with the patch on
//! stdin, unstaging the same with `--reverse` (as lazygit does). Picking lines
//! (`GitGui::line`, and `anchor` for a range) makes a patch of part of a hunk
//! (`Diff::lines_patch`): chosen lines kept, and of the rest, applied forward,
//! removed lines turned into context and added ones dropped (in reverse the
//! other way round, as the new side is what's in the index), with the header's
//! counts worked out again. After the diff's read again, the selected line goes
//! to the nearest changed one left. After any action the status, branches,
//! commits and diff are read again, and the selection stays on the same file in
//! the same section when it's still there. Commit and discard ask in the input
//! box and popup, which go back to the git view (they remember the mode they
//! came from). `ui::draw` draws the view over everything but the command line
//! and records its panels and their rows for the mouse (`GitGuiView::record`).
//!
//! Branches are `git switch` (`-c` for a new one) and `git branch -d`/`-D`;
//! `ahead_behind` is `rev-list --left-right --count HEAD...@{upstream}`, read
//! with the status. Fetch, pull and push (`git::start_remote`) run on a thread
//! that reads git's `--progress` lines (redrawn with `\r`) from stderr and
//! sends them, then the result, through a channel; the editor keeps the
//! receiver (`GitState::job`, one at a time) and the event loop updates it while
//! it runs, as it does the terminal panel's shell. They run with
//! `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true` and (unless you've set your own
//! ssh command) `ssh -o BatchMode=yes`, so nothing can stop to ask on the
//! terminal ferra is drawing on. A branch's first push adds `--set-upstream
//! <first remote> <branch>`.
//!
//! Amending is `commit --amend -m` with the last message (`log -1 --format=%B`)
//! put in the input box to edit; the title says if the branch isn't ahead of
//! its upstream (the commit's been pushed). Stashing is `stash push
//! --include-untracked`, the list `stash list --format=%gd%x00%gs`, and putting
//! one back `stash pop <name>`. Like switching branch, both refuse while
//! buffers have unsaved changes (`git_check_saved`) and reload open files
//! after. With the git view open, a picker, box or `:` line asked from it comes
//! back to it when it closes (`Editor::set_mode`).
//!
//! A file's history (`GitGui::history`) swaps the commits panel's log for `log
//! --follow --name-only -- <path>` (`git::file_log`), keeping the file's name
//! in each commit. A commit's diff is then `show -- <name>`
//! (`git::show_commit_file`), with the name in the commit before it added when
//! they differ, so a rename shows as one rather than as a new file. `<Esc>`
//! (`git_gui_back`) leaves the history before it closes the view.
//!
//! A commit's diff and a branch's log are read on a thread (`spawn_diff`, a
//! `DiffJob` in `GitGui::diff_job`), since `git show` takes tens of
//! milliseconds and a held `j` repeats faster than that: the selection moves at
//! once and the diff follows, the event loop taking it with
//! `update_git_diff`. Only the latest request is kept; an earlier one's
//! answer is dropped with its receiver. Commits' diffs are cached (up to 100,
//! by hash and paths), so moving back is instant. A file's diff stays
//! synchronous, as staging a hunk needs it exactly as it is.
//!
//! A file's blame (`GitGui::blame`, a `BlameView`) is drawn over the panels
//! while it's there, and the view's movement keys act on it first. It's `blame
//! --porcelain --contents -` (`git::blame_file`) given the buffer encoded as it
//! would be saved (`TextFormat::encode`), so unchanged lines match what's
//! committed; from the files panel it's the file on disk. A change side by side
//! (`GitGui::side`, a `SideView`) is the same kind of page: `git::side_by_side`
//! diffs the two texts with `similar` and lines them up into `SideRow`s
//! (unchanged lines level, a changed run's removed and added lines paired,
//! filler on the shorter side), the old text from `git show :path` or
//! `HEAD:path` (`git::blob`), the new from the file or the index. Both sides
//! are scratch documents (`SIDE_DOCUMENTS`) highlighted like the blame's.
//! The diff panel's code is too: `DiffSyntax` makes each file's old lines
//! (context and removed) and new ones (context and added) into a pair of
//! scratch documents (`diff_document`), hunk after hunk, its language from
//! the `+++`/`---` path, and maps each diff line to its row in one (context
//! to the new side). It's made again only when the diff read differs, so the
//! highlights are kept; `Editor::highlighted_documents` keeps them in the
//! caches. Hunks are parsed together though there are lines between them, so
//! a construct cut by a hunk's edge (a block comment's start) can colour
//! what follows wrongly, as in other tools' diffs.
//! Opened from the sidebar (`<CR>` and clicks in the Git tab), `<Esc>` closes
//! the view back to the sidebar (`return_to`).

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use super::{Document, DocumentId, Editor, GitRow, Message, Mode};
use crate::core::Position;
use crate::git::{self, BlameLine, Branch, Commit, Diff, SideKind, SideRow};

/// How many commits the commits panel lists.
const LOG_LENGTH: usize = 300;

/// A panel of the git view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GitPanel {
    #[default]
    Files,
    Branches,
    Commits,
    Diff,
}

impl GitPanel {
    const LISTS: [GitPanel; 3] = [GitPanel::Files, GitPanel::Branches, GitPanel::Commits];
}

/// What the diff panel shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DiffOf {
    #[default]
    Nothing,
    /// A file's staged or unstaged changes.
    File {
        path: String,
        staged: bool,
    },
    Commit(String),
    Branch(String),
}

#[derive(Debug, Default)]
pub struct GitGui {
    pub panel: GitPanel,
    /// The list panel the diff panel was entered from.
    pub from: GitPanel,
    pub branches: Vec<Branch>,
    pub branch: usize,
    pub commits: Vec<Commit>,
    pub commit: usize,
    /// The file whose history the commits panel shows (its path in the
    /// repository), instead of the branch's log.
    pub history: Option<String>,
    /// The file's name in each of `commits` (it may have been renamed).
    pub history_names: Vec<String>,
    pub diff: Diff,
    /// The diff's code, for highlighting.
    pub diff_syntax: DiffSyntax,
    pub diff_title: String,
    pub diff_of: DiffOf,
    /// The diff's first line shown.
    pub diff_scroll: usize,
    /// The selected hunk, while the diff panel has focus.
    pub hunk: usize,
    /// Picking lines (`<CR>` on a hunk): the diff line selected, and where
    /// a range (`v`) started.
    pub line: Option<usize>,
    pub anchor: Option<usize>,
    /// The lists' first rows shown (set as they're drawn).
    pub branch_scroll: Cell<usize>,
    pub commit_scroll: Cell<usize>,
    /// Rows the diff panel showed last (for keeping a hunk in view).
    pub diff_rows: Cell<usize>,
    /// The diff being read in the background, to show when it's come.
    pub diff_job: Option<DiffJob>,
    /// Commits' diffs already read.
    diff_cache: HashMap<String, Diff>,
    /// A file's blame, shown over the panels.
    pub blame: Option<BlameView>,
    /// A file's change side by side, shown over the panels.
    pub side: Option<SideView>,
}

/// A file's change side by side (`v`).
#[derive(Debug, Default)]
pub struct SideView {
    /// The file, relative to the repository.
    pub path: String,
    /// Its staged change (`HEAD` against what's staged), or else its
    /// unstaged one (what's staged against the file).
    pub staged: bool,
    /// What each side's called.
    pub titles: [String; 2],
    pub rows: Vec<SideRow>,
    /// The selected row.
    pub row: usize,
    /// The first row shown (set as it's drawn).
    pub scroll: Cell<usize>,
    /// Each side's text, for drawing and highlighting.
    pub documents: [Document; 2],
    pub language: Option<String>,
    /// The mode to go back to on `<Esc>`, closing the view (it was
    /// opened for this); `None` goes back to the panels.
    return_to: Option<Mode>,
}

/// The side-by-side view's documents (never ones that are open).
pub const SIDE_DOCUMENTS: [DocumentId; 2] = [DocumentId(u64::MAX - 2), DocumentId(u64::MAX - 3)];

impl SideView {
    /// Where each run of changed rows starts.
    pub fn changes(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&i| {
                self.rows[i].kind == SideKind::Changed
                    && (i == 0 || self.rows[i - 1].kind != SideKind::Changed)
            })
            .collect()
    }
}

/// A whole file's blame (`B`).
#[derive(Debug, Default)]
pub struct BlameView {
    /// The file, relative to the repository.
    pub path: String,
    pub lines: Vec<BlameLine>,
    /// The selected line.
    pub row: usize,
    /// The first line shown (set as it's drawn).
    pub scroll: Cell<usize>,
    /// How recent each line's change is, among the file's commits: 0
    /// for the newest quarter up to 3 for the oldest.
    pub heat: Vec<u8>,
    /// The text, for syntax highlighting.
    pub document: Document,
    /// Its language, if it's one that's highlighted.
    pub language: Option<String>,
    /// Whether `<Esc>` closes the git view too (it was opened for this).
    close_view: bool,
}

/// The blame view's document (never one that's open).
pub const BLAME_DOCUMENT: DocumentId = DocumentId(u64::MAX - 1);

/// Each line's heat: its commit's rank by time among the file's commits,
/// in quarters (0 the newest).
fn heat(lines: &[BlameLine]) -> Vec<u8> {
    let mut times: Vec<i64> = lines
        .iter()
        .filter(|l| !l.uncommitted())
        .map(|l| l.time)
        .collect();
    times.sort_unstable_by(|a, b| b.cmp(a));
    times.dedup();
    lines
        .iter()
        .map(|l| {
            let rank = times.iter().position(|t| *t == l.time).unwrap_or(0);
            (rank * 4 / times.len().max(1)) as u8
        })
        .collect()
}

impl BlameView {
    /// The selected line's commit, if it's committed.
    pub fn commit(&self) -> Option<&BlameLine> {
        self.lines.get(self.row).filter(|l| !l.uncommitted())
    }
}

/// The diff panel's code as documents, for highlighting: each file's old
/// lines (context and removed) and its new ones (context and added), hunk
/// after hunk, for files in a language that's highlighted.
#[derive(Debug, Default)]
pub struct DiffSyntax {
    pub files: Vec<DiffFile>,
    /// Each diff line's place in them: its file (in `files`), side (0 old,
    /// 1 new) and row. `None` for headers, `\ No newline` and files that
    /// aren't highlighted.
    pub lines: Vec<Option<(usize, usize, usize)>>,
}

#[derive(Debug)]
pub struct DiffFile {
    pub language: String,
    pub documents: [Document; 2],
}

/// The diff panel's documents' ids, counting down from below the blame's
/// and the side-by-side view's (never ones that are open).
pub fn diff_document(file: usize, side: usize) -> DocumentId {
    DocumentId(u64::MAX - 16 - 2 * file as u64 - side as u64)
}

impl DiffSyntax {
    pub fn new(diff: &Diff, languages: &crate::config::Languages) -> Self {
        use crate::git::DiffLineKind;
        let mut syntax = DiffSyntax {
            files: Vec::new(),
            lines: vec![None; diff.lines.len()],
        };
        // Each file's lines on each side, and where each diff line went.
        let mut file_start = 0;
        let flush = |syntax: &mut DiffSyntax, header: usize, end: usize| {
            let mut sides: [Vec<&str>; 2] = [Vec::new(), Vec::new()];
            let mut places = Vec::new();
            let (mut path, mut first_line) = (None, "");
            for i in header..end {
                let (kind, text) = &diff.lines[i];
                match kind {
                    DiffLineKind::Header => {
                        // `--- a/path` and `+++ b/path` (one may be `/dev/null`).
                        if let Some(name) = text
                            .strip_prefix("+++ ")
                            .or_else(|| text.strip_prefix("--- "))
                            .filter(|n| *n != "/dev/null")
                        {
                            let name = name.split_once('/').map_or(name, |(_, rest)| rest);
                            path = Some(std::path::PathBuf::from(name));
                        }
                    }
                    DiffLineKind::HunkHeader => {
                        // The file's first line, for a `#!` line.
                        if sides[1].is_empty() && (text.contains(" +1,") || text.contains(" +1 ")) {
                            first_line = diff
                                .lines
                                .get(i + 1)
                                .and_then(|(_, t)| t.get(1..))
                                .unwrap_or("");
                        }
                    }
                    DiffLineKind::Context if text.starts_with('\\') => {}
                    DiffLineKind::Context | DiffLineKind::Added | DiffLineKind::Removed => {
                        let code = &text[1..];
                        if *kind != DiffLineKind::Added {
                            sides[0].push(code);
                        }
                        if *kind != DiffLineKind::Removed {
                            sides[1].push(code);
                        }
                        let side = usize::from(*kind != DiffLineKind::Removed);
                        places.push((i, side, sides[side].len() - 1));
                    }
                }
            }
            // A combined diff's lines have a sign per parent: not code.
            let combined = diff
                .lines
                .get(header)
                .is_some_and(|(_, t)| t.starts_with("diff --cc"));
            let Some(language) = path
                .filter(|_| !combined && !places.is_empty())
                .and_then(|p| languages.detect(Some(&p), first_line))
            else {
                return;
            };
            let file = syntax.files.len();
            let documents = [0, 1].map(|side| {
                let mut document = Document::with_text(&sides[side].join("\n"));
                document.set_id(diff_document(file, side));
                document
            });
            syntax.files.push(DiffFile {
                language: language.to_string(),
                documents,
            });
            for (i, side, row) in places {
                syntax.lines[i] = Some((file, side, row));
            }
        };
        for (i, (_, text)) in diff.lines.iter().enumerate() {
            if text.starts_with("diff --git") || text.starts_with("diff --cc") {
                flush(&mut syntax, file_start, i);
                file_start = i;
            }
        }
        flush(&mut syntax, file_start, diff.lines.len());
        syntax
    }

    /// Every document's id, for keeping their highlights.
    pub fn document_ids(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.files
            .iter()
            .flat_map(|f| f.documents.iter().map(Document::id))
    }
}

/// A diff being read on a thread.
#[derive(Debug)]
pub struct DiffJob {
    of: DiffOf,
    title: String,
    /// What to keep it under, if it never changes (a commit).
    key: Option<String>,
    receiver: Receiver<Diff>,
}

enum Load {
    Ready(DiffOf, String, Diff),
    Thread(DiffJob),
}

/// Runs `read` on a thread, its output parsed as a diff.
fn spawn_diff(read: impl FnOnce() -> String + Send + 'static) -> Receiver<Diff> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(Diff::parse(&read()));
    });
    receiver
}

/// How many commits' diffs are kept before starting again.
const DIFF_CACHE_SIZE: usize = 100;

impl GitGui {
    fn set_diff(
        &mut self,
        of: DiffOf,
        title: String,
        diff: Diff,
        languages: &crate::config::Languages,
    ) {
        if of != self.diff_of {
            self.diff_scroll = 0;
            self.hunk = 0;
            self.line = None;
        }
        self.anchor = None;
        // The same diff read again keeps its documents (and highlights).
        if diff != self.diff || self.diff_syntax.lines.len() != diff.lines.len() {
            self.diff_syntax = DiffSyntax::new(&diff, languages);
        }
        self.diff = diff;
        self.diff_title = title;
        self.diff_of = of;
        self.hunk = self.hunk.min(self.diff.hunks.len().saturating_sub(1));
        // Still picking lines: the nearest changed line, below or else above.
        if let Some(at) = self.line {
            let changed = |i: &usize| self.diff.is_change(*i);
            self.line = (at..self.diff.lines.len())
                .find(changed)
                .or_else(|| (0..at).rev().find(changed));
            if let Some(line) = self.line {
                self.hunk = self.diff.hunk_at(line).unwrap_or(self.hunk);
            }
        }
    }

    /// The lines chosen while picking lines: the range from `v` (or the
    /// selected line), within the selected line's hunk.
    pub fn chosen_lines(&self) -> Option<std::ops::Range<usize>> {
        let line = self.line?;
        let anchor = self.anchor.unwrap_or(line);
        let hunk = self.diff.hunks.get(self.diff.hunk_at(line)?)?;
        let from = line.min(anchor).max(hunk.start);
        let to = (line.max(anchor) + 1).min(hunk.end);
        Some(from..to)
    }

    fn finish_job(&mut self, diff: Diff, languages: &crate::config::Languages) {
        let Some(job) = self.diff_job.take() else {
            return;
        };
        if let Some(key) = job.key {
            if self.diff_cache.len() >= DIFF_CACHE_SIZE {
                self.diff_cache.clear();
            }
            self.diff_cache.insert(key, diff.clone());
        }
        self.set_diff(job.of, job.title, diff, languages);
    }
}

impl Editor {
    pub fn git_gui(&self) -> Option<&GitGui> {
        self.git_gui.as_ref()
    }

    /// `<Space>g`: opens the git view, at the changed files.
    pub fn open_git_gui(&mut self) -> Result<(), String> {
        if !self.options.git {
            return Err("git is turned off (git = false)".into());
        }
        self.git_refresh_all();
        if self.git_status().is_none() {
            return Err("Not in a git repository".into());
        }
        self.git_gui = Some(GitGui::default());
        self.git_gui_reload();
        self.git_select_file(0);
        self.git_gui_show_diff();
        self.set_mode(Mode::Git);
        Ok(())
    }

    /// `q`: closes it, back to the files being edited.
    pub fn close_git_gui(&mut self) {
        self.git_gui = None;
        if self.mode() == Mode::Git {
            self.set_mode(Mode::Normal);
        }
        self.git_refresh_all();
    }

    /// The branches and commits again (after an action).
    pub(super) fn git_gui_reload(&mut self) {
        let Some(root) = self.git_status().map(|s| s.root.clone()) else {
            return;
        };
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        gui.branches = git::branches(&root);
        match &gui.history {
            Some(path) => {
                let (commits, names) = git::file_log(&root, path, LOG_LENGTH).into_iter().unzip();
                gui.commits = commits;
                gui.history_names = names;
            }
            None => {
                gui.commits = git::log(&root, "HEAD", LOG_LENGTH);
                gui.history_names.clear();
            }
        }
        gui.branch = gui.branch.min(gui.branches.len().saturating_sub(1));
        gui.commit = gui.commit.min(gui.commits.len().saturating_sub(1));
        self.git_gui_show_diff();
    }

    /// The diff for what the focused list has selected.
    fn git_gui_show_diff(&mut self) {
        let Some(root) = self.git_status().map(|s| s.root.clone()) else {
            return;
        };
        let rows = self.git_rows();
        let selected_file = match rows.get(self.git_selected()) {
            Some(GitRow::File { file, staged }) => Some((file.clone(), *staged)),
            _ => None,
        };
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        let list = match gui.panel {
            GitPanel::Diff => gui.from,
            panel => panel,
        };
        // A file's diff is read here (staging a hunk needs it as it is
        // now); a commit's or a branch's log on a thread, so moving down the
        // list doesn't wait for git each step.
        let ready = |of, title: String, text: String| Load::Ready(of, title, Diff::parse(&text));
        let load = match list {
            GitPanel::Files => match selected_file {
                Some((file, staged)) => {
                    let text = git::file_diff(&root, &file, staged);
                    let what = if staged { "staged" } else { "unstaged" };
                    ready(
                        DiffOf::File {
                            path: file.path.clone(),
                            staged,
                        },
                        format!("{} ({what})", file.path),
                        text,
                    )
                }
                None => ready(DiffOf::Nothing, "No changes".into(), String::new()),
            },
            GitPanel::Branches => match gui.branches.get(gui.branch) {
                Some(branch) => {
                    let name = branch.name.clone();
                    Load::Thread(DiffJob {
                        of: DiffOf::Branch(name.clone()),
                        title: format!("Log: {name}"),
                        key: None,
                        receiver: spawn_diff(move || {
                            git::log(&root, &name, 100)
                                .iter()
                                .map(|c| {
                                    format!("{} {}  ({}, {})", c.short, c.subject, c.author, c.when)
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }),
                    })
                }
                None => ready(DiffOf::Nothing, String::new(), String::new()),
            },
            GitPanel::Commits | GitPanel::Diff => match gui.commits.get(gui.commit) {
                Some(commit) => {
                    let hash = commit.hash.clone();
                    let (title, paths) = if gui.history.is_some() {
                        let name = gui
                            .history_names
                            .get(gui.commit)
                            .cloned()
                            .unwrap_or_default();
                        // The name before this commit, if it renamed the file.
                        let before = gui
                            .history_names
                            .get(gui.commit + 1)
                            .filter(|n| **n != name)
                            .cloned();
                        let title = format!("{name} @ {}", commit.short);
                        (
                            title,
                            Some(
                                [Some(name), before]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>(),
                            ),
                        )
                    } else {
                        (format!("Commit {}", commit.short), None)
                    };
                    let of = DiffOf::Commit(hash.clone());
                    // A commit never changes, so its diff is kept.
                    let key = format!(
                        "{hash}\0{}",
                        paths.as_deref().unwrap_or_default().join("\0")
                    );
                    match gui.diff_cache.get(&key) {
                        Some(diff) => Load::Ready(of, title, diff.clone()),
                        None => Load::Thread(DiffJob {
                            of,
                            title,
                            key: Some(key),
                            receiver: spawn_diff(move || match paths {
                                Some(paths) => {
                                    let paths: Vec<&str> =
                                        paths.iter().map(String::as_str).collect();
                                    git::show_commit_file(&root, &hash, &paths)
                                }
                                None => git::show_commit(&root, &hash),
                            }),
                        }),
                    }
                }
                None => ready(DiffOf::Nothing, String::new(), String::new()),
            },
        };
        match load {
            Load::Ready(of, title, diff) => {
                gui.diff_job = None;
                gui.set_diff(of, title, diff, &self.languages);
            }
            // The one asked for last wins; an earlier one's answer is dropped.
            Load::Thread(job) => gui.diff_job = Some(job),
        }
    }

    /// Takes the diff loaded in the background, if it's come. Returns
    /// whether there's anything new to draw.
    pub fn update_git_diff(&mut self) -> bool {
        let Some(gui) = self.git_gui.as_mut() else {
            return false;
        };
        let Some(job) = &gui.diff_job else {
            return false;
        };
        match job.receiver.try_recv() {
            Ok(diff) => {
                gui.finish_job(diff, &self.languages);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                gui.diff_job = None;
                false
            }
        }
    }

    /// Whether a diff is loading (so the event loop keeps checking).
    pub fn git_gui_diff_loading(&self) -> bool {
        self.git_gui.as_ref().is_some_and(|g| g.diff_job.is_some())
    }

    /// Waits for the diff loading, if one is (for tests and scripts).
    pub fn git_gui_wait(&mut self) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        let Some(job) = &gui.diff_job else {
            return;
        };
        match job
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
        {
            Ok(diff) => gui.finish_job(diff, &self.languages),
            Err(_) => gui.diff_job = None,
        }
    }

    /// `1` `2` `3`, `<Tab>`, a click: focuses a panel.
    pub fn git_gui_focus(&mut self, panel: GitPanel) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        gui.blame = None;
        gui.side = None;
        if panel == GitPanel::Diff {
            if gui.panel != GitPanel::Diff {
                gui.from = gui.panel;
            }
        } else {
            gui.from = panel;
            gui.line = None;
            gui.anchor = None;
        }
        gui.panel = panel;
        self.git_gui_show_diff();
        self.git_gui_keep_hunk_in_view();
    }

    /// `<Tab>` / `<S-Tab>`: the next or previous list panel.
    pub fn git_gui_cycle(&mut self, delta: isize) {
        let Some(gui) = self.git_gui.as_ref() else {
            return;
        };
        let current = match gui.panel {
            GitPanel::Diff => gui.from,
            panel => panel,
        };
        let at = GitPanel::LISTS
            .iter()
            .position(|p| *p == current)
            .unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(GitPanel::LISTS.len() as isize) as usize;
        self.git_gui_focus(GitPanel::LISTS[next]);
    }

    /// `j` / `k`: down or up the focused panel (between hunks in a file's
    /// diff, by lines in another diff).
    pub fn git_gui_move(&mut self, delta: isize) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        let step = |at: usize, len: usize| -> usize {
            (at as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize
        };
        if let Some(blame) = gui.blame.as_mut() {
            blame.row = step(blame.row, blame.lines.len());
            return;
        }
        if let Some(side) = gui.side.as_mut() {
            side.row = step(side.row, side.rows.len());
            return;
        }
        match gui.panel {
            GitPanel::Files => {
                self.git_select_file(delta);
                self.git_gui_show_diff();
            }
            GitPanel::Branches => {
                gui.branch = step(gui.branch, gui.branches.len());
                self.git_gui_show_diff();
            }
            GitPanel::Commits => {
                gui.commit = step(gui.commit, gui.commits.len());
                self.git_gui_show_diff();
            }
            GitPanel::Diff => {
                if let Some(mut line) = gui.line {
                    // Between changed lines.
                    for _ in 0..delta.unsigned_abs() {
                        let next = if delta > 0 {
                            (line + 1..gui.diff.lines.len()).find(|&i| gui.diff.is_change(i))
                        } else {
                            (0..line).rev().find(|&i| gui.diff.is_change(i))
                        };
                        match next {
                            Some(i) => line = i,
                            None => break,
                        }
                    }
                    gui.line = Some(line);
                    gui.hunk = gui.diff.hunk_at(line).unwrap_or(gui.hunk);
                    let rows = gui.diff_rows.get().max(1);
                    if line < gui.diff_scroll {
                        gui.diff_scroll = line;
                    } else if line >= gui.diff_scroll + rows {
                        gui.diff_scroll = line + 1 - rows;
                    }
                } else if matches!(gui.diff_of, DiffOf::File { .. }) && !gui.diff.hunks.is_empty() {
                    gui.hunk = step(gui.hunk, gui.diff.hunks.len());
                    self.git_gui_keep_hunk_in_view();
                } else {
                    self.git_gui_scroll(delta);
                }
            }
        }
    }

    /// A click on a list's row `index`.
    pub fn git_gui_select(&mut self, panel: GitPanel, index: usize) {
        self.git_gui_focus(panel);
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        match panel {
            GitPanel::Files => self.git_select_row(index),
            GitPanel::Branches if index < gui.branches.len() => gui.branch = index,
            GitPanel::Commits if index < gui.commits.len() => gui.commit = index,
            GitPanel::Diff => {
                if let Some(hunk) = gui.diff.hunk_at(index) {
                    gui.hunk = hunk;
                }
                if gui.line.is_some() && gui.diff.is_change(index) {
                    gui.line = Some(index);
                }
                return;
            }
            _ => return,
        }
        self.git_gui_show_diff();
    }

    /// A click on the blame's line `index`.
    pub fn git_gui_blame_select(&mut self, index: usize) {
        if let Some(blame) = self.git_gui.as_mut().and_then(|g| g.blame.as_mut())
            && index < blame.lines.len()
        {
            blame.row = index;
        }
    }

    /// `<C-d>` / `<C-u>`, the wheel: scrolls the diff.
    pub fn git_gui_scroll(&mut self, lines: isize) {
        if self
            .git_gui
            .as_ref()
            .is_some_and(|g| g.blame.is_some() || g.side.is_some())
        {
            self.git_gui_move(lines);
            return;
        }
        if let Some(gui) = self.git_gui.as_mut() {
            let last = gui.diff.lines.len().saturating_sub(1);
            gui.diff_scroll = gui.diff_scroll.saturating_add_signed(lines).min(last);
        }
    }

    fn git_gui_keep_hunk_in_view(&mut self) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        if gui.panel != GitPanel::Diff {
            return;
        }
        if let Some(hunk) = gui.diff.hunks.get(gui.hunk) {
            let rows = gui.diff_rows.get().max(1);
            if hunk.start < gui.diff_scroll || hunk.start >= gui.diff_scroll + rows {
                gui.diff_scroll = hunk.start.saturating_sub(2);
            }
        }
    }

    /// `<CR>` / `l`: into the diff (to pick hunks), from a list; on a
    /// branch, switches to it.
    pub fn git_gui_enter(&mut self) -> Result<(), String> {
        let Some(gui) = self.git_gui.as_ref() else {
            return Ok(());
        };
        if let Some(side) = &gui.side {
            let row = side.rows.get(side.row).copied();
            // The new side's line, or the next one there is.
            let line = side.rows[side.row.min(side.rows.len().saturating_sub(1))..]
                .iter()
                .find_map(|r| r.right)
                .or_else(|| row.and_then(|r| r.left))
                .unwrap_or(0);
            let root = self
                .git_status()
                .ok_or("Not in a git repository")?
                .root
                .clone();
            let path = root.join(&side.path);
            if !path.exists() {
                return Err(format!("{} was deleted", side.path));
            }
            self.close_git_gui();
            self.open(&path).map_err(|e| e.to_string())?;
            let line = line.min(self.document.buffer().line_count().saturating_sub(1));
            self.document.set_cursor(Position::new(line, 0));
            return Ok(());
        }
        if let Some(blame) = &gui.blame {
            let hash = blame
                .commit()
                .ok_or("This line isn't committed yet")?
                .hash
                .clone();
            self.git_gui_show_commit(&hash);
            return Ok(());
        }
        match gui.panel {
            GitPanel::Diff => self.git_gui_pick_lines(),
            GitPanel::Branches => {
                let name = gui
                    .branches
                    .get(gui.branch)
                    .ok_or("No branch selected")?
                    .name
                    .clone();
                self.git_switch_branch(&name)
            }
            GitPanel::Files if self.git_selected_conflicted() => {
                self.close_git_gui();
                self.git_open_selected()
            }
            _ => {
                self.git_gui_focus(GitPanel::Diff);
                Ok(())
            }
        }
    }

    /// `<CR>` on a hunk of a file's diff: picks lines in it, starting at
    /// its first changed one.
    fn git_gui_pick_lines(&mut self) -> Result<(), String> {
        let gui = self.git_gui.as_mut().ok_or("The git view isn't open")?;
        if !matches!(gui.diff_of, DiffOf::File { .. }) {
            return Ok(());
        }
        let hunk = gui.diff.hunks.get(gui.hunk).ok_or("No hunk selected")?;
        let first = (hunk.start..hunk.end)
            .find(|&i| gui.diff.is_change(i))
            .ok_or("No changed lines in this hunk")?;
        gui.line = Some(first);
        gui.anchor = None;
        Ok(())
    }

    /// Whether the selected file's in conflict (so it opens at its first
    /// conflict rather than showing a diff).
    fn git_selected_conflicted(&self) -> bool {
        matches!(
            self.git_rows().get(self.git_selected()),
            Some(GitRow::File { file, .. }) if file.conflicted()
        )
    }

    /// `<Esc>`: out of the diff, out of a file's history, or else closes
    /// the view.
    pub fn git_gui_back(&mut self) {
        if let Some(side) = self.git_gui.as_mut().and_then(|g| g.side.take()) {
            if let Some(mode) = side.return_to {
                self.close_git_gui();
                self.set_mode(mode);
            }
            return;
        }
        if let Some(blame) = self.git_gui.as_mut().and_then(|g| g.blame.take()) {
            if blame.close_view {
                self.close_git_gui();
            }
            return;
        }
        if let Some(gui) = self.git_gui.as_mut()
            && gui.line.is_some()
        {
            // Out of the range, then out of picking lines.
            if gui.anchor.take().is_none() {
                gui.line = None;
            }
            return;
        }
        match self
            .git_gui
            .as_ref()
            .map(|g| (g.panel, g.from, g.history.is_some()))
        {
            Some((GitPanel::Diff, from, _)) => self.git_gui_focus(from),
            Some((GitPanel::Commits, _, true)) => self.git_gui_set_history(None),
            Some(_) => self.close_git_gui(),
            None => {}
        }
    }

    /// The commits panel shows `path`'s history (or, with `None`, the
    /// branch's log again), focused, from its newest commit.
    fn git_gui_set_history(&mut self, path: Option<String>) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        gui.history = path;
        gui.commit = 0;
        gui.commit_scroll.set(0);
        gui.panel = GitPanel::Commits;
        gui.from = GitPanel::Commits;
        self.git_gui_reload();
    }

    /// Shows the history of `path` (relative to the repository's root),
    /// opening the git view if it isn't.
    pub fn git_show_history(&mut self, path: &str) -> Result<(), String> {
        if self.git_gui.is_none() {
            self.open_git_gui()?;
        }
        let root = self
            .git_status()
            .ok_or("Not in a git repository")?
            .root
            .clone();
        if git::file_log(&root, path, 1).is_empty() {
            return Err(format!("No commits have changed {path} yet"));
        }
        self.git_gui_set_history(Some(path.to_string()));
        Ok(())
    }

    /// `<Space>l`, `:git_history`: the current file's history, in the
    /// git view.
    pub fn git_file_history(&mut self) -> Result<(), String> {
        if !self.options.git {
            return Err("git is turned off (git = false)".into());
        }
        let (_, root, relative) = self.git_current_file()?;
        // Before opening the view, so there's nothing to back out of.
        if git::file_log(&root, &relative, 1).is_empty() {
            return Err(format!("No commits have changed {relative} yet"));
        }
        self.git_show_history(&relative)
    }

    /// `H`: on a file, its history; in a history, back to the whole log.
    pub fn git_gui_history(&mut self) -> Result<(), String> {
        let gui = self.git_gui.as_mut().ok_or("The git view isn't open")?;
        if let Some(blame) = gui.blame.take() {
            return self.git_show_history(&blame.path);
        }
        let gui = self.git_gui.as_ref().ok_or("The git view isn't open")?;
        if gui.history.is_some() && gui.panel != GitPanel::Files {
            self.git_gui_set_history(None);
            return Ok(());
        }
        match (gui.panel, &gui.diff_of) {
            (GitPanel::Files, _) | (GitPanel::Diff, DiffOf::File { .. }) => {
                let path = match self.git_rows().get(self.git_selected()) {
                    Some(GitRow::File { file, .. }) => file.path.clone(),
                    _ => return Err("Choose a file (in [1] Files) to see its history".into()),
                };
                self.git_show_history(&path)
            }
            _ => Err("Choose a file (in [1] Files) to see its history".into()),
        }
    }

    /// The commit `hash` in the diff panel, selected in the commits if
    /// it's there (the whole log, not a file's history).
    fn git_gui_show_commit(&mut self, hash: &str) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        gui.blame = None;
        if gui.history.is_some() {
            self.git_gui_set_history(None);
        }
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        match gui.commits.iter().position(|c| c.hash == hash) {
            Some(index) => {
                gui.commit = index;
                gui.panel = GitPanel::Commits;
                self.git_gui_focus(GitPanel::Diff);
            }
            // Older than the log goes back: shown on its own.
            None => {
                let Some(root) = self.git_status().map(|s| s.root.clone()) else {
                    return;
                };
                let Some(gui) = self.git_gui.as_mut() else {
                    return;
                };
                gui.panel = GitPanel::Diff;
                gui.from = GitPanel::Commits;
                let hash = hash.to_string();
                gui.diff_job = Some(DiffJob {
                    of: DiffOf::Commit(hash.clone()),
                    title: format!("Commit {}", &hash[..hash.len().min(7)]),
                    key: None,
                    receiver: spawn_diff(move || git::show_commit(&root, &hash)),
                });
            }
        }
    }

    /// Shows `path`'s blame (`contents`: its text now) over the panels,
    /// at line `row`, opening the git view if it isn't.
    fn git_show_blame(
        &mut self,
        path: &std::path::Path,
        relative: String,
        contents: &str,
        row: usize,
    ) -> Result<(), String> {
        let lines = git::blame_file(path, contents)
            .filter(|l| !l.is_empty())
            .ok_or_else(|| format!("git can't blame {relative} (is it committed?)"))?;
        let close_view = self.git_gui.is_none();
        if close_view {
            self.open_git_gui()?;
        }
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        let mut document = Document::with_text(&text.join("\n"));
        document.set_id(BLAME_DOCUMENT);
        let language = self
            .languages
            .detect(Some(path), text.first().copied().unwrap_or(""))
            .map(str::to_string);
        let gui = self.git_gui.as_mut().ok_or("The git view isn't open")?;
        gui.blame = Some(BlameView {
            path: relative,
            row: row.min(lines.len() - 1),
            heat: heat(&lines),
            lines,
            scroll: Cell::new(0),
            document,
            language,
            close_view,
        });
        Ok(())
    }

    /// `<Space>B`, `:git_blame_view`: the current file's blame, at the
    /// cursor's line.
    pub fn git_blame_view(&mut self) -> Result<(), String> {
        if !self.options.git {
            return Err("git is turned off (git = false)".into());
        }
        let (path, _, relative) = self.git_current_file()?;
        // As it'd be saved (line endings, the last line break), so lines
        // not changed match what's committed.
        let contents = self
            .document
            .format()
            .encode(&self.document.buffer().text());
        let row = self.document.cursor().row;
        self.git_show_blame(&path, relative, &contents, row)
    }

    /// `B`: the selected file's blame, as it is on disk; in the blame,
    /// back out of it.
    pub fn git_gui_blame(&mut self) -> Result<(), String> {
        let gui = self.git_gui.as_ref().ok_or("The git view isn't open")?;
        if gui.blame.is_some() {
            self.git_gui_back();
            return Ok(());
        }
        let relative = match (gui.panel, self.git_rows().get(self.git_selected())) {
            (GitPanel::Files | GitPanel::Diff, Some(GitRow::File { file, .. })) => {
                file.path.clone()
            }
            _ => return Err("Choose a file (in [1] Files) to blame".into()),
        };
        let root = self
            .git_status()
            .ok_or("Not in a git repository")?
            .root
            .clone();
        let path = root.join(&relative);
        let contents = std::fs::read_to_string(&path)
            .map_err(|_| format!("{relative} isn't there to blame"))?;
        self.git_show_blame(&path, relative, &contents, 0)
    }

    /// Shows `relative`'s staged or unstaged change side by side, opening
    /// the git view if it isn't (and closing it again on `<Esc>`).
    pub fn git_show_side(&mut self, relative: &str, staged: bool) -> Result<(), String> {
        let root = self
            .git_status()
            .ok_or("Not in a git repository")?
            .root
            .clone();
        let untracked = self
            .git_status()
            .and_then(|s| s.files.iter().find(|f| f.path == relative))
            .is_some_and(|f| f.unstaged == Some('U'));
        let staged_text = || git::blob(&root, &format!(":{relative}")).unwrap_or_default();
        let (old, new, titles) = if staged {
            (
                git::blob(&root, &format!("HEAD:{relative}")).unwrap_or_default(),
                staged_text(),
                ["HEAD".to_string(), "staged".to_string()],
            )
        } else {
            let old = if untracked {
                String::new()
            } else {
                staged_text()
            };
            let new = std::fs::read_to_string(root.join(relative)).unwrap_or_default();
            (old, new, ["staged".to_string(), "working copy".to_string()])
        };
        let rows = git::side_by_side(&old, &new);
        if rows.iter().all(|r| r.kind == SideKind::Same) {
            return Err(format!("No changes in {relative}"));
        }
        let return_to = match self.git_gui {
            Some(_) => None,
            None => {
                let mode = match self.mode() {
                    Mode::Sidebar => Mode::Sidebar,
                    _ => Mode::Normal,
                };
                self.open_git_gui()?;
                Some(mode)
            }
        };
        let text = |t: &str| t.strip_suffix('\n').unwrap_or(t).replace("\r\n", "\n");
        let mut documents = [
            Document::with_text(&text(&old)),
            Document::with_text(&text(&new)),
        ];
        for (document, id) in documents.iter_mut().zip(SIDE_DOCUMENTS) {
            document.set_id(id);
        }
        let first_line = new.lines().next().or(old.lines().next()).unwrap_or("");
        let language = self
            .languages
            .detect(Some(&root.join(relative)), first_line)
            .map(str::to_string);
        let mut view = SideView {
            path: relative.to_string(),
            staged,
            titles: titles.map(|t| format!("{relative} ({t})")),
            rows,
            documents,
            language,
            return_to,
            ..SideView::default()
        };
        view.row = view.changes().first().copied().unwrap_or(0);
        let gui = self.git_gui.as_mut().ok_or("The git view isn't open")?;
        gui.blame = None;
        gui.side = Some(view);
        Ok(())
    }

    /// `<CR>` on a file in the sidebar's Git tab, `v` in the git view:
    /// the selected file's change side by side.
    pub fn git_side_selected(&mut self) -> Result<(), String> {
        if let Some(gui) = self.git_gui.as_mut()
            && gui.line.is_some()
        {
            // Picking lines: `v` starts (or ends) a range.
            gui.anchor = match gui.anchor {
                Some(_) => None,
                None => gui.line,
            };
            return Ok(());
        }
        if let Some(gui) = &self.git_gui
            && gui.side.is_some()
        {
            self.git_gui_back();
            return Ok(());
        }
        if self.git_selected_conflicted() {
            self.close_git_gui();
            return self.git_open_selected();
        }
        match self.git_rows().get(self.git_selected()) {
            Some(GitRow::File { file, staged }) => {
                let (path, staged) = (file.path.clone(), *staged);
                self.git_show_side(&path, staged)
            }
            _ => Err("Choose a changed file".into()),
        }
    }

    /// `:git_diff`: the current file's unstaged change side by side.
    pub fn git_diff_current(&mut self) -> Result<(), String> {
        let (_, _, relative) = self.git_current_file()?;
        self.git_refresh_status();
        self.git_show_side(&relative, false)
    }

    /// The current buffer's file, its repository's root, and its path
    /// there (with `/`).
    fn git_current_file(&self) -> Result<(PathBuf, PathBuf, String), String> {
        let path = self
            .document
            .path()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .ok_or("This buffer isn't a file")?;
        let root = path
            .parent()
            .and_then(git::repo_root)
            .and_then(|r| std::fs::canonicalize(r).ok())
            .ok_or("This file isn't in a git repository")?;
        let relative = path
            .strip_prefix(&root)
            .map_err(|_| "This file isn't in the repository")?
            .to_string_lossy()
            .replace('\\', "/");
        Ok((path, root, relative))
    }

    /// `]c` / `[c` in the side-by-side view: the next or previous change.
    pub fn git_gui_change(&mut self, forward: bool) -> Result<(), String> {
        let side = self
            .git_gui
            .as_mut()
            .and_then(|g| g.side.as_mut())
            .ok_or("]c and [c move between changes side by side (v on a file)")?;
        let changes = side.changes();
        let next = if forward {
            changes.iter().find(|&&c| c > side.row)
        } else {
            changes.iter().rev().find(|&&c| c < side.row)
        };
        side.row = *next.ok_or(if forward {
            "No more changes below"
        } else {
            "No more changes above"
        })?;
        Ok(())
    }

    /// A click on the side-by-side view's row `index`.
    pub fn git_gui_side_select(&mut self, index: usize) {
        if let Some(side) = self.git_gui.as_mut().and_then(|g| g.side.as_mut())
            && index < side.rows.len()
        {
            side.row = index;
        }
    }

    /// `s` (and `u`): in the files, stages (or unstages) the file; in a
    /// file's diff, the selected hunk.
    pub fn git_gui_stage(&mut self, stage: bool) -> Result<(), String> {
        let gui = self.git_gui.as_ref().ok_or("The git view isn't open")?;
        if gui.blame.is_some() || gui.side.is_some() {
            return Err("Esc goes back to the panels first".into());
        }
        match (gui.panel, &gui.diff_of) {
            // Space on a branch switches to it, as in lazygit.
            (GitPanel::Branches, _) if stage => self.git_gui_enter(),
            (GitPanel::Files, _) => {
                let staged_row = matches!(
                    self.git_rows().get(self.git_selected()),
                    Some(GitRow::File { staged: true, .. })
                );
                // `s` on a staged file unstages it, as lazygit's space does.
                if stage && !staged_row {
                    self.git_stage_selected()
                } else {
                    self.git_unstage_selected()
                }
            }
            (GitPanel::Diff, DiffOf::File { staged, .. }) => {
                let staged = *staged;
                if stage == staged {
                    return Err(if staged {
                        "This is staged already (u unstages it)".into()
                    } else {
                        "This isn't staged (s stages it)".into()
                    });
                }
                let (patch, what) = match gui.chosen_lines() {
                    Some(chosen) => {
                        let hunk = gui.diff.hunk_at(chosen.start).unwrap_or(gui.hunk);
                        let count = chosen.clone().filter(|&i| gui.diff.is_change(i)).count();
                        let patch = gui
                            .diff
                            .lines_patch(hunk, chosen, !stage)
                            .ok_or("No changed lines chosen")?;
                        let plural = if count == 1 { "" } else { "s" };
                        (patch, format!("{count} line{plural}"))
                    }
                    None => (
                        gui.diff.hunk_patch(gui.hunk).ok_or("No hunk selected")?,
                        "the hunk".to_string(),
                    ),
                };
                let root = self
                    .git_status()
                    .ok_or("Not in a git repository")?
                    .root
                    .clone();
                git::apply_to_index(&root, &patch, !stage)?;
                self.git_refresh_all();
                // Back to the files when the diff's emptied.
                if self
                    .git_gui
                    .as_ref()
                    .is_some_and(|g| g.diff.hunks.is_empty())
                {
                    self.git_gui_focus(GitPanel::Files);
                }
                let verb = if stage { "Staged" } else { "Unstaged" };
                self.set_message(Message::info(format!("{verb} {what}")));
                Ok(())
            }
            _ => Err("Choose a file (in [1] Files) to stage".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diffs_code_is_split_into_each_files_old_and_new_text() {
        let diff = Diff::parse(concat!(
            "commit abc\n",
            "--- not a file, a commit message line\n",
            "diff --git a/src/a.rs b/src/a.rs\n",
            "index 1..2 100644\n",
            "--- a/src/a.rs\n",
            "+++ b/src/a.rs\n",
            "@@ -1,3 +1,3 @@\n",
            " fn a() {\n",
            "-    old();\n",
            "+    new();\n",
            " }\n",
            "@@ -10,1 +10,2 @@\n",
            " x\n",
            "+y\n",
            "\\ No newline at end of file\n",
            "diff --git a/notes.unknown b/notes.unknown\n",
            "--- a/notes.unknown\n",
            "+++ b/notes.unknown\n",
            "@@ -1 +1 @@\n",
            "-a\n",
            "+b\n",
            "diff --git a/gone.py b/gone.py\n",
            "deleted file mode 100644\n",
            "--- a/gone.py\n",
            "+++ /dev/null\n",
            "@@ -1 +0,0 @@\n",
            "-print(1)\n",
        ));
        let languages = crate::config::Config::default().languages;
        let syntax = DiffSyntax::new(&diff, &languages);
        let names: Vec<&str> = syntax.files.iter().map(|f| f.language.as_str()).collect();
        assert_eq!(
            names,
            ["rust", "python"],
            "a file in no language is left out"
        );
        let rust = &syntax.files[0];
        assert_eq!(
            rust.documents[0].buffer().text(),
            "fn a() {\n    old();\n}\nx"
        );
        assert_eq!(
            rust.documents[1].buffer().text(),
            "fn a() {\n    new();\n}\nx\ny"
        );
        let place = |text: &str| {
            let i = diff.lines.iter().position(|(_, t)| t == text).unwrap();
            syntax.lines[i]
        };
        assert_eq!(place(" fn a() {"), Some((0, 1, 0)), "context: the new side");
        assert_eq!(place("-    old();"), Some((0, 0, 1)));
        assert_eq!(place("+    new();"), Some((0, 1, 1)));
        assert_eq!(place("+y"), Some((0, 1, 4)));
        assert_eq!(place("\\ No newline at end of file"), None);
        assert_eq!(place("+b"), None);
        assert_eq!(place("-print(1)"), Some((1, 0, 0)), "named by its old path");
        assert_eq!(place("@@ -1,3 +1,3 @@"), None);
        let ids: Vec<DocumentId> = syntax.document_ids().collect();
        assert_eq!(ids.len(), 4);
        assert!(
            ids.iter().all(|id| id.0 > u64::MAX - 1000),
            "never an open one's"
        );
    }

    #[test]
    fn opens_on_the_changes_and_stages_one_hunk() {
        let Some(dir) = crate::git::tests_support::repo() else {
            return;
        };
        let path = dir.path().join("file.txt");
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        crate::git::stage(&root, &["file.txt"]).unwrap();
        crate::git::commit(&root, "twenty").unwrap();
        let mut changed = lines.clone();
        changed[1] = "LINE 2".into();
        changed[17] = "LINE 18".into();
        std::fs::write(&path, changed.join("\n") + "\n").unwrap();

        let mut editor = Editor::default();
        editor.open(&path).unwrap();
        editor.open_git_gui().unwrap();
        assert_eq!(editor.mode(), Mode::Git);
        let gui = editor.git_gui().unwrap();
        assert_eq!(gui.diff_title, "file.txt (unstaged)");
        assert_eq!(gui.diff.hunks.len(), 2);
        assert_eq!(gui.commits.len(), 2);

        editor.git_gui_enter().unwrap();
        editor.git_gui_move(1);
        assert_eq!(editor.git_gui().unwrap().hunk, 1);
        editor.git_gui_stage(true).unwrap();
        let status = editor.git_status().unwrap();
        assert_eq!(status.staged().count(), 1);
        assert_eq!(status.unstaged().count(), 1, "the other hunk isn't");
        let gui = editor.git_gui().unwrap();
        assert_eq!(gui.diff.hunks.len(), 1);
        assert_eq!(
            gui.diff_title, "file.txt (unstaged)",
            "still the unstaged diff"
        );

        editor.git_gui_back();
        assert_eq!(editor.git_gui().unwrap().panel, GitPanel::Files);
        editor.git_gui_cycle(2);
        assert_eq!(editor.git_gui().unwrap().panel, GitPanel::Commits);
        editor.git_gui_wait();
        assert!(editor.git_gui().unwrap().diff_title.starts_with("Commit "));
        editor.git_gui_back();
        assert!(editor.git_gui().is_none());
        assert_eq!(editor.mode(), Mode::Normal);
    }
}
