//! Language-aware indentation from the syntax tree.
//!
//! Each language with rules has a small tree-sitter query of its own:
//!
//! - `@indent` marks a node whose lines after its first are one level in
//!   (a block, an argument list, an HTML element).
//! - `@outdent` marks a token that, starting a line, puts that line back at
//!   the level of the node it closes (`}`, `)`, `fi`, `else`, `</div>`).
//!
//! A line's indentation is *relative*: the indentation of the line where
//! its innermost enclosing `@indent` node starts, plus one level (or plus
//! nothing, for an `@outdent` line). So a file's own style (two spaces,
//! tabs) is kept, and nodes opened on the same line (`foo(bar(`) add one
//! level, not two. An `@outdent` token only counts when it belongs to
//! (is a child of) that innermost node, so the `}` of a node without rules
//! (Go's `switch`) doesn't cancel the block around it.
//!
//! While typing, brackets are often unclosed (`fn main() {` and nothing
//! after). Tree-sitter's error recovery then either makes up the missing
//! token, or leaves the bracket loose inside an `ERROR` node; either way,
//! it's taken to be open to the end of the text.
//!
//! Lines that start inside a multi-line string or comment (or an HTML
//! `<script>`'s raw text) are left alone. Languages where indentation is
//! meaning (Python, YAML) or that don't nest (Markdown) have no rules.

use std::collections::{HashMap, HashSet};
use std::ops::{Range, RangeInclusive};
use std::sync::OnceLock;

use tree_sitter::{Node, Point, Query, QueryCursor, StreamingIterator, Tree};

use super::languages::LANGUAGES;
use crate::core::Buffer;
use crate::core::columns::display_width;

const RUST: &str = r#"
[
  (block) (declaration_list) (field_declaration_list) (enum_variant_list)
  (match_block) (arguments) (parameters) (tuple_expression) (array_expression)
  (field_initializer_list) (use_list) (token_tree) (tuple_type)
  (ordered_field_declaration_list) (type_arguments) (type_parameters)
  (closure_parameters) (tuple_pattern) (slice_pattern) (field_pattern)
  (struct_pattern) (where_clause)
] @indent
["}" ")" "]" ">"] @outdent
"#;

const JSON: &str = r#"
[(object) (array)] @indent
["}" "]"] @outdent
"#;

const TOML: &str = r#"
[(array) (inline_table)] @indent
["}" "]"] @outdent
"#;

/// JavaScript, and the base of TypeScript's.
const JAVASCRIPT: &str = r#"
[
  (statement_block) (class_body) (object) (array) (arguments)
  (formal_parameters) (switch_body) (switch_case) (switch_default)
  (object_pattern) (array_pattern) (named_imports) (export_clause)
  (parenthesized_expression) (template_substitution)
] @indent
["}" ")" "]"] @outdent
"#;

/// JSX, in JavaScript and TSX.
const JSX: &str = r#"
[(jsx_element) (jsx_opening_element) (jsx_self_closing_element)] @indent
[(jsx_closing_element) ">" "/>"] @outdent
"#;

const TYPESCRIPT_EXTRA: &str = r#"
[(interface_body) (enum_body) (object_type) (type_arguments) (type_parameters)] @indent
"#;

const GO: &str = r#"
[
  (block) (literal_value) (argument_list) (parameter_list)
  (field_declaration_list) (interface_type) (import_spec_list)
  (const_declaration) (var_declaration) (type_declaration)
  (expression_case) (default_case) (type_case) (communication_case)
] @indent
["}" ")" "]"] @outdent
"#;

/// C, and the base of C++'s.
const C: &str = r#"
[
  (compound_statement) (field_declaration_list) (enumerator_list)
  (initializer_list) (argument_list) (parameter_list) (case_statement)
  (parenthesized_expression)
] @indent
["}" ")" "]"] @outdent
"#;

const CPP_EXTRA: &str = r#"
[(template_parameter_list) (template_argument_list) (lambda_expression)] @indent
"#;

const BASH: &str = r#"
[
  (compound_statement) (do_group) (if_statement) (case_statement)
  (case_item) (subshell) (array)
] @indent
["}" ")" "fi" "done" "esac" "else" "elif" "then" (elif_clause) (else_clause)] @outdent
"#;

