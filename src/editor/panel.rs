//! The bottom panel, as VS Code's: tabs for Problems (every error and
//! warning in the open files, by file), Output (the messages ferra showed
//! and what language servers logged) and Terminal (`terminal_panel.rs`).
//! Problems and Output have focus in `Mode::Panel` (`[keys.panel]`); the
//! terminal in `Mode::Terminal`.

use std::cell::Cell;
use std::path::PathBuf;

use super::diagnostics::Diagnostic;
use super::{Editor, Message, Mode, Severity};

/// The panel's tabs, in order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PanelTab {
    Problems,
    Output,
    #[default]
    Terminal,
}

impl PanelTab {
    pub const ALL: [PanelTab; 3] = [PanelTab::Problems, PanelTab::Output, PanelTab::Terminal];

    pub fn title(self) -> &'static str {
        match self {
            PanelTab::Problems => "PROBLEMS",
            PanelTab::Output => "OUTPUT",
            PanelTab::Terminal => "TERMINAL",
        }
    }
}

/// A line of the Output tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLine {
    /// `09:30:05` (UTC).
    pub time: String,
    pub error: bool,
    pub text: String,
}

/// A row of the Problems tab: a file, or one of its problems.
#[derive(Debug, Clone, PartialEq)]
pub enum ProblemRow {
    File {
        path: PathBuf,
        count: usize,
    },
    Problem {
        path: PathBuf,
        diagnostic: Diagnostic,
    },
}

/// How many lines the Output tab keeps.
const OUTPUT_LINES: usize = 1000;

#[derive(Debug, Default)]
pub struct PanelState {
    pub tab: PanelTab,
    /// The selected problem (a row of `problem_rows`).
    pub selected: usize,
    /// The first row shown, of the Problems or Output tab (the UI keeps
    /// the selection, or the Output's end, in sight).
    pub scroll: Cell<usize>,
    pub output: Vec<OutputLine>,
    /// The Output tab was scrolled back (else it follows the end).
    pub output_held: bool,
}

impl Editor {
    /// The panel's tab shown, if the panel's open (the terminal's only
    /// with a terminal).
    pub fn panel_shown(&self) -> Option<PanelTab> {
        if !self.panel_open {
            return None;
        }
        match self.panel.tab {
            PanelTab::Terminal if self.terminal.is_none() => None,
            tab => Some(tab),
        }
    }

    pub fn panel(&self) -> &PanelState {
        &self.panel
    }

    /// Shows the panel at `tab`, with focus (`:problems`, `:output`, a tab
    /// clicked). The terminal's starts a shell if there's none.
    pub fn panel_show(&mut self, tab: PanelTab) -> Result<(), String> {
        if tab == PanelTab::Terminal {
            return self.focus_terminal();
        }
        self.panel.tab = tab;
        self.panel_open = true;
        self.panel.scroll.set(0);
        self.panel.output_held = false;
        if tab == PanelTab::Problems {
            self.panel.selected = self.first_problem_row(0);
        }
        self.set_mode(Mode::Panel);
        Ok(())
    }

    /// `Ctrl+J` (`:panel`): shows the panel at its last tab, or hides it.
    pub fn panel_toggle(&mut self) -> Result<(), String> {
        if self.panel_shown().is_some() {
            self.panel_open = false;
            if matches!(self.mode(), Mode::Panel | Mode::Terminal) {
                self.set_mode(Mode::Normal);
            }
            return Ok(());
        }
        let tab = self.panel.tab;
        self.panel_show(tab)
    }

    /// `<Esc>` in the panel: back to the text, leaving it open.
    pub fn panel_leave(&mut self) {
        if self.mode() == Mode::Panel {
            self.set_mode(Mode::Normal);
        }
    }

    /// `<Tab>` / `<S-Tab>`: the next or previous tab.
    pub fn panel_cycle(&mut self, delta: isize) -> Result<(), String> {
        let at = PanelTab::ALL
            .iter()
            .position(|t| *t == self.panel.tab)
            .unwrap_or(0) as isize;
        let tab = PanelTab::ALL[(at + delta).rem_euclid(3) as usize];
        self.panel_show(tab)
    }

    /// Every problem in the open files (and others the servers reported),
    /// by file, then place.
    pub fn problems(&self) -> Vec<(PathBuf, Diagnostic)> {
        let mut found: Vec<(PathBuf, Diagnostic)> = Vec::new();
        for document in std::iter::once(&self.document).chain(&self.background) {
            if let Some(path) = document.path() {
                found.extend(
                    document
                        .diagnostics()
                        .iter()
                        .map(|d| (path.to_path_buf(), d.clone())),
                );
            }
        }
        for (path, list) in &self.file_diagnostics {
            if self.find_open_path(path).is_none() {
                found.extend(list.iter().map(|d| (path.clone(), d.clone())));
            }
        }
        found.sort_by(|(pa, a), (pb, b)| (pa, a.start).cmp(&(pb, b.start)));
        found
    }

