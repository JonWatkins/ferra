//! Bracket pair colours and indent guides' editor side: each buffer's
//! line states (how deeply brackets are nested where each line starts),
//! worked out once per edit for the whole file (`core::brackets`), and a
//! buffer's indent width.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{Document, DocumentId, Editor};
use crate::config::EditorOptions;
use crate::core::IndentUnit;
use crate::core::brackets::{Comments, LineState, line_states};

/// A buffer's line states, and for which text (its buffer and edit).
type Cached = ((u64, u64), Rc<Vec<LineState>>);

/// Each buffer's line states.
#[derive(Debug, Default)]
pub struct BracketCache {
    states: RefCell<HashMap<DocumentId, Cached>>,
}

impl Editor {
    /// The language's comments, for skipping brackets in them.
    pub fn comments_of(&self, document: &Document) -> Comments {
        let tokens = self
            .language_of(document)
            .and_then(|language| self.languages.comment_tokens(language));
        match tokens {
            Some(tokens) => match tokens.end {
                Some(end) => Comments {
                    line: None,
                    block: Some((tokens.start, end)),
                },
                None => Comments {
                    line: Some(tokens.start),
                    block: None,
                },
            },
            None => Comments::default(),
        }
    }

    /// Where each line of `document` starts: how deeply brackets are
    /// nested, and whether in a block comment. Worked out again after an
    /// edit.
    pub fn bracket_states(&self, document: &Document) -> Rc<Vec<LineState>> {
        let buffer = document.buffer();
        let key = (buffer.id(), buffer.edit_seq());
        if let Some((at, states)) = self.brackets.states.borrow().get(&document.id())
            && *at == key
        {
            return Rc::clone(states);
        }
        let comments = self.comments_of(document);
        let lines = (0..buffer.line_count()).map(|row| buffer.line(row));
        let states = Rc::new(line_states(lines, &comments));
        self.brackets
            .states
            .borrow_mut()
            .insert(document.id(), (key, Rc::clone(&states)));
        states
    }

    /// How many columns a level of indentation takes in `document`, with
    /// `options` (its window's).
    pub fn indent_width_of(&self, document: &Document, options: &EditorOptions) -> usize {
        let unit = document
            .chosen_indent()
            .or_else(|| document.detected_indent().filter(|_| options.detect_indent));
        match unit {
            Some(IndentUnit::Spaces(n)) => n.max(1),
            Some(IndentUnit::Tabs) => options.tab_width.max(1),
            None if options.expand_tab => options.tab_width.max(1),
            None => options.tab_width.max(1),
        }
    }
}
