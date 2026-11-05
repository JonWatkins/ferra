/// The text typed after `:` in command mode.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommandLine {
    input: String,
}

impl CommandLine {
    pub fn input(&self) -> &str {
        &self.input
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
}