    /// The Problems tab's rows: each file, then its problems.
    pub fn problem_rows(&self) -> Vec<ProblemRow> {
        let mut rows = Vec::new();
        let problems = self.problems();
        let mut i = 0;
        while i < problems.len() {
            let path = problems[i].0.clone();
            let count = problems[i..].iter().take_while(|(p, _)| *p == path).count();
            rows.push(ProblemRow::File {
                path: path.clone(),
                count,
            });
            for (path, diagnostic) in &problems[i..i + count] {
                rows.push(ProblemRow::Problem {
                    path: path.clone(),
                    diagnostic: diagnostic.clone(),
                });
            }
            i += count;
        }
        rows
    }

    /// The first problem's row from `from` on (files' rows are skipped).
    fn first_problem_row(&self, from: usize) -> usize {
        let rows = self.problem_rows();
        (from..rows.len())
            .find(|&i| matches!(rows[i], ProblemRow::Problem { .. }))
            .unwrap_or(0)
    }

    /// `j` / `k` (and the wheel): the next or previous problem, or the
    /// Output scrolled.
    pub fn panel_move(&mut self, delta: isize) {
        match self.panel.tab {
            PanelTab::Problems => {
                let rows = self.problem_rows();
                let mut at = self.panel.selected as isize;
                for _ in 0..delta.unsigned_abs() {
                    let step = delta.signum();
                    let mut next = at + step;
                    while next >= 0
                        && (next as usize) < rows.len()
                        && matches!(rows[next as usize], ProblemRow::File { .. })
                    {
                        next += step;
                    }
                    if next < 0 || next as usize >= rows.len() {
                        break;
                    }
                    at = next;
                }
                self.panel.selected = at.max(0) as usize;
            }
            PanelTab::Output => {
                let last = self.panel.output.len().saturating_sub(1);
                let top = (self.panel.scroll.get() as isize + delta).clamp(0, last as isize);
                self.panel.scroll.set(top as usize);
                self.panel.output_held = delta < 0 || self.panel.output_held;
            }
            PanelTab::Terminal => {}
        }
    }

    /// `gg` / `G`.
    pub fn panel_jump(&mut self, last: bool) {
        match self.panel.tab {
            PanelTab::Problems => {
                let rows = self.problem_rows();
                self.panel.selected = if last {
                    (0..rows.len())
                        .rev()
                        .find(|&i| matches!(rows[i], ProblemRow::Problem { .. }))
                        .unwrap_or(0)
                } else {
                    self.first_problem_row(0)
                };
            }
            PanelTab::Output => {
                self.panel.output_held = !last;
                self.panel.scroll.set(0);
            }
            PanelTab::Terminal => {}
        }
    }

    /// Selects the Problems tab's row `index` (a click).
    pub fn panel_select(&mut self, index: usize) {
        if self.panel.tab == PanelTab::Problems
            && matches!(
                self.problem_rows().get(index),
                Some(ProblemRow::Problem { .. })
            )
        {
            self.panel.selected = index;
        }
    }

    /// `<CR>` (or a click) on a problem: its file, at its place.
    pub fn panel_open_selected(&mut self) -> Result<(), String> {
        if self.panel.tab != PanelTab::Problems {
            return Ok(());
        }
        let Some(ProblemRow::Problem { path, diagnostic }) =
            self.problem_rows().get(self.panel.selected).cloned()
        else {
            return Err("No problem selected".into());
        };
        let location = super::lsp::Location {
            path,
            position: diagnostic.start,
            line: String::new(),
        };
        self.set_mode(Mode::Normal);
        self.goto_location(&location)
    }

    /// A message shown, or a language server's log line: kept for the
    /// Output tab (the same again isn't).
    pub fn note_output(&mut self, text: &str, error: bool) {
        let text = text.trim();
        if text.is_empty() || self.panel.output.last().is_some_and(|l| l.text == text) {
            return;
        }
        let stamp = crate::log::timestamp(std::time::SystemTime::now());
        self.panel.output.push(OutputLine {
            time: stamp.split(' ').nth(1).unwrap_or_default().to_string(),
            error,
            text: text.to_string(),
        });
        if self.panel.output.len() > OUTPUT_LINES {
            self.panel.output.remove(0);
        }
    }

    /// A message for the Output tab, as `set_message` shows it.
    pub(super) fn note_message(&mut self, message: &Message) {
        let error = message.severity == Severity::Error;
        let text = message.text.clone();
        self.note_output(&text, error);
    }
}
