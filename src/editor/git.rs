//! Git in the editor (`crate::git`): each open file's staged text, its
//! changes against it (for the gutter, `]c`/`[c` and the status line), and
//! the branch.
//!
//! The staged text is read when a file opens, after it's saved, and when
//! the terminal comes back into focus (git may have run meanwhile); the
//! changes are diffed again before drawing whenever the text has changed
//! since. `git = false` turns it all off.

use std::collections::HashMap;

use super::{DocumentId, Editor, Message, Mode};
use crate::events::Event;
use crate::git::{self, ChangeCounts, FileStatus, Hunk, RepoStatus};

#[derive(Debug, Default)]
pub struct GitState {
    files: HashMap<DocumentId, GitFile>,
    branch: Option<String>,
    /// The repository's changes (for the Source Control view and the
    /// file tree), read with `git status`.
    status: Option<RepoStatus>,
    /// The Source Control view's selected row and first row shown.
    selected: usize,
    pub scroll: std::cell::Cell<usize>,
    /// The file `x` asked to discard, waiting for a yes.
    discarding: Option<FileStatus>,
    /// The branch `d` asked to delete (and whether forcing), waiting.
    deleting: Option<(String, bool)>,
    /// How far the current branch is ahead of and behind its upstream.
    ahead_behind: Option<(usize, usize)>,
    /// A fetch, pull or push running in the background.
    job: Option<(git::RemoteOp, std::sync::mpsc::Receiver<git::RemoteUpdate>)>,
}

/// A row of the Source Control view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRow {
    /// `Staged (2)`, `Changes (3)`.
    Header(String),
    File {
        file: FileStatus,
        staged: bool,
    },
}

/// The Source Control view's keys (bound in the sidebar while it's shown).
pub const GIT_VIEW_KEYS: [(&str, &str); 10] = [
    ("f", "git_fetch"),
    ("p", "git_pull"),
    ("P", "git_push"),
    ("s", "git_stage"),
    ("u", "git_unstage"),
    ("x", "git_discard"),
    ("c", "git_commit"),
    ("S", "git_stage_all"),
    ("U", "git_unstage_all"),
    ("<CR>", "git_open"),
];

#[derive(Debug, Default)]
struct GitFile {
    /// The staged text (`None`: not in a repository, or untracked).
    base: Option<String>,
    hunks: Vec<Hunk>,
    /// The edit count the hunks were made at.
    made_at: Option<u64>,
}

impl Editor {
    /// Reads the staged text of buffer `id`'s file again (and the branch).
    pub fn git_refresh(&mut self, id: DocumentId) {
        if !self.options.git {
            return;
        }
        let Some(path) = self
            .document_by_id(id)
            .and_then(|d| d.path().map(|p| p.to_path_buf()))
        else {
            return;
        };
        let base = git::staged_text(&path);
        self.git.branch = git::branch(&path).or_else(|| self.git.branch.take());
        let file = self.git.files.entry(id).or_default();
        file.base = base;
        file.made_at = None;
    }

    /// The repository's changes, again: the one the current file's in, or
    /// the current folder's.
    pub fn git_refresh_status(&mut self) {
        if !self.options.git {
            self.git.status = None;
            return;
        }
        let dir = self
            .document
            .path()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .filter(|d| git::repo_root(d).is_some())
            .or_else(|| std::env::current_dir().ok());
        // The selected file stays selected (in the same section) if it's
        // still there; else the same row, as near as there is.
        let before = self.git_selected_file();
        self.git.status = dir.and_then(|d| git::status(&d));
        self.git.ahead_behind = self
            .git
            .status
            .as_ref()
            .and_then(|s| git::ahead_behind(&s.root));
        let rows = self.git_rows();
        let same = before.and_then(|(file, staged)| {
            rows.iter().position(|row| {
                matches!(row, GitRow::File { file: f, staged: s } if f.path == file.path && *s == staged)
            })
        });
        self.git.selected = same.unwrap_or(self.git.selected.min(rows.len().saturating_sub(1)));
        self.git_select_file(0);
    }

    pub fn git_status(&self) -> Option<&RepoStatus> {
        self.git.status.as_ref().filter(|_| self.options.git)
    }

