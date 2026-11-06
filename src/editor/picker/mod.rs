//! The picker: a filterable list shown in a popup (buffers, files,
//! commands, registers).
//!
//! The picker only holds state; the commands in `commands/builtin/picker.rs`
//! fill it and act on the chosen item, and `ui/picker.rs` draws it.

mod fuzzy;

pub use fuzzy::{FuzzyMatch, fuzzy_match};

use std::path::PathBuf;

use super::DocumentId;

/// What choosing an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    SwitchBuffer(DocumentId),
    RunCommand(String),
    OpenFile(PathBuf),
    /// Paste from this register after the cursor.
    Paste(char),
}

impl PickerItem {
    /// A file found under `base` (the directory given to the file picker),
    /// labelled with its path relative to it. Without a `base`, it's under
    /// the current directory and opens by its relative path, which keeps
    /// buffer names short.
    pub fn file(relative: PathBuf, base: Option<&std::path::Path>) -> Self {
        let path = match base {
            Some(dir) => dir.join(&relative),
            None => relative.clone(),
        };
        Self {
            label: relative.to_string_lossy().into_owned(),
            detail: String::new(),
            action: PickerAction::OpenFile(path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    /// The text that's matched and shown.
    pub label: String,
    /// Shown dimmed beside the label (a description, `[+]`...).
    pub detail: String,
    pub action: PickerAction,
}

/// An item that matches the current query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerMatch {
    /// Index into the picker's items.
    pub index: usize,
    /// Char positions in the label that matched the query.
    pub positions: Vec<usize>,
    score: i64,
}

#[derive(Debug, Clone)]
pub struct Picker {
    title: String,
    items: Vec<PickerItem>,
    query: String,
    matches: Vec<PickerMatch>,
    selected: usize,
    /// Long labels lose their end rather than their start (the default,
    /// which suits paths).
    keep_label_start: bool,
    /// The background job still adding items (the file listing), if any.
    loading: Option<u64>,
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        let mut picker = Self {
            title: title.into(),
            items,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            keep_label_start: false,
            loading: None,
        };
        picker.refilter();
        picker
    }

    /// Marks the picker as filled in by background job `job`, which adds
    /// items with [`Picker::extend`] until [`Picker::finish_loading`].
    pub fn loading_from(mut self, job: u64) -> Self {
        self.loading = Some(job);
        self
    }

    /// The background job still adding items, if any.
    pub fn loading(&self) -> Option<u64> {
        self.loading
    }

    pub fn finish_loading(&mut self) {
        self.loading = None;
    }

    /// Adds items (from a background listing). Only the new ones are
    /// matched against the query, and the highlighted item stays
    /// highlighted.
    pub fn extend(&mut self, items: Vec<PickerItem>) {
        let selected = self.matches.get(self.selected).map(|m| m.index);
        let first_new = self.items.len();
        self.items.extend(items);
        for index in first_new..self.items.len() {
            if let Some(FuzzyMatch { score, positions }) =
                fuzzy_match(&self.query, &self.items[index].label)
            {
                self.matches.push(PickerMatch {
                    index,
                    positions,
                    score,
                });
            }
        }
        self.sort_matches();
        self.selected = selected
            .and_then(|index| self.matches.iter().position(|m| m.index == index))
            .unwrap_or(0);
    }

    /// Starts with the given item selected (e.g. the alternate buffer), if
    /// it's in the list.
    pub fn with_selected(mut self, action: &PickerAction) -> Self {
        if let Some(i) = self
            .matches
            .iter()
            .position(|m| self.items[m.index].action == *action)
        {
            self.selected = i;
        }
        self
    }

    /// Shortens long labels at the end, for labels whose start matters
    /// most (`"a  text...`).
    pub fn keep_label_start(mut self) -> Self {
        self.keep_label_start = true;
        self
    }

    pub fn keeps_label_start(&self) -> bool {
        self.keep_label_start
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// The items matching the query, best first.
    pub fn matches(&self) -> &[PickerMatch] {
        &self.matches
    }

    pub fn item(&self, m: &PickerMatch) -> &PickerItem {
        &self.items[m.index]
    }

    /// Index into `matches()` of the highlighted row.
    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.matches.get(self.selected).map(|m| self.item(m))
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    /// Removes the last query character. Returns `false` if it was empty.
    pub fn pop(&mut self) -> bool {
        let popped = self.query.pop().is_some();
        if popped {
            self.refilter();
        }
        popped
    }

    /// Moves the selection by `delta` rows, wrapping around.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.matches.len();
        if len > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        }
    }

