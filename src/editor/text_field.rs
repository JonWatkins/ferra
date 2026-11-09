//! A one-line text field for the interface's boxes (the find box's find
//! and replace fields): text, a cursor, and the usual editing keys.
//!
//! Text put in by the editor (the word under the cursor when the find box
//! opens) starts *selected*, as in VS Code: typing replaces it, `<BS>`
//! clears it, and moving the cursor keeps it.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextField {
    text: String,
    /// In chars.
    cursor: usize,
    selected: bool,
}

impl TextField {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor, in chars.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether all the text is selected (typing replaces it).
    pub fn is_selected(&self) -> bool {
        self.selected && !self.text.is_empty()
    }

    /// Replaces the text, selected, with the cursor at its end.
    pub fn set_selected(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.len();
        self.selected = true;
    }

    /// Selects all the text.
    pub fn select_all(&mut self) {
        self.cursor = self.len();
        self.selected = true;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, col: usize) -> usize {
        self.text
            .char_indices()
            .nth(col)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// Takes the selected text away, if it's selected. Returns whether it was.
    fn clear_selected(&mut self) -> bool {
        let was = self.is_selected();
        if was {
            self.text.clear();
            self.cursor = 0;
        }
        self.selected = false;
        was
    }

    pub fn push(&mut self, c: char) {
        self.clear_selected();
        let at = self.byte(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.clear_selected() || self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte(self.cursor);
        self.text.remove(at);
    }

    pub fn delete(&mut self) {
        if self.clear_selected() || self.cursor >= self.len() {
            return;
        }
        let at = self.byte(self.cursor);
        self.text.remove(at);
    }

    /// `<C-w>`: the word (and the spaces after it) before the cursor.
    pub fn delete_word(&mut self) {
        if self.clear_selected() {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let word = |c: char| c.is_alphanumeric() || c == '_';
        if start > 0 && word(chars[start - 1]) {
            while start > 0 && word(chars[start - 1]) {
                start -= 1;
            }
        } else {
            start = start.saturating_sub(1);
        }
        let (from, to) = (self.byte(start), self.byte(self.cursor));
        self.text.replace_range(from..to, "");
        self.cursor = start;
    }

    /// `<C-u>`: everything before the cursor.
    pub fn delete_to_start(&mut self) {
        if self.clear_selected() {
            return;
        }
        let to = self.byte(self.cursor);
        self.text.replace_range(..to, "");
        self.cursor = 0;
    }

    pub fn left(&mut self) {
        if !std::mem::take(&mut self.selected) {
            self.cursor = self.cursor.saturating_sub(1);
        } else {
            self.cursor = 0;
        }
    }

    pub fn right(&mut self) {
        self.selected = false;
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.selected = false;
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.selected = false;
        self.cursor = self.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> TextField {
        let mut field = TextField::default();
        text.chars().for_each(|c| field.push(c));
        field
    }

    #[test]
    fn edits_at_the_cursor() {
        let mut field = typed("hello world");
        field.left();
        field.left();
        field.push('X');
        assert_eq!(field.text(), "hello worXld");
        field.home();
        field.delete();
        field.end();
        field.backspace();
        assert_eq!(field.text(), "ello worXl");
        field.delete_word();
        assert_eq!(field.text(), "ello ");
        field.delete_to_start();
        assert_eq!((field.text(), field.cursor()), ("", 0));
    }

    #[test]
    fn selected_text_is_replaced_by_typing_and_kept_by_moving() {
        let mut field = TextField::default();
        field.set_selected("needle");
        assert!(field.is_selected());
        field.push('x');
        assert_eq!(field.text(), "x");

        field.set_selected("needle");
        field.backspace();
        assert_eq!(field.text(), "", "<BS> clears it");

        field.set_selected("needle");
        field.left();
        assert_eq!(
            (field.text(), field.cursor()),
            ("needle", 0),
            "to its start"
        );
        field.end();
        field.push('s');
        assert_eq!(field.text(), "needles");
    }

    #[test]
    fn deleting_a_word_stops_at_punctuation() {
        let mut field = typed("foo.bar_baz");
        field.delete_word();
        assert_eq!(field.text(), "foo.");
        field.delete_word();
        assert_eq!(field.text(), "foo");
    }
}