    /// Git's marks for the file tree: each changed file's letter, by its
    /// full path, and the folders they're in (marked `•`).
    pub fn git_tree_marks(&self) -> HashMap<std::path::PathBuf, char> {
        let mut marks = HashMap::new();
        let Some(status) = self.git_status() else {
            return marks;
        };
        for file in &status.files {
            let letter = match (file.staged, file.unstaged) {
                (_, Some('!')) => '!',
                (_, Some(c)) => c,
                (Some(c), None) => c,
                (None, None) => continue,
            };
            let path = status.root.join(&file.path);
            for dir in path.ancestors().skip(1) {
                if !dir.starts_with(&status.root) || dir == status.root {
                    break;
                }
                marks.entry(dir.to_path_buf()).or_insert('•');
            }
            marks.insert(path, letter);
        }
        marks
    }

    /// The Source Control view's rows: what's staged, then what isn't.
    pub fn git_rows(&self) -> Vec<GitRow> {
        let Some(status) = self.git_status() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let staged: Vec<&FileStatus> = status.staged().collect();
        if !staged.is_empty() {
            rows.push(GitRow::Header(format!("Staged ({})", staged.len())));
            rows.extend(staged.into_iter().map(|f| GitRow::File {
                file: f.clone(),
                staged: true,
            }));
        }
        let changes: Vec<&FileStatus> = status.unstaged().collect();
        if !changes.is_empty() {
            rows.push(GitRow::Header(format!("Changes ({})", changes.len())));
            rows.extend(changes.into_iter().map(|f| GitRow::File {
                file: f.clone(),
                staged: false,
            }));
        }
        rows
    }

    /// The Source Control view's first row shown (set as it's drawn).
    pub fn git_scroll(&self) -> usize {
        self.git.scroll.get()
    }

    /// Scrolls the view (as it's drawn) to keep the selection in `height`
    /// rows. Returns the first row to show.
    pub fn git_first_shown(&self, height: usize) -> usize {
        let rows = self.git_rows().len();
        let mut top = self.git.scroll.get();
        let selected = self.git.selected;
        if selected < top {
            top = selected;
        } else if height > 0 && selected >= top + height {
            top = selected + 1 - height;
        }
        top = top.min(rows.saturating_sub(height.max(1)));
        self.git.scroll.set(top);
        top
    }

    /// The selected row of the Source Control view.
    pub fn git_selected(&self) -> usize {
        self.git.selected
    }

