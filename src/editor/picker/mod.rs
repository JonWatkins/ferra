//! The picker: a filterable list shown in a popup (buffers, commands).
//!
//! The picker only holds state; the commands in `commands/builtin/picker.rs`
//! fill it and act on the chosen item, and `ui/picker.rs` draws it.

mod fuzzy;

pub use fuzzy::{FuzzyMatch, fuzzy_match};

use super::DocumentId;

/// What choosing an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    SwitchBuffer(DocumentId),
    RunCommand(String),
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
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        let mut picker = Self {
            title: title.into(),
            items,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
        };
        picker.refilter();
        picker
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
        // Best score first; ties keep the items' own order.
        self.matches
            .sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        self.selected = 0;
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
}
