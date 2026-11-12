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

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use super::{Editor, GitRow, Message, Mode};
use crate::git::{self, Branch, Commit, Diff};

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
    pub diff_title: String,
    pub diff_of: DiffOf,
    /// The diff's first line shown.
    pub diff_scroll: usize,
    /// The selected hunk, while the diff panel has focus.
    pub hunk: usize,
    /// The lists' first rows shown (set as they're drawn).
    pub branch_scroll: Cell<usize>,
    pub commit_scroll: Cell<usize>,
    /// Rows the diff panel showed last (for keeping a hunk in view).
    pub diff_rows: Cell<usize>,
    /// The diff being read in the background, to show when it's come.
    pub diff_job: Option<DiffJob>,
    /// Commits' diffs already read.
    diff_cache: HashMap<String, Diff>,
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
    fn set_diff(&mut self, of: DiffOf, title: String, diff: Diff) {
        if of != self.diff_of {
            self.diff_scroll = 0;
            self.hunk = 0;
        }
        self.diff = diff;
        self.diff_title = title;
        self.diff_of = of;
        self.hunk = self.hunk.min(self.diff.hunks.len().saturating_sub(1));
    }

    fn finish_job(&mut self, diff: Diff) {
        let Some(job) = self.diff_job.take() else {
            return;
        };
        if let Some(key) = job.key {
            if self.diff_cache.len() >= DIFF_CACHE_SIZE {
                self.diff_cache.clear();
            }
            self.diff_cache.insert(key, diff.clone());
        }
        self.set_diff(job.of, job.title, diff);
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
                gui.set_diff(of, title, diff);
            }
            // The one asked for last wins; an earlier one's answer is dropped.
            Load::Thread(job) => gui.diff_job = Some(job),
        }
    }

    /// Takes the diff loaded in the background, if it's come. Returns
    /// whether there's anything new to draw.
    pub fn pump_git_gui_diff(&mut self) -> bool {
        let Some(gui) = self.git_gui.as_mut() else {
            return false;
        };
        let Some(job) = &gui.diff_job else {
            return false;
        };
        match job.receiver.try_recv() {
            Ok(diff) => {
                gui.finish_job(diff);
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
            Ok(diff) => gui.finish_job(diff),
            Err(_) => gui.diff_job = None,
        }
    }

    /// `1` `2` `3`, `<Tab>`, a click: focuses a panel.
    pub fn git_gui_focus(&mut self, panel: GitPanel) {
        let Some(gui) = self.git_gui.as_mut() else {
            return;
        };
        if panel == GitPanel::Diff {
            if gui.panel != GitPanel::Diff {
                gui.from = gui.panel;
            }
        } else {
            gui.from = panel;
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
                if matches!(gui.diff_of, DiffOf::File { .. }) && !gui.diff.hunks.is_empty() {
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
                return;
            }
            _ => return,
        }
        self.git_gui_show_diff();
    }

    /// `<C-d>` / `<C-u>`, the wheel: scrolls the diff.
    pub fn git_gui_scroll(&mut self, lines: isize) {
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
        match gui.panel {
            GitPanel::Diff => Ok(()),
            GitPanel::Branches => {
                let name = gui
                    .branches
                    .get(gui.branch)
                    .ok_or("No branch selected")?
                    .name
                    .clone();
                self.git_switch_branch(&name)
            }
            _ => {
                self.git_gui_focus(GitPanel::Diff);
                Ok(())
            }
        }
    }

    /// `<Esc>`: out of the diff, out of a file's history, or else closes
    /// the view.
    pub fn git_gui_back(&mut self) {
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
        // Before opening the view, so there's nothing to back out of.
        if git::file_log(&root, &relative, 1).is_empty() {
            return Err(format!("No commits have changed {relative} yet"));
        }
        self.git_show_history(&relative)
    }

    /// `H`: on a file, its history; in a history, back to the whole log.
    pub fn git_gui_history(&mut self) -> Result<(), String> {
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

    /// `s` (and `u`): in the files, stages (or unstages) the file; in a
    /// file's diff, the selected hunk.
    pub fn git_gui_stage(&mut self, stage: bool) -> Result<(), String> {
        let gui = self.git_gui.as_ref().ok_or("The git view isn't open")?;
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
                        "This hunk's staged already (u unstages it)".into()
                    } else {
                        "This hunk isn't staged (s stages it)".into()
                    });
                }
                let patch = gui.diff.hunk_patch(gui.hunk).ok_or("No hunk selected")?;
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
                self.set_message(Message::info(format!("{verb} the hunk")));
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
