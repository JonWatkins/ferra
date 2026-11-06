//! The bottom-line prompt (`:`, `/`, `?`): its text and cursor, editing,
//! history, and completion state.
//!
//! Completion candidates are worked out by the commands layer, which can see
//! the command registry and the filesystem; this module only applies them.

mod history;

pub use history::History;

/// What the bottom-line prompt is for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// `:` commands.
    #[default]
    Command,
    /// `/` searches forward.
    SearchForward,
    /// `?` searches backward.
    SearchBackward,
}

impl Prompt {
    /// The character shown before the input.
    pub const fn symbol(self) -> char {
        match self {
            Prompt::Command => ':',
            Prompt::SearchForward => '/',
            Prompt::SearchBackward => '?',
        }
    }

    pub const fn is_search(self) -> bool {
        matches!(self, Prompt::SearchForward | Prompt::SearchBackward)
    }
}

/// An in-progress `<Tab>` completion: the candidates for the word that
/// started at `start`, and which one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub start: usize,
    pub candidates: Vec<String>,
    pub index: usize,
}

/// The word being completed: where it starts, its text, and its position on
/// the line (0 for the command name, 1 for the first argument...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionContext {
    pub start: usize,
    pub word: String,
    pub word_index: usize,
    /// The first word on the line (the command), if the word is an argument.
    pub command: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommandLine {
    input: String,
    /// Cursor position in chars.
    cursor: usize,
    prompt: Prompt,
    command_history: History,
    search_history: History,
    completion: Option<Completion>,
}

impl CommandLine {
    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The text before the cursor.
    pub fn before_cursor(&self) -> &str {
        &self.input[..self.byte_index(self.cursor)]
    }

    pub fn prompt(&self) -> Prompt {
        self.prompt
    }

    pub fn completion(&self) -> Option<&Completion> {
        self.completion.as_ref()
    }

    /// Starts a fresh prompt of the given kind.
    pub fn open(&mut self, prompt: Prompt) {
        self.prompt = prompt;
        self.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    /// Empties the input (history is kept).
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.completion = None;
        self.history_mut().reset();
    }

    /// Returns the input, clears the line, and adds it to this prompt's
    /// history.
    pub fn take(&mut self) -> String {
        let input = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.completion = None;
        self.history_mut().push(&input);
        input
    }

    /// Inserts `c` at the cursor.
    pub fn push(&mut self, c: char) {
        let at = self.byte_index(self.cursor);
        self.input.insert(at, c);
        self.cursor += 1;
        self.edited();
    }