    fn refilter(&mut self) {
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let FuzzyMatch { score, positions } = fuzzy_match(&self.query, &item.label)?;
                Some(PickerMatch {
                    index,
                    positions,
                    score,
                })
            })
            .collect();
        self.sort_matches();
        self.selected = 0;
    }

    /// Best score first; ties keep the items' own order.
    fn sort_matches(&mut self) {
        self.matches
            .sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str) -> PickerItem {
        PickerItem {
            label: name.into(),
            detail: String::new(),
            action: PickerAction::RunCommand(name.into()),
        }
    }

    fn picker(names: &[&str]) -> Picker {
        Picker::new("Commands", names.iter().map(|n| command(n)).collect())
    }

    fn labels(picker: &Picker) -> Vec<&str> {
        picker
            .matches()
            .iter()
            .map(|m| picker.item(m).label.as_str())
            .collect()
    }

    #[test]
    fn empty_query_lists_everything_in_order() {
        let p = picker(&["write", "quit", "undo"]);
        assert_eq!(labels(&p), ["write", "quit", "undo"]);
        assert_eq!(p.selected_item().unwrap().label, "write");
    }

    #[test]
    fn typing_filters_and_ranks() {
        let mut p = picker(&["write_all", "buffer_next", "write", "abandon"]);
        p.push('w');
        p.push('r');
        assert_eq!(labels(&p), ["write_all", "write"]);
        p.pop();
        p.pop();
        p.push('b');
        p.push('n');
        assert_eq!(labels(&p)[0], "buffer_next");
    }

    #[test]
    fn selection_wraps_and_resets_on_filter() {
        let mut p = picker(&["a", "b", "c"]);
        p.move_selection(-1);
        assert_eq!(p.selected_item().unwrap().label, "c");
        p.move_selection(1);
        assert_eq!(p.selected_item().unwrap().label, "a");
        p.move_selection(2);
        p.push('b');
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn no_matches_means_no_selection() {
        let mut p = picker(&["a"]);
        p.push('z');
        assert!(p.selected_item().is_none());
        p.move_selection(1);
        assert!(p.selected_item().is_none());
    }

    #[test]
    fn pop_on_empty_query_reports_it() {
        let mut p = picker(&["a"]);
        assert!(!p.pop());
        p.push('a');
        assert!(p.pop());
    }

    #[test]
    fn can_start_on_a_chosen_item() {
        let p = picker(&["a", "b", "c"]).with_selected(&PickerAction::RunCommand("c".into()));
        assert_eq!(p.selected_item().unwrap().label, "c");
    }

    #[test]
    fn extending_matches_new_items_and_keeps_the_selection() {
        let mut p = picker(&["write", "quit"]).loading_from(7);
        assert_eq!(p.loading(), Some(7));
        p.push('w');
        assert_eq!(labels(&p), ["write"]);
        p.extend(vec![command("wrap"), command("undo"), command("aw")]);
        assert_eq!(
            labels(&p),
            ["write", "wrap", "aw"],
            "only matches added, in order"
        );
        p.move_selection(1);
        p.extend(vec![command("w")]);
        assert_eq!(
            p.selected_item().unwrap().label,
            "wrap",
            "selection stays put"
        );
        p.finish_loading();
        assert_eq!(p.loading(), None);
    }
}