const HTML: &str = r#"
[(element) (start_tag) (self_closing_tag)] @indent
[(end_tag) ">" "/>"] @outdent
"#;

const CSS: &str = r#"
[(block) (keyframe_block_list) (arguments)] @indent
["}" ")"] @outdent
"#;

/// Words that, typed as the first on a line, re-indent it by the rules
/// (`fi` goes back to its `if`), as Vim's `indentkeys` do. A line whose
/// first word only started as one (`fi` then `l`) is re-indented again.
pub fn keywords(language: &str) -> &'static [&'static str] {
    match language {
        "bash" => &["fi", "done", "esac", "else", "elif", "then", "do"],
        _ => &[],
    }
}

/// Whether typing `>` to finish a closing tag (`</div>`) first on a line
/// re-indents it by the rules.
pub fn closes_tags(language: &str) -> bool {
    matches!(language, "html" | "javascript" | "tsx")
}

/// The rules for a language, by name: its query sources, joined.
fn rules_source(language: &str) -> Option<String> {
    Some(match language {
        "rust" => RUST.into(),
        "json" => JSON.into(),
        "toml" => TOML.into(),
        "javascript" => format!("{JAVASCRIPT}\n{JSX}"),
        "typescript" => format!("{JAVASCRIPT}\n{TYPESCRIPT_EXTRA}"),
        "tsx" => format!("{JAVASCRIPT}\n{TYPESCRIPT_EXTRA}\n{JSX}"),
        "go" => GO.into(),
        "c" => C.into(),
        "cpp" => format!("{C}\n{CPP_EXTRA}"),
        "bash" => BASH.into(),
        "html" => HTML.into(),
        "css" => CSS.into(),
        _ => return None,
    })
}

/// A language's compiled rules (built on first use).
fn rules(language: &str) -> Option<&'static Query> {
    static QUERIES: OnceLock<Vec<OnceLock<Option<Query>>>> = OnceLock::new();
    let index = LANGUAGES.iter().position(|l| l.name == language)?;
    let cells = QUERIES.get_or_init(|| LANGUAGES.iter().map(|_| OnceLock::new()).collect());
    cells[index]
        .get_or_init(|| {
            let source = rules_source(language)?;
            Query::new(&(LANGUAGES[index].grammar)(), &source).ok()
        })
        .as_ref()
}

/// Whether `language` has indentation rules.
pub fn has_rules(language: &str) -> bool {
    rules(language).is_some()
}

/// How indentation is measured and written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentStyle {
    pub tab_width: usize,
    /// Display columns in one level.
    pub unit: usize,
}

/// The indentation (in display columns) each line of `rows` should have,
/// computed top to bottom so a line below follows the new indentation of
/// one above. `None` for a line to leave alone (inside a multi-line string
/// or comment). Blank lines get the indentation text typed there would
/// have. `None` overall if `language` has no rules or parsing failed.
pub fn compute(
    language: &str,
    tree: &Tree,
    buffer: &Buffer,
    rows: RangeInclusive<usize>,
    style: IndentStyle,
) -> Option<Vec<Option<usize>>> {
    let query = rules(language)?;
    // Only nodes touching the rows matter: the ones they're inside, and
    // closers on them. One byte before, so an empty last line still
    // touches what's open at the end.
    let start = buffer.row_to_byte(*rows.start());
    let end = buffer.row_to_byte(rows.end().saturating_add(1));
    let bytes = start.saturating_sub(1)..end.max(start + 1);
    let mut structure = Structure::collect(query, tree, buffer, bytes.clone());
    if tree.root_node().has_error() {
        // A node left open (its closer made up by error recovery) is open
        // to the end of the text, but if it ends before the rows, the query
        // above didn't see it: look around each made-up closer before them.
        let mut missing = Vec::new();
        missing_before(tree.root_node(), start, &mut missing);
        for at in missing {
            let around = Structure::collect(query, tree, buffer, at.saturating_sub(1)..at + 1);
            let open = around.indents.into_iter().filter(|n| n.unterminated);
            structure.indents.extend(open);
        }
        loose_brackets(tree.root_node(), &mut structure.indents);
    }
    let innermost = structure.innermost_by_row(&rows);

    // Widths decided so far; other lines keep the width they have.
    let mut decided: HashMap<usize, usize> = HashMap::new();
    let width_of = |decided: &HashMap<usize, usize>, row: usize| {
        decided
            .get(&row)
            .copied()
            .unwrap_or_else(|| display_width(leading_blanks(&buffer.line(row)), style.tab_width))
    };
    let mut result = Vec::new();
    for row in rows.clone() {
        if row >= buffer.line_count() {
            result.push(None);
            continue;
        }
        let line = buffer.line(row);
        let point = Point::new(row, leading_blanks(&line).len());
        if inside_text(tree, point) {
            result.push(None);
            continue;
        }
        let width = match innermost[row - rows.start()] {
            None => 0,
            Some(node) if structure.outdents_at(point, node) => width_of(&decided, node.start_row),
            Some(node) => width_of(&decided, node.start_row) + style.unit,
        };
        decided.insert(row, width);
        result.push(Some(width));
    }
    Some(result)
}