    /// Moves the selection `delta` files (skipping headers).
    pub fn git_select_file(&mut self, delta: isize) {
        let rows = self.git_rows();
        let files: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, GitRow::File { .. }))
            .map(|(i, _)| i)
            .collect();
        if files.is_empty() {
            self.git.selected = 0;
            return;
        }
        let at = files
            .iter()
            .position(|&i| i >= self.git.selected)
            .unwrap_or(files.len() - 1) as isize;
        let to = (at + delta).clamp(0, files.len() as isize - 1) as usize;
        self.git.selected = files[to];
    }

    /// A click on the view's row `index`.
    pub fn git_select_row(&mut self, index: usize) {
        if matches!(self.git_rows().get(index), Some(GitRow::File { .. })) {
            self.git.selected = index;
        }
    }

    fn git_selected_file(&self) -> Option<(FileStatus, bool)> {
        match self.git_rows().get(self.git.selected) {
            Some(GitRow::File { file, staged }) => Some((file.clone(), *staged)),
            _ => None,
        }
    }

    fn git_root(&self) -> Result<std::path::PathBuf, String> {
        self.git_status()
            .map(|s| s.root.clone())
            .ok_or_else(|| "Not in a git repository".to_string())
    }

    /// After an action: the status and the open files' staged text again.
    fn git_after_action(&mut self, result: Result<(), String>) -> Result<(), String> {
        self.git_refresh_all();
        result
    }

    /// `s`: stages the selected file.
    pub fn git_stage_selected(&mut self) -> Result<(), String> {
        let root = self.git_root()?;
        let (file, _) = self.git_selected_file().ok_or("No file selected")?;
        let result = git::stage(&root, &[&file.path]);
        self.git_after_action(result)
    }

    /// `u`: unstages the selected file.
    pub fn git_unstage_selected(&mut self) -> Result<(), String> {
        let root = self.git_root()?;
        let (file, staged) = self.git_selected_file().ok_or("No file selected")?;
        if !staged && file.staged.is_none() {
            return Err(format!("{} isn't staged", file.path));
        }
        let result = git::unstage(&root, &[&file.path]);
        self.git_after_action(result)
    }

    /// `S` / `U`: stages or unstages everything.
    pub fn git_stage_all(&mut self, stage: bool) -> Result<(), String> {
        let root = self.git_root()?;
        let status = self.git_status().cloned().unwrap_or_default();
        let paths: Vec<&str> = if stage {
            status.unstaged().map(|f| f.path.as_str()).collect()
        } else {
            status.staged().map(|f| f.path.as_str()).collect()
        };
        if paths.is_empty() {
            return Err(if stage {
                "Nothing to stage"
            } else {
                "Nothing staged"
            }
            .into());
        }
        let result = if stage {
            git::stage(&root, &paths)
        } else {
            git::unstage(&root, &paths)
        };
        self.git_after_action(result)
    }

    /// `x`: asks (in a popup) before throwing away the selected file's
    /// unstaged changes.
    pub fn git_ask_discard(&mut self) -> Result<(), String> {
        if self
            .git_gui()
            .is_some_and(|g| g.panel != super::git_gui::GitPanel::Files)
        {
            return Err("Choose a file (in [1] Files) to discard".into());
        }
        let (file, staged) = self.git_selected_file().ok_or("No file selected")?;
        if staged || file.unstaged.is_none() {
            return Err(format!("{} has no unstaged changes to discard", file.path));
        }
        let what = if file.unstaged == Some('U') {
            format!(
                "Delete {}? It's untracked, so it can't be got back.",
                file.path
            )
        } else {
            format!(
                "Discard the changes to {}? They can't be got back.",
                file.path
            )
        };
        self.git.discarding = Some(file);
        let span = |text: &str, style: Option<&str>| super::popup::PopupSpan {
            text: text.to_string(),
            style: style.map(str::to_string),
        };
        self.open_popup(super::popup::Popup {
            name: "git-discard".into(),
            title: "Discard changes".into(),
            lines: vec![
                vec![span(&what, None)],
                vec![],
                vec![
                    span("y", Some("ui.popup.title")),
                    span(" discard   ", None),
                    span("n", Some("ui.popup.title")),
                    span(" keep", None),
                ],
            ],
            footer: String::new(),
            keys: vec![
                ("y".into(), "git_discard_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..super::popup::Popup::default()
        });
        Ok(())
    }

    /// `y` in that popup: discards.
    pub fn git_discard_confirmed(&mut self) -> Result<(), String> {
        self.close_popup();
        let file = self.git.discarding.take().ok_or("Nothing to discard")?;
        let root = self.git_root()?;
        let result = git::discard(&root, &file);
        // An open buffer of it shows what's on disk now.
        self.check_disk_changes();
        self.git_after_action(result)?;
        self.set_message(Message::info(format!(
            "Discarded the changes to {}",
            file.path
        )));
        Ok(())
    }

    /// `c`: asks for a commit message (an input box).
    pub fn git_ask_commit(&mut self) -> Result<(), String> {
        let status = self.git_status().ok_or("Not in a git repository")?;
        let staged = status.staged().count();
        if staged == 0 {
            return Err("Nothing staged to commit (s stages a file, S everything)".into());
        }
        let plural = if staged == 1 { "" } else { "s" };
        let mut input = super::input_box::InputBox::new(
            &format!("Commit {staged} file{plural}"),
            "",
            "git_commit_message",
        );
        input.placeholder = "Message".into();
        self.open_input_box(input);
        Ok(())
    }

    /// Commits what's staged with `message`.
    pub fn git_commit(&mut self, message: &str) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("A commit needs a message".into());
        }
        let root = self.git_root()?;
        let result = git::commit(&root, message);
        let id = match result {
            Ok(id) => id,
            Err(error) => return self.git_after_action(Err(error)),
        };
        self.git_after_action(Ok(()))?;
        let first = message.lines().next().unwrap_or("");
        self.set_message(Message::info(format!("Committed {id}: {first}")));
        Ok(())
    }

    /// `<CR>` on a file: opens it (back in the editor).
    pub fn git_open_selected(&mut self) -> Result<(), String> {
        let root = self.git_root()?;
        let (file, _) = self.git_selected_file().ok_or("No file selected")?;
        let path = root.join(&file.path);
        if !path.exists() {
            return Err(format!("{} was deleted", file.path));
        }
        self.set_mode(Mode::Normal);
        self.open(&path).map_err(|e| e.to_string())
    }

    /// Every open buffer's staged text, again.
    pub fn git_refresh_all(&mut self) {
        let ids: Vec<DocumentId> = self.buffers().iter().map(|b| b.id).collect();
        for id in ids {
            self.git_refresh(id);
        }
        if self
            .buffers()
            .iter()
            .all(|b| self.git.files.get(&b.id).is_none_or(|f| f.base.is_none()))
        {
            // No file in a repository: the branch of the folder ferra's in.
            self.git.branch = std::env::current_dir().ok().and_then(|d| git::branch(&d));
        }
        self.git_refresh_status();
        self.git_gui_reload();
    }

    /// After an event: an opened or saved file's staged text, again.
    pub fn git_saw(&mut self, event: &Event) {
        let path = match event {
            Event::Opened { path } | Event::AfterSave { path } => path,
            _ => return,
        };
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        let id = self.buffers().iter().map(|b| b.id).find(|&id| {
            self.document_by_id(id)
                .and_then(|d| d.path())
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p == path)
        });
        if let Some(id) = id {
            self.git_refresh(id);
        }
        self.git_refresh_status();
    }

    /// Before drawing: diffs the buffers whose text changed since.
    pub fn update_git_changes(&mut self) {
        if !self.options.git {
            return;
        }
        let ids: Vec<DocumentId> = self.git.files.keys().copied().collect();
        for id in ids {
            let Some(document) = self.document_by_id(id) else {
                self.git.files.remove(&id);
                continue;
            };
            let edits = document.edit_count();
            let file = &self.git.files[&id];
            if file.made_at == Some(edits) {
                continue;
            }
            let hunks = match &file.base {
                Some(base) => git::line_changes(base, &document.buffer().text()),
                None => Vec::new(),
            };
            let file = self.git.files.get_mut(&id).expect("just looked");
            file.hunks = hunks;
            file.made_at = Some(edits);
        }
    }

    /// Buffer `id`'s changes against what's staged.
    pub fn git_hunks(&self, id: DocumentId) -> &[Hunk] {
        if !self.options.git {
            return &[];
        }
        self.git.files.get(&id).map_or(&[], |f| f.hunks.as_slice())
    }

    /// The current buffer's counts of changed lines, if it's in git.
    pub fn git_counts(&self) -> Option<ChangeCounts> {
        if !self.options.git {
            return None;
        }
        let file = self.git.files.get(&self.document.id())?;
        file.base.as_ref()?;
        Some(git::counts(&file.hunks))
    }

    pub fn git_branch(&self) -> Option<&str> {
        self.git.branch.as_deref().filter(|_| self.options.git)
    }

    /// How far the branch is ahead of and behind its upstream.
    pub fn git_ahead_behind(&self) -> Option<(usize, usize)> {
        self.git.ahead_behind.filter(|_| self.options.git)
    }

    /// `f` `p` `P`: fetches, pulls or pushes in the background.
    pub fn git_remote(&mut self, op: git::RemoteOp) -> Result<(), String> {
        if let Some((running, _)) = &self.git.job {
            return Err(format!("Still {}…", running.doing().to_lowercase()));
        }
        let root = self.git_root()?;
        let branch = git::branches(&root)
            .into_iter()
            .find(|b| b.current)
            .map(|b| (b.name, !b.upstream.is_empty()));
        if op == git::RemoteOp::Push && branch.is_none() {
            return Err("Not on a branch to push".into());
        }
        let receiver = git::start_remote(&root, op, branch);
        self.git.job = Some((op, receiver));
        self.set_message(Message::info(format!("{}…", op.doing())));
        Ok(())
    }

    /// Whether a fetch, pull or push is running (so its news is waited for).
    pub fn git_job_running(&self) -> bool {
        self.git.job.is_some()
    }

    /// Takes a running job's news: its progress as the message, and at
    /// the end what happened, everything read again (a pull changes files).
    /// Returns whether there's anything new to draw.
    pub fn pump_git_job(&mut self) -> bool {
        let Some((op, receiver)) = &self.git.job else {
            return false;
        };
        let op = *op;
        let mut progress = None;
        let mut done = None;
        loop {
            match receiver.try_recv() {
                Ok(git::RemoteUpdate::Progress(text)) => progress = Some(text),
                Ok(git::RemoteUpdate::Done(result)) => {
                    done = Some(result);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    done = Some(Err("git stopped".into()));
                    break;
                }
            }
        }
        if let Some(result) = done {
            self.git.job = None;
            match result {
                Ok(_) => self.set_message(Message::info(op.done())),
                Err(error) => {
                    self.set_message(Message::error(format!("{} failed: {error}", op.doing())))
                }
            }
            if op == git::RemoteOp::Pull {
                self.check_disk_changes();
            }
            self.git_refresh_all();
            return true;
        }
        if let Some(text) = progress {
            self.set_message(Message::info(format!("{}: {text}", op.doing())));
            return true;
        }
        false
    }

    /// Switches branch, refusing while buffers have unsaved changes (they'd
    /// be left showing the old branch's files); open files then show the
    /// new branch's.
    pub fn git_switch_branch(&mut self, name: &str) -> Result<(), String> {
        let root = self.git_root()?;
        if self.git_branch() == Some(name) {
            return Err(format!("Already on {name}"));
        }
        let unsaved = self.buffers().iter().filter(|b| b.modified).count();
        if unsaved > 0 {
            return Err(format!(
                "Save or undo the unsaved changes first ({unsaved} buffer{} with them)",
                if unsaved == 1 { "" } else { "s" }
            ));
        }
        git::switch(&root, name)?;
        self.check_disk_changes();
        self.git_refresh_all();
        self.set_message(Message::info(format!("Switched to {name}")));
        Ok(())
    }

    /// `:git_branch`: a picker of the branches, to switch to one.
    pub fn git_branch_picker(&mut self) -> Result<(), String> {
        let root = self.git_root()?;
        let items = git::branches(&root)
            .into_iter()
            .map(|b| super::picker::PickerItem {
                detail: match (b.current, b.track.is_empty()) {
                    (true, _) => "current".into(),
                    (false, false) => b.track.clone(),
                    (false, true) => b.upstream.clone(),
                },
                action: super::picker::PickerAction::RunWith(
                    "git_switch".into(),
                    vec![b.name.clone()],
                ),
                label: b.name,
            })
            .collect();
        self.open_picker(super::picker::Picker::new("Branches", items));
        Ok(())
    }

    /// `n`: asks for a new branch's name (made where HEAD is).
    pub fn git_ask_new_branch(&mut self) -> Result<(), String> {
        self.git_root()?;
        let from = self.git_branch().unwrap_or("HEAD").to_string();
        let mut input = super::input_box::InputBox::new(
            &format!("New branch from {from}"),
            "",
            "git_new_branch",
        );
        input.placeholder = "Name".into();
        self.open_input_box(input);
        Ok(())
    }

    /// Makes branch `name` and switches to it.
    pub fn git_new_branch(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A branch needs a name".into());
        }
        let root = self.git_root()?;
        let result = git::create_branch(&root, name);
        self.git_after_action(result)?;
        self.set_message(Message::info(format!("Made and switched to {name}")));
        Ok(())
    }

    /// `d` (`D`: even if unmerged): asks before deleting the branch
    /// selected in the git view.
    pub fn git_ask_delete_branch(&mut self, force: bool) -> Result<(), String> {
        let gui = self.git_gui().ok_or("The git view isn't open")?;
        if gui.panel != super::git_gui::GitPanel::Branches {
            return Err("Choose a branch (in [2] Branches) to delete".into());
        }
        let branch = gui
            .branches
            .get(gui.branch)
            .ok_or("No branch selected")?
            .clone();
        if branch.current {
            return Err(format!(
                "{} is checked out: switch to another branch first",
                branch.name
            ));
        }
        let what = if force {
            format!(
                "Delete {}, even if it isn't merged? Its commits may be lost.",
                branch.name
            )
        } else {
            format!("Delete branch {}?", branch.name)
        };
        self.git.deleting = Some((branch.name, force));
        let span = |text: &str, style: Option<&str>| super::popup::PopupSpan {
            text: text.to_string(),
            style: style.map(str::to_string),
        };
        self.open_popup(super::popup::Popup {
            name: "git-delete-branch".into(),
            title: "Delete branch".into(),
            lines: vec![
                vec![span(&what, None)],
                vec![],
                vec![
                    span("y", Some("ui.popup.title")),
                    span(" delete   ", None),
                    span("n", Some("ui.popup.title")),
                    span(" keep", None),
                ],
            ],
            keys: vec![
                ("y".into(), "git_delete_branch_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..super::popup::Popup::default()
        });
        Ok(())
    }

    /// `y` in that popup: deletes it.
    pub fn git_delete_branch_confirmed(&mut self) -> Result<(), String> {
        self.close_popup();
        let (name, force) = self.git.deleting.take().ok_or("Nothing to delete")?;
        let root = self.git_root()?;
        let result = git::delete_branch(&root, &name, force).map_err(|e| {
            if e.contains("not fully merged") {
                format!("{name} isn't merged: D deletes it anyway")
            } else {
                e
            }
        });
        self.git_after_action(result)?;
        self.set_message(Message::info(format!("Deleted {name}")));
        Ok(())
    }

    /// `]c` / `[c`: to the `count`th next (or previous) change.
    pub fn goto_git_change(&mut self, forward: bool, count: usize) -> Result<(), String> {
        let row = self.document.cursor().row;
        let starts: Vec<usize> = self
            .git_hunks(self.document.id())
            .iter()
            .map(Hunk::first_row)
            .collect();
        let mut target = None;
        let mut from = row;
        for _ in 0..count.max(1) {
            let next = if forward {
                starts.iter().copied().find(|&r| r > from)
            } else {
                starts.iter().rev().copied().find(|&r| r < from)
            };
            match next {
                Some(r) => {
                    target = Some(r);
                    from = r;
                }
                None => break,
            }
        }
        let Some(row) = target else {
            return Err(if starts.is_empty() {
                "No changes".to_string()
            } else if forward {
                "No more changes below".to_string()
            } else {
                "No more changes above".to_string()
            });
        };
        self.push_jump();
        let col = crate::core::motion::first_non_blank(self.document.buffer(), row);
        self.document
            .set_cursor(crate::core::Position::new(row, col));
        self.reveal_cursor();
        let hunks = self.git_hunks(self.document.id()).len();
        let index = starts.iter().position(|&r| r == row).map_or(0, |i| i + 1);
        self.set_message(Message::info(format!("Change {index} of {hunks}")));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangeKind;

    #[test]
    fn changes_follow_edits_and_saves() {
        let Some(dir) = crate::git::tests_support::repo() else {
            eprintln!("git isn't available: skipped");
            return;
        };
        let mut editor = Editor::default();
        editor.open(&dir.path().join("file.txt")).unwrap();
        let id = editor.document.id();
        editor.git_refresh(id);
        editor.update_git_changes();
        assert_eq!(editor.git_hunks(id), []);
        assert_eq!(editor.git_branch(), Some("main"));

        let end = editor.document.buffer().line_end(1);
        editor.document.insert(end, "\nthree");
        editor.update_git_changes();
        let hunks = editor.git_hunks(id);
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].kind, hunks[0].start), (ChangeKind::Added, 2));
        assert_eq!(editor.git_counts().unwrap().added, 1);

        editor.document.set_cursor(crate::core::Position::new(0, 0));
        editor.goto_git_change(true, 1).unwrap();
        assert_eq!(editor.document.cursor().row, 2);
        assert!(editor.goto_git_change(true, 1).is_err(), "no more below");
        assert!(editor.goto_git_change(false, 1).is_err(), "none above");
    }

    #[test]
    fn untracked_files_and_git_off_have_none() {
        let Some(dir) = crate::git::tests_support::repo() else {
            return;
        };
        std::fs::write(dir.path().join("new.txt"), "x\n").unwrap();
        let mut editor = Editor::default();
        editor.open(&dir.path().join("new.txt")).unwrap();
        let id = editor.document.id();
        editor.git_refresh(id);
        editor.update_git_changes();
        assert_eq!(editor.git_hunks(id), []);
        assert_eq!(editor.git_counts(), None);

        editor.open(&dir.path().join("file.txt")).unwrap();
        editor
            .set_option("git", toml::Value::Boolean(false), false)
            .unwrap();
        editor.git_refresh(editor.document.id());
        editor.document.insert_str("changed ");
        editor.update_git_changes();
        assert_eq!(editor.git_hunks(editor.document.id()), []);
    }
}
