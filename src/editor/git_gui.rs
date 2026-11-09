//! The full-screen git view (`<Space>g`, `:git`), in the style of lazygit:
//! panels for the changed files, the branches and the commits on the
//! left, and a diff of what's selected on the right. It has focus in
//! `Mode::Git`, with `[keys.git]`.
//!
//! In the files panel `s` stages or unstages the file; `<CR>` goes into
//! its diff, where `j`/`k` move between hunks and `s`/`u` stage or
//! unstage just the hunk (a patch of it applied to the index). Committing
//! and discarding ask in an input box and a popup, which come back here.

use std::cell::Cell;

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
        gui.commits = git::log(&root, "HEAD", LOG_LENGTH);
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
        let (of, title, text) = match list {
            GitPanel::Files => match selected_file {
                Some((file, staged)) => {
                    let text = git::file_diff(&root, &file, staged);
                    let what = if staged { "staged" } else { "unstaged" };
                    (
                        DiffOf::File {
                            path: file.path.clone(),
                            staged,
                        },
                        format!("{} ({what})", file.path),
                        text,
                    )
                }
                None => (DiffOf::Nothing, "No changes".into(), String::new()),
            },
            GitPanel::Branches => match gui.branches.get(gui.branch) {
                Some(branch) => {
                    let log: Vec<String> = git::log(&root, &branch.name, 100)
                        .iter()
                        .map(|c| format!("{} {}  ({}, {})", c.short, c.subject, c.author, c.when))
                        .collect();
                    (
                        DiffOf::Branch(branch.name.clone()),
                        format!("Log: {}", branch.name),
                        log.join("\n"),
                    )
                }
                None => (DiffOf::Nothing, String::new(), String::new()),
            },
            GitPanel::Commits | GitPanel::Diff => match gui.commits.get(gui.commit) {
                Some(commit) => (
                    DiffOf::Commit(commit.hash.clone()),
                    format!("Commit {}", commit.short),
                    git::show_commit(&root, &commit.hash),
                ),
                None => (DiffOf::Nothing, String::new(), String::new()),
            },
        };
        if of != gui.diff_of {
            gui.diff_scroll = 0;
            gui.hunk = 0;
        }
        gui.diff = Diff::parse(&text);
        gui.diff_title = title;
        gui.diff_of = of;
        gui.hunk = gui.hunk.min(gui.diff.hunks.len().saturating_sub(1));
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

    /// `<Esc>`: out of the diff, or else closes the view.
    pub fn git_gui_back(&mut self) {
        match self.git_gui.as_ref().map(|g| (g.panel, g.from)) {
            Some((GitPanel::Diff, from)) => self.git_gui_focus(from),
            Some(_) => self.close_git_gui(),
            None => {}
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
        assert!(editor.git_gui().unwrap().diff_title.starts_with("Commit "));
        editor.git_gui_back();
        assert!(editor.git_gui().is_none());
        assert_eq!(editor.mode(), Mode::Normal);
    }
}