/// The `@indent` nodes and `@outdent` tokens of a tree.
struct Structure {
    indents: Vec<IndentNode>,
    /// Where each starts (row, column), and its parent node's id.
    outdents: HashSet<(usize, usize, usize)>,
}

#[derive(Debug, Clone, Copy)]
struct IndentNode {
    /// The tree node's id; none for a loose bracket in an `ERROR`.
    id: Option<usize>,
    start_row: usize,
    start_byte: usize,
    end: Point,
    /// Its closing token is missing, so it runs to the end of the text.
    unterminated: bool,
}

impl Structure {
    /// The nodes and tokens of `tree` touching `bytes`.
    fn collect(query: &Query, tree: &Tree, buffer: &Buffer, bytes: Range<usize>) -> Structure {
        let names = query.capture_names();
        let mut indents = Vec::new();
        let mut outdents = HashSet::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes);
        let text = |node: Node| buffer.byte_chunks(node.byte_range());
        let mut captures = cursor.captures(query, tree.root_node(), text);
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let node = capture.node;
            match names[capture.index as usize] {
                "indent" => indents.push(IndentNode {
                    id: Some(node.id()),
                    start_row: node.start_position().row,
                    start_byte: node.start_byte(),
                    end: node.end_position(),
                    unterminated: ends_missing(node),
                }),
                // A closer made up by error recovery (a `fi` the user
                // hasn't typed) isn't on the line it's placed at.
                "outdent" if node.is_missing() => {}
                "outdent" => {
                    if let Some(parent) = node.parent() {
                        let start = node.start_position();
                        outdents.insert((start.row, start.column, parent.id()));
                    }
                }
                _ => {}
            }
        }
        Structure { indents, outdents }
    }

    /// For each row of `rows`, the innermost `@indent` node that started
    /// on an earlier line and is still open on it. Nodes are laid over the
    /// rows they cover in the order they start, so a nested node (starting
    /// later) wins over the one around it.
    fn innermost_by_row(&self, rows: &RangeInclusive<usize>) -> Vec<Option<IndentNode>> {
        let (first, last) = (*rows.start(), *rows.end());
        let mut result = vec![None; last + 1 - first];
        let mut nodes = self.indents.clone();
        nodes.sort_by_key(|n| n.start_byte);
        for node in nodes {
            let covers_last = if node.unterminated {
                usize::MAX
            } else if node.end.column > 0 {
                node.end.row
            } else {
                node.end.row.saturating_sub(1)
            };
            let from = (node.start_row + 1).max(first);
            let to = covers_last.min(last);
            for row in from..=to {
                result[row - first] = Some(node);
            }
        }
        result
    }

    /// Whether an `@outdent` token belonging to `node` starts at `point`.
    fn outdents_at(&self, point: Point, node: IndentNode) -> bool {
        node.id
            .is_some_and(|id| self.outdents.contains(&(point.row, point.column, id)))
    }
}