    /// Deletes the char before the cursor and returns it.
    pub fn pop(&mut self) -> Option<char> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        let at = self.byte_index(self.cursor);
        let removed = self.input.remove(at);
        self.edited();
        Some(removed)
    }

    /// Deletes the char under the cursor.
    pub fn delete_forward(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index(self.cursor);
            self.input.remove(at);
            self.edited();
        }
    }

    /// `<C-w>`: deletes the word (and any spaces after it) before the cursor.
    pub fn delete_word_backward(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        self.replace(start, self.cursor, "");
        self.edited();
    }

    /// `<C-u>`: deletes everything before the cursor.
    pub fn delete_to_start(&mut self) {
        self.replace(0, self.cursor, "");
        self.edited();
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.completion = None;
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
        self.completion = None;
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
        self.completion = None;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.len();
        self.completion = None;
    }

    /// `<Up>`: the previous history entry starting with what was typed.
    pub fn history_older(&mut self) -> bool {
        let current = self.input.clone();
        match self.history_mut().older(&current) {
            Some(entry) => {
                self.set_input(entry);
                true
            }
            None => false,
        }
    }

    /// `<Down>`: the next history entry, or back to what was typed.
    pub fn history_newer(&mut self) -> bool {
        match self.history_mut().newer() {
            Some(entry) => {
                self.set_input(entry);
                true
            }
            None => false,
        }
    }

    /// The word before the cursor, for working out completion candidates.
    pub fn completion_context(&self) -> CompletionContext {
        let before: Vec<char> = self.before_cursor().chars().collect();
        let start = before
            .iter()
            .rposition(|c| c.is_whitespace())
            .map_or(0, |i| i + 1);
        let word: String = before[start..].iter().collect();
        let earlier: Vec<String> = before[..start]
            .iter()
            .collect::<String>()
            .split_whitespace()
            .map(String::from)
            .collect();
        CompletionContext {
            start,
            word,
            word_index: earlier.len(),
            command: earlier.first().cloned(),
        }
    }

    /// Starts completing the word at `start` with `candidates`, showing the
    /// first. A single candidate is applied and completion ends, so `<Tab>`
    /// again completes further (into a directory, say).
    pub fn start_completion(&mut self, start: usize, candidates: Vec<String>) -> bool {
        let Some(first) = candidates.first().cloned() else {
            return false;
        };
        self.replace(start, self.cursor, &first);
        self.history_mut().reset();
        self.completion = (candidates.len() > 1).then_some(Completion {
            start,
            candidates,
            index: 0,
        });
        true
    }

    /// `<Tab>`/`<S-Tab>` while completing: shows the next or previous
    /// candidate. Returns `false` if not completing.
    pub fn cycle_completion(&mut self, delta: isize) -> bool {
        let Some(mut completion) = self.completion.take() else {
            return false;
        };
        let len = completion.candidates.len() as isize;
        completion.index = (completion.index as isize + delta).rem_euclid(len) as usize;
        let candidate = completion.candidates[completion.index].clone();
        self.replace(completion.start, self.cursor, &candidate);
        self.completion = Some(completion);
        true
    }

    /// Replaces the input, with the cursor at the end.
    fn set_input(&mut self, input: String) {
        self.input = input;
        self.cursor = self.len();
        self.completion = None;
    }

    /// Replaces chars `from..to` with `text`, leaving the cursor after it.
    fn replace(&mut self, from: usize, to: usize, text: &str) {
        let (a, b) = (self.byte_index(from), self.byte_index(to));
        self.input.replace_range(a..b, text);
        self.cursor = from + text.chars().count();
    }

    /// Any edit ends completion and history browsing.
    fn edited(&mut self) {
        self.completion = None;
        self.history_mut().reset();
    }

    fn history_mut(&mut self) -> &mut History {
        if self.prompt.is_search() {
            &mut self.search_history
        } else {
            &mut self.command_history
        }
    }

    fn len(&self) -> usize {
        self.input.chars().count()
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_index)
            .map_or(self.input.len(), |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> CommandLine {
        let mut line = CommandLine::default();
        text.chars().for_each(|c| line.push(c));
        line
    }

    #[test]
    fn push_pop_and_take() {
        let mut line = line("q!");
        assert_eq!(line.pop(), Some('!'));
        assert_eq!(line.take(), "q");
        assert!(line.is_empty());
    }

    #[test]
    fn open_sets_prompt_and_clears_input() {
        let mut line = line("x");
        line.open(Prompt::SearchBackward);
        assert!(line.is_empty());
        assert_eq!(line.prompt().symbol(), '?');
        assert!(line.prompt().is_search());
        assert!(!Prompt::Command.is_search());
    }

    #[test]
    fn editing_at_the_cursor() {
        let mut l = line("wrte");
        l.move_left();
        l.move_left();
        l.push('i');
        assert_eq!(l.input(), "write");
        assert_eq!(l.cursor(), 3);
        l.move_home();
        l.delete_forward();
        assert_eq!(l.input(), "rite");
        l.move_end();
        assert_eq!(l.pop(), Some('e'));
        l.move_home();
        assert_eq!(l.pop(), None, "nothing before the cursor");
        assert_eq!(l.input(), "rit");
    }

    #[test]
    fn multibyte_text() {
        let mut l = line("héé");
        l.move_left();
        l.push('x');
        assert_eq!(l.input(), "héxé");
        assert_eq!(l.before_cursor(), "héx");
    }

    #[test]
    fn delete_word_and_to_start() {
        let mut l = line("e some/file.txt  ");
        l.delete_word_backward();
        assert_eq!(l.input(), "e ");
        let mut l = line("goto_line 10");
        l.move_left();
        l.delete_to_start();
        assert_eq!(l.input(), "0");
        assert_eq!(l.cursor(), 0);
    }

    #[test]
    fn histories_are_separate_per_prompt() {
        let mut l = CommandLine::default();
        l.open(Prompt::Command);
        "w".chars().for_each(|c| l.push(c));
        l.take();
        l.open(Prompt::SearchForward);
        "needle".chars().for_each(|c| l.push(c));
        l.take();

        l.open(Prompt::Command);
        assert!(l.history_older());
        assert_eq!(l.input(), "w");
        l.open(Prompt::SearchForward);
        assert!(l.history_older());
        assert_eq!(l.input(), "needle");
        assert!(l.history_newer());
        assert_eq!(l.input(), "");
    }

    #[test]
    fn completion_context_for_names_and_arguments() {
        let l = line("e src/ma");
        let cx = l.completion_context();
        assert_eq!(
            (cx.start, cx.word.as_str(), cx.word_index),
            (2, "src/ma", 1)
        );
        assert_eq!(cx.command.as_deref(), Some("e"));

        let cx = line("wri").completion_context();
        assert_eq!(
            (cx.start, cx.word.as_str(), cx.word_index, cx.command),
            (0, "wri", 0, None)
        );

        let cx = line("e ").completion_context();
        assert_eq!((cx.start, cx.word.as_str(), cx.word_index), (2, "", 1));
    }

    #[test]
    fn completion_applies_and_cycles() {
        let mut l = line("w");
        assert!(l.start_completion(0, vec!["write".into(), "write_all".into()]));
        assert_eq!(l.input(), "write");
        assert!(l.cycle_completion(1));
        assert_eq!(l.input(), "write_all");
        assert!(l.cycle_completion(1));
        assert_eq!(l.input(), "write", "wraps");
        assert!(l.cycle_completion(-1));
        assert_eq!(l.input(), "write_all");
        l.push(' ');
        assert!(l.completion().is_none(), "typing ends completion");
        assert!(!l.cycle_completion(1));
    }

    #[test]
    fn single_candidate_completes_and_ends() {
        let mut l = line("e sr");
        assert!(l.start_completion(2, vec!["src/".into()]));
        assert_eq!(l.input(), "e src/");
        assert!(l.completion().is_none());
        assert!(!l.start_completion(2, Vec::new()), "no candidates");
    }
}
