//! Prompt history, browsed with `<Up>`/`<Down>`.

/// How many entries each history keeps.
const LIMIT: usize = 100;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct History {
    /// Oldest first.
    entries: Vec<String>,
    /// The entry being shown while browsing, if any.
    position: Option<usize>,
    /// What was typed before browsing started, which `<Down>` returns to
    /// at the end.
    draft: String,
    /// What the entries browsed start with: the draft, or nothing (every
    /// entry).
    prefix: String,
}

impl History {
    #[cfg(test)]
    fn entries(&self) -> &[String] {
        &self.entries
    }

    /// The most recent entry.
    pub fn newest(&self) -> Option<&str> {
        self.entries.last().map(String::as_str)
    }

    /// Adds `entry` as the newest one. Blank entries are skipped, and an
    /// existing copy moves to the end rather than appearing twice.
    pub fn push(&mut self, entry: &str) {
        self.reset();
        if entry.trim().is_empty() {
            return;
        }
        self.entries.retain(|e| e != entry);
        self.entries.push(entry.to_string());
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
    }

    /// Stops browsing.
    pub fn reset(&mut self) {
        self.position = None;
    }

    /// The next older entry starting with what was typed when browsing
    /// began (`current`, on the first call), or `None` if there's none.
    pub fn older(&mut self, current: &str) -> Option<String> {
        self.step_older(current, true)
    }

    /// As `older`, but every entry, whatever was typed (the find box, as in
    /// VS Code).
    pub fn older_any(&mut self, current: &str) -> Option<String> {
        self.step_older(current, false)
    }

    fn step_older(&mut self, current: &str, by_prefix: bool) -> Option<String> {
        let start = match self.position {
            Some(p) => p,
            None => {
                self.draft = current.to_string();
                self.prefix = if by_prefix {
                    current.to_string()
                } else {
                    String::new()
                };
                self.entries.len()
            }
        };
        let found = (0..start)
            .rev()
            .find(|&i| self.entries[i].starts_with(&self.prefix))?;
        self.position = Some(found);
        Some(self.entries[found].clone())
    }

    /// The next newer matching entry, or the original draft once past the
    /// newest. `None` if not browsing.
    pub fn newer(&mut self) -> Option<String> {
        let current = self.position?;
        let found =
            (current + 1..self.entries.len()).find(|&i| self.entries[i].starts_with(&self.prefix));
        self.position = found;
        Some(match found {
            Some(i) => self.entries[i].clone(),
            None => self.draft.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(entries: &[&str]) -> History {
        let mut h = History::default();
        entries.iter().for_each(|e| h.push(e));
        h
    }

    #[test]
    fn browses_newest_first_and_back_to_the_draft() {
        let mut h = history(&["w", "e a.txt", "q"]);
        assert_eq!(h.older("").as_deref(), Some("q"));
        assert_eq!(h.older("").as_deref(), Some("e a.txt"));
        assert_eq!(h.older("").as_deref(), Some("w"));
        assert_eq!(h.older(""), None, "no older entries");
        assert_eq!(h.newer().as_deref(), Some("e a.txt"));
        assert_eq!(h.newer().as_deref(), Some("q"));
        assert_eq!(h.newer().as_deref(), Some(""), "back to what was typed");
        assert_eq!(h.newer(), None, "not browsing any more");
    }

    #[test]
    fn only_entries_starting_with_the_draft() {
        let mut h = history(&["e one", "w", "e two"]);
        assert_eq!(h.older("e ").as_deref(), Some("e two"));
        assert_eq!(h.older("ignored once browsing").as_deref(), Some("e one"));
        assert_eq!(h.newer().as_deref(), Some("e two"));
        assert_eq!(h.newer().as_deref(), Some("e "));
    }

    #[test]
    fn older_any_ignores_what_was_typed_but_comes_back_to_it() {
        let mut h = history(&["one", "two"]);
        assert_eq!(h.older_any("t").as_deref(), Some("two"));
        assert_eq!(h.older_any("ignored").as_deref(), Some("one"));
        assert_eq!(h.newer().as_deref(), Some("two"));
        assert_eq!(h.newer().as_deref(), Some("t"));
    }

    #[test]
    fn duplicates_move_to_the_end_and_blanks_are_skipped() {
        let h = history(&["a", "b", "a", "  ", ""]);
        assert_eq!(h.entries(), ["b", "a"]);
    }

    #[test]
    fn keeps_a_bounded_number_of_entries() {
        let mut h = History::default();
        for i in 0..150 {
            h.push(&i.to_string());
        }
        assert_eq!(h.entries().len(), LIMIT);
        assert_eq!(h.entries()[0], "50");
    }
}