/// Adds the brackets (and HTML start tags, and Bash's `then` and `do`) left
/// open inside `ERROR` nodes under `node`, as indent nodes open to the end
/// of the text.
fn loose_brackets(node: Node, indents: &mut Vec<IndentNode>) {
    if node.is_error() {
        let mut open: Vec<Node> = Vec::new();
        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else {
                continue;
            };
            // Bash's blocks open with keywords: `then` and `do` (on the
            // `if` or `for` line, usually), closed by `fi` and `done`.
            match child.kind() {
                "{" | "(" | "[" | "start_tag" | "then" | "do" => open.push(child),
                "}" | ")" | "]" | "end_tag" | "fi" | "done" => {
                    open.pop();
                }
                "else" | "elif" => {
                    open.pop();
                    open.push(child);
                }
                _ => {}
            }
        }
        indents.extend(open.into_iter().map(|bracket| IndentNode {
            id: None,
            start_row: bracket.start_position().row,
            start_byte: bracket.start_byte(),
            end: bracket.end_position(),
            unterminated: true,
        }));
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.has_error()
        {
            loose_brackets(child, indents);
        }
    }
}

/// The positions of the tokens made up by error recovery (`MISSING`)
/// under `node` that start before byte `before`. Only branches holding an
/// error are walked.
fn missing_before(node: Node, before: usize, found: &mut Vec<usize>) {
    if node.start_byte() >= before {
        return;
    }
    if node.is_missing() {
        found.push(node.start_byte());
        return;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.has_error()
        {
            missing_before(child, before, found);
        }
    }
}

/// Whether `node`'s last token was made up by error recovery.
fn ends_missing(node: Node) -> bool {
    let mut node = node;
    while let Some(last) = node.child(node.child_count().saturating_sub(1)) {
        if last.is_missing() {
            return true;
        }
        node = last;
    }
    false
}

/// Whether `point` (a line's first non-blank) is inside a string, comment
/// or raw text that began on an earlier line.
fn inside_text(tree: &Tree, point: Point) -> bool {
    let mut node = tree.root_node().descendant_for_point_range(point, point);
    while let Some(n) = node {
        let kind = n.kind();
        let text_like = ["string", "comment", "raw_text", "heredoc"]
            .iter()
            .any(|k| kind.contains(k));
        if text_like && n.start_position().row < point.row {
            return true;
        }
        node = n.parent();
    }
    false
}

