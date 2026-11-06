//! Turning tree-sitter's highlight events into styled spans per line.

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// The highlight names the editor understands, which are also the theme
/// scopes they're styled with. Tree-sitter maps each query capture to the
/// most specific of these that matches it (`function.method.call` →
/// `function.method`), and the theme falls back to the parent scope
/// (`function.method` → `function`), so themes can be as coarse or as
/// detailed as they like.
pub const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "module",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "text.emphasis",
    "text.literal",
    "text.reference",
    "text.strong",
    "text.title",
    "text.uri",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

/// A highlighted run of characters on one line: columns `start..end` (in
/// chars) styled as `HIGHLIGHT_NAMES[highlight]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub highlight: usize,
}

impl Span {
    pub fn name(&self) -> &'static str {
        HIGHLIGHT_NAMES[self.highlight]
    }
}

/// The highlighted spans of a whole document, line by line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlights {
    lines: Vec<Vec<Span>>,
}

impl Highlights {
    /// Spans on `row`, in order. Empty for rows past the end.
    pub fn line(&self, row: usize) -> &[Span] {
        self.lines.get(row).map_or(&[], Vec::as_slice)
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

/// Highlights `text` (lines separated by `\n`) with `config`. Text in
/// another language inside it (a Markdown code block, `<script>` in HTML)
/// is highlighted with the configuration `inject` gives for that language's
/// name, if any.
pub fn highlight<'a>(
    highlighter: &'a mut Highlighter,
    config: &'a HighlightConfiguration,
    text: &'a str,
    inject: impl Fn(&str) -> Option<&'a HighlightConfiguration> + 'a,
) -> Result<Highlights, String> {
    let events = highlighter
        .highlight(config, text.as_bytes(), None, None, inject)
        .map_err(|e| format!("highlighting failed: {e}"))?;

    let mut lines: Vec<Vec<Span>> = vec![Vec::new()];
    let mut stack: Vec<usize> = Vec::new();
    let mut col = 0;

    for event in events {
        match event.map_err(|e| format!("highlighting failed: {e}"))? {
            HighlightEvent::HighlightStart(h) => stack.push(h.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let current = stack.last().copied();
                for c in text[start..end].chars() {
                    if c == '\n' {
                        lines.push(Vec::new());
                        col = 0;
                        continue;
                    }
                    if let Some(highlight) = current {
                        let line = lines.last_mut().expect("at least one line");
                        match line.last_mut() {
                            Some(span) if span.end == col && span.highlight == highlight => {
                                span.end += 1;
                            }
                            _ => line.push(Span {
                                start: col,
                                end: col + 1,
                                highlight,
                            }),
                        }
                    }
                    col += 1;
                }
            }
        }
    }
    Ok(Highlights { lines })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::languages::LANGUAGES;

    fn rust_config() -> HighlightConfiguration {
        let rust = &LANGUAGES[0];
        let mut config = HighlightConfiguration::new(
            (rust.grammar)(),
            rust.name,
            &rust.highlight_query(),
            rust.injections,
            rust.locals,
        )
        .unwrap();
        config.configure(HIGHLIGHT_NAMES);
        config
    }

    /// `(text, highlight name)` for each span on `row`.
    fn spans(text: &str, row: usize) -> Vec<(String, &'static str)> {
        let highlights =
            highlight(&mut Highlighter::new(), &rust_config(), text, |_| None).unwrap();
        let line: Vec<char> = text.split('\n').nth(row).unwrap().chars().collect();
        highlights
            .line(row)
            .iter()
            .map(|s| (line[s.start..s.end].iter().collect(), s.name()))
            .collect()
    }

    #[test]
    fn highlights_rust() {
        let found = spans("fn main() {\n    let x = \"hi\"; // note\n}", 1);
        assert!(found.contains(&("let".into(), "keyword")), "{found:?}");
        assert!(found.contains(&("\"hi\"".into(), "string")), "{found:?}");
        assert!(found.contains(&("// note".into(), "comment")), "{found:?}");
    }

    #[test]
    fn function_names_and_one_line_per_row() {
        let text = "fn main() {}\n\nstruct Foo;";
        let highlights =
            highlight(&mut Highlighter::new(), &rust_config(), text, |_| None).unwrap();
        assert_eq!(highlights.line_count(), 3);
        assert!(highlights.line(1).is_empty(), "blank line");
        assert!(spans(text, 0).contains(&("main".into(), "function")));
        assert!(spans(text, 2).contains(&("Foo".into(), "type")));
    }

    #[test]
    fn columns_are_chars() {
        let found = spans("let s = \"é\"; let t = 1;", 0);
        assert!(found.contains(&("\"é\"".into(), "string")), "{found:?}");
        assert!(
            found.contains(&("1".into(), "constant.builtin"))
                || found.contains(&("1".into(), "number")),
            "{found:?}"
        );
    }

    #[test]
    fn rows_past_the_end_are_empty() {
        assert!(Highlights::default().line(10).is_empty());
    }
}
