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

/// The text typed at the bottom-line prompt (in command mode).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommandLine {
    input: String,
    prompt: Prompt,
}

impl CommandLine {
    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn prompt(&self) -> Prompt {
        self.prompt
    }

    /// Starts a fresh prompt of the given kind.
    pub fn open(&mut self, prompt: Prompt) {
        self.input.clear();
        self.prompt = prompt;
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn push(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn pop(&mut self) -> Option<char> {
        self.input.pop()
    }

    pub fn clear(&mut self) {
        self.input.clear();
    }

    /// Returns the current input and clears the line.
    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_and_take() {
        let mut line = CommandLine::default();
        line.push('q');
        line.push('!');
        assert_eq!(line.pop(), Some('!'));
        assert_eq!(line.take(), "q");
        assert!(line.is_empty());
    }

    #[test]
    fn open_sets_prompt_and_clears_input() {
        let mut line = CommandLine::default();
        line.push('x');
        line.open(Prompt::SearchBackward);
        assert!(line.is_empty());
        assert_eq!(line.prompt().symbol(), '?');
        assert!(line.prompt().is_search());
        assert!(!Prompt::Command.is_search());
    }
}