fn leading_blanks(line: &str) -> &str {
    let rest = line.trim_start_matches([' ', '\t']);
    &line[..line.len() - rest.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: IndentStyle = IndentStyle {
        tab_width: 4,
        unit: 4,
    };

    /// [`compute`] on `text`, freshly parsed.
    fn compute(
        language: &str,
        text: &str,
        rows: RangeInclusive<usize>,
        style: IndentStyle,
    ) -> Option<Vec<Option<usize>>> {
        let buffer = Buffer::from_text(text);
        let tree = super::super::trees::parse_fresh(language, &buffer)?;
        super::compute(language, &tree, &buffer, rows, style)
    }

    /// `text` re-indented with spaces, blank lines left empty.
    fn reindent(language: &str, text: &str) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        let widths = compute(language, text, 0..=lines.len() - 1, STYLE).unwrap();
        lines
            .iter()
            .zip(widths)
            .map(|(line, width)| match width {
                None => line.to_string(),
                Some(_) if line.trim().is_empty() => String::new(),
                Some(width) => format!("{}{}", " ".repeat(width), line.trim_start()),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Checks that re-indenting `expected` with its indentation stripped
    /// gives it back.
    fn check(language: &str, expected: &str) {
        let flat: String = expected
            .split('\n')
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(reindent(language, &flat), expected, "{language}");
    }

    #[test]
    fn rust_blocks_calls_and_closing_brackets() {
        check(
            "rust",
            "impl A {
    fn f(&self) {
        let x = foo(
            a,
            b,
        );
        if x {
            y();
        } else {
            z();
        }
        match x {
            1 => {}
            _ => (),
        }
    }
}",
        );
    }

    #[test]
    fn nodes_opened_on_one_line_add_one_level() {
        check("rust", "fn f() {\n    foo(bar(\n        1,\n    ));\n}");
    }

    #[test]
    fn strings_and_comments_spanning_lines_are_left_alone() {
        let text = "fn f() {\n/* a\n  b */\nlet s = \"x\n y\";\n}";
        assert_eq!(
            reindent("rust", text),
            "fn f() {\n    /* a\n  b */\n    let s = \"x\n y\";\n}"
        );
    }

    #[test]
    fn relative_to_the_lines_own_style() {
        // Two-space indentation on the opening line is kept, and a level
        // added to it.
        let text = "  fn f() {\nx();\n  }";
        let widths = compute("rust", text, 1..=2, STYLE).unwrap();
        assert_eq!(widths, [Some(6), Some(2)]);
    }

    #[test]
    fn one_row_at_a_time_matches_the_whole_text() {
        let samples = [
            (
                "rust",
                "fn main() {\n    let x = foo(\n        1,\n    );\n    if x {\n        y();\n    }\n}\n",
            ),
            ("rust", "impl A {\n    fn f() {\n        g(\n"),
            (
                "javascript",
                "function f() {\n    return [\n        1,\n    ];\n}\nif (x) {\n",
            ),
            (
                "go",
                "func f() {\n    switch x {\n    case 1:\n        y()\n    }\n}\n",
            ),
            (
                "bash",
                "if a; then\n    b\nelse\n    c\nfi\nfor i in x; do\n    a\n",
            ),
            (
                "html",
                "<ul>\n    <li>\n        a\n    </li>\n</ul>\n<div>\n",
            ),
        ];
        for (language, text) in samples {
            let rows = text.split('\n').count() - 1;
            let whole = compute(language, text, 0..=rows, STYLE).unwrap();
            for (row, width) in whole.iter().enumerate() {
                let one = compute(language, text, row..=row, STYLE).unwrap();
                assert_eq!(one[0], *width, "{language} row {row} of {text:?}");
            }
        }
    }

    #[test]
    fn unclosed_brackets_while_typing() {
        for (language, text) in [
            ("rust", "fn main() {\n"),
            ("rust", "fn main() {\n\nfn other() {}"),
            ("rust", "fn main() {\n    let x = 1;\n"),
            ("javascript", "if (x) {\n"),
            ("c", "int main() {\n"),
            ("go", "func main() {\n"),
            ("bash", "if x; then\n"),
            // The made-up `fi` sits on the new line; it isn't typed there.
            ("bash", "if x; then\n    file=1\n"),
            ("bash", "for i in x; do\n    a\n"),
            // Unparseable, with `then` left loose in an error.
            ("bash", "if a; then\n    b\n\nx"),
            ("html", "<div>\n"),
            ("json", "{\n"),
        ] {
            let row = text.split('\n').count() - 1;
            let row = if text.ends_with("{}") { 1 } else { row };
            let widths = compute(language, text, row..=row, STYLE).unwrap();
            assert_eq!(widths, [Some(4)], "{language}: {text:?}");
        }
    }

    #[test]
    fn javascript_switch_and_jsx() {
        check(
            "javascript",
            "function f() {
    switch (x) {
        case 1:
            foo();
            break;
        default:
            bar();
    }
    return <div>
        <span/>
    </div>;
}",
        );
    }

    #[test]
    fn go_keeps_cases_at_the_switchs_level() {
        check(
            "go",
            "func main() {
    switch x {
    case 1:
        foo()
    }
    const (
        a = 1
    )
}",
        );
    }

    #[test]
    fn c_and_bash_and_html_and_css() {
        check(
            "c",
            "int main() {
    struct s v = {
        1,
    };
}",
        );
        check(
            "bash",
            "if x; then
    y
elif z; then
    w
else
    v
fi
for i in 1; do
    echo
done",
        );
        check(
            "html",
            "<div>
    <p>
        hi
    </p>
    <img
        src=\"x\"
    />
</div>",
        );
        check("css", "@media x {\n    b {\n        c: d;\n    }\n}");
        check("json", "{\n    \"a\": [\n        1\n    ]\n}");
        check(
            "typescript",
            "interface A {\n    x: number;\n}\nenum B {\n    C,\n}",
        );
    }

    #[test]
    fn every_languages_rules_compile() {
        for def in LANGUAGES {
            if let Some(source) = rules_source(def.name) {
                Query::new(&(def.grammar)(), &source)
                    .unwrap_or_else(|e| panic!("{}: {e}", def.name));
            }
        }
        assert!(!has_rules("python"));
        assert!(!has_rules("nonsense"));
    }
}
