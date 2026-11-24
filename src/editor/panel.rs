//! The bottom panel, as VS Code's: tabs for Problems (every error and
//! warning in the open files, by file), Locations (a list of places the
//! coding agent gave, to step through; shown once there's one), Output (the
//! messages ferra showed and what language servers logged) and Terminal
//! (`terminal_panel.rs`).
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
    Locations,
    Output,
    #[default]
    Terminal,
}

impl PanelTab {
    pub const ALL: [PanelTab; 4] = [
        PanelTab::Problems,
        PanelTab::Locations,
        PanelTab::Output,
        PanelTab::Terminal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            PanelTab::Problems => "PROBLEMS",
            PanelTab::Locations => "LOCATIONS",
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
    /// The Locations tab's list, once there's one.
    pub locations: Option<LocationList>,
}

/// Places to step through, with what they're about (the agent's answer:
/// "these need changing").
#[derive(Debug, Clone, PartialEq)]
pub struct LocationList {
    pub title: String,
    pub items: Vec<LocationItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocationItem {
    pub path: PathBuf,
    pub position: crate::core::Position,
    /// What it's about (or the line's text, if no note was given).
    pub note: String,
}

/// A row of the Locations tab: its title, a file, or one of its places.
#[derive(Debug, Clone, PartialEq)]
pub enum LocationRow {
    Title(String),
    File { path: PathBuf, count: usize },
    Item(LocationItem),
}

impl Editor {
    /// The panel's tab shown, if the panel's open (the terminal's only
    /// with a terminal).
    pub fn panel_shown(&self) -> Option<PanelTab> {
        if !self.panel_open {
            return None;
        }
        match self.panel.tab {
            PanelTab::Terminal if self.terminals.is_empty() => None,
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
        if matches!(tab, PanelTab::Problems | PanelTab::Locations) {
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
        let tabs = self.panel_tabs();
        let at = tabs.iter().position(|t| *t == self.panel.tab).unwrap_or(0) as isize;
        let tab = tabs[(at + delta).rem_euclid(tabs.len() as isize) as usize];
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
        let rows = self.list_places();
        (from..rows.len()).find(|&i| rows[i].is_some()).unwrap_or(0)
    }

    /// The tabs shown: Locations only once there's a list.
    pub fn panel_tabs(&self) -> Vec<PanelTab> {
        PanelTab::ALL
            .into_iter()
            .filter(|&t| t != PanelTab::Locations || self.panel.locations.is_some())
            .collect()
    }

    /// The Locations tab's rows: its title, then each file (in the order
    /// first given) and its places, in order.
    pub fn location_rows(&self) -> Vec<LocationRow> {
        let Some(list) = &self.panel.locations else {
            return Vec::new();
        };
        let mut files: Vec<&PathBuf> = Vec::new();
        for item in &list.items {
            if !files.contains(&&item.path) {
                files.push(&item.path);
            }
        }
        let mut rows = vec![LocationRow::Title(list.title.clone())];
        for file in files {
            let mut items: Vec<&LocationItem> =
                list.items.iter().filter(|i| &i.path == file).collect();
            items.sort_by_key(|i| i.position);
            rows.push(LocationRow::File {
                path: file.clone(),
                count: items.len(),
            });
            rows.extend(items.into_iter().cloned().map(LocationRow::Item));
        }
        rows
    }

    /// The Problems or Locations tab's rows as places: where each goes
    /// (none for a file's or the title's row).
    fn list_places(&self) -> Vec<Option<(PathBuf, crate::core::Position)>> {
        match self.panel.tab {
            PanelTab::Problems => self
                .problem_rows()
                .into_iter()
                .map(|row| match row {
                    ProblemRow::Problem { path, diagnostic } => Some((path, diagnostic.start)),
                    ProblemRow::File { .. } => None,
                })
                .collect(),
            PanelTab::Locations => self
                .location_rows()
                .into_iter()
                .map(|row| match row {
                    LocationRow::Item(item) => Some((item.path, item.position)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// How many places the Problems or Locations tab lists.
    pub fn list_places_count(&self) -> usize {
        self.list_places().iter().filter(|p| p.is_some()).count()
    }

    /// Which of them is selected, from 1.
    pub fn panel_selected_number(&self) -> usize {
        self.list_places()
            .iter()
            .take(self.panel.selected + 1)
            .filter(|p| p.is_some())
            .count()
    }

    /// The selected row's place, of the Problems or Locations tab.
    pub fn panel_selected_place(&self) -> Option<Option<(PathBuf, crate::core::Position)>> {
        self.list_places().get(self.panel.selected).cloned()
    }

    /// Puts `list` in the Locations tab and shows it, the first place
    /// selected; focus stays where it is.
    pub fn panel_show_locations(&mut self, list: LocationList) {
        self.panel.locations = Some(list);
        self.panel.tab = PanelTab::Locations;
        self.panel_open = true;
        self.panel.scroll.set(0);
        self.panel.selected = self.first_problem_row(0);
    }

    /// `j` / `k` (and the wheel): the next or previous problem, or the
    /// Output scrolled.
    pub fn panel_move(&mut self, delta: isize) {
        match self.panel.tab {
            PanelTab::Problems | PanelTab::Locations => {
                let rows = self.list_places();
                let mut at = self.panel.selected as isize;
                for _ in 0..delta.unsigned_abs() {
                    let step = delta.signum();
                    let mut next = at + step;
                    while next >= 0 && (next as usize) < rows.len() && rows[next as usize].is_none()
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
            PanelTab::Problems | PanelTab::Locations => {
                let rows = self.list_places();
                self.panel.selected = if last {
                    (0..rows.len())
                        .rev()
                        .find(|&i| rows[i].is_some())
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

    /// Selects the Problems or Locations tab's row `index` (a click).
    pub fn panel_select(&mut self, index: usize) {
        if matches!(self.panel.tab, PanelTab::Problems | PanelTab::Locations)
            && self.list_places().get(index).is_some_and(Option::is_some)
        {
            self.panel.selected = index;
        }
    }

    /// `<CR>` (or a click) on a problem or a place: its file, at it.
    pub fn panel_open_selected(&mut self) -> Result<(), String> {
        if !matches!(self.panel.tab, PanelTab::Problems | PanelTab::Locations) {
            return Ok(());
        }
        let Some(Some((path, position))) = self.list_places().get(self.panel.selected).cloned()
        else {
            return Err("Nothing selected".into());
        };
        let location = super::lsp::Location {
            path,
            position,
            line: String::new(),
        };
        self.set_mode(Mode::Normal);
        self.goto_location(&location)
    }

    /// A message shown, or a language server's log line: kept for the
    /// Output tab (the same again isn't).
    pub fn note_output(&mut self, text: &str, error: bool) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        // The same again (a server's log line, then the same shown as an
        // error): one line, an error if either was.
        if let Some(last) = self.panel.output.last_mut()
            && last.text == text
        {
            last.error |= error;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_logged_then_shown_as_an_error_is_one_error_line() {
        let mut e = Editor::default();
        e.note_output("rust-analyzer: cargo check failed", false);
        e.note_output("rust-analyzer: cargo check failed", true);
        assert_eq!(e.panel.output.len(), 1);
        assert!(e.panel.output[0].error);
        e.note_output("something else", false);
        assert_eq!(e.panel.output.len(), 2);
    }
}
