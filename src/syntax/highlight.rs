//! Highlighting a range of rows from a syntax tree.
//!
//! This follows `tree-sitter-highlight`'s rules, but works from a tree the
//! engine keeps up to date as the text changes, and looks only at the rows
//! asked for:
//!
//! - The locals and highlights queries run as one query, so their captures
//!   come in document order. A reference to a local definition (a
//!   parameter, say) takes the definition's highlight, and patterns marked
//!   `(#is-not? local)` don't apply to locals.
//! - When several highlight patterns match the same node, the last wins.
//! - Highlights start in document order and each shows over those started
//!   before it, so a node's shows over its parent's, unless the parent's
//!   pattern found it later (TOML's `(pair (bare_key)) @property` colours
//!   its key, though `(bare_key) @type` matches first). An injected
//!   language's shows over its host's for exactly the same range.
//! - Injections (a Markdown code block's language, `<script>` in HTML) are
//!   parsed from the parts of the text they cover, as their own layer.

use std::ops::Range;

use tree_sitter::{
    Language, Node, Parser, Point, Query, QueryCursor, QueryMatch, StreamingIterator, Tree,
};

use crate::core::Buffer;

/// The highlight names the editor understands, which are also the theme
/// scopes they're styled with. Each query capture maps to the most specific
/// of these whose parts it has (`function.method.call` → `function.method`),
/// and the theme falls back to the parent scope (`function.method` →
/// `function`), so themes can be as coarse or as detailed as they like.
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

/// The highlighted spans of some of a document's rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlights {
    first: usize,
    lines: Vec<Vec<Span>>,
}

impl Highlights {
    /// Spans on `row`, in order. Empty for rows not covered.
    pub fn line(&self, row: usize) -> &[Span] {
        row.checked_sub(self.first)
            .and_then(|i| self.lines.get(i))
            .map_or(&[], Vec::as_slice)
    }

    /// The rows these highlights are for.
    pub fn rows(&self) -> Range<usize> {
        self.first..self.first + self.lines.len()
    }
}

/// A language's queries, compiled.
pub struct LanguageConfig {
    pub name: &'static str,
    pub language: Language,
    /// The locals query's patterns, then the highlights query's.
    query: Query,
    /// Patterns below this index are from the locals query.
    locals_patterns: usize,
    injections: Option<Query>,
    /// Each capture's index in `HIGHLIGHT_NAMES`, if it has one.
    highlight_indices: Vec<Option<usize>>,
    /// Patterns that don't apply to local variables (`#is-not? local`).
    non_local: Vec<bool>,
    scope_capture: Option<u32>,
    definition_capture: Option<u32>,
    definition_value_capture: Option<u32>,
    reference_capture: Option<u32>,
    injection_content_capture: Option<u32>,
    injection_language_capture: Option<u32>,
}

impl LanguageConfig {
    pub fn new(
        name: &'static str,
        language: Language,
        highlights: &str,
        injections: &str,
        locals: &str,
    ) -> Result<Self, String> {
        let source = format!("{locals}\n{highlights}");
        let query = Query::new(&language, &source)
            .map_err(|e| format!("{name}: invalid highlight query: {e}"))?;
        let locals_end = locals.len() + 1;
        let locals_patterns = (0..query.pattern_count())
            .take_while(|&i| query.start_byte_for_pattern(i) < locals_end)
            .count();
        let injections = if injections.trim().is_empty() {
            None
        } else {
            Some(
                Query::new(&language, injections)
                    .map_err(|e| format!("{name}: invalid injection query: {e}"))?,
            )
        };
        let non_local = (0..query.pattern_count())
            .map(|i| {
                query
                    .property_predicates(i)
                    .iter()
                    .any(|(property, is)| !is && &*property.key == "local")
            })
            .collect();
        let capture = |query: &Query, wanted: &str| {
            query
                .capture_names()
                .iter()
                .position(|n| *n == wanted)
                .map(|i| i as u32)
        };
        let injection_capture = |wanted: &str| injections.as_ref().and_then(|q| capture(q, wanted));
        Ok(Self {
            name,
            highlight_indices: query.capture_names().iter().map(|n| recognise(n)).collect(),
            non_local,
            scope_capture: capture(&query, "local.scope"),
            definition_capture: capture(&query, "local.definition"),
            definition_value_capture: capture(&query, "local.definition-value"),
            reference_capture: capture(&query, "local.reference"),
            injection_content_capture: injection_capture("injection.content"),
            injection_language_capture: injection_capture("injection.language"),
            injections,
            locals_patterns,
            query,
            language,
        })
    }

    fn has_locals(&self) -> bool {
        self.locals_patterns > 0
    }
}

/// The entry in `HIGHLIGHT_NAMES` with the most parts that are all in the
/// capture `name`'s parts.
fn recognise(name: &str) -> Option<usize> {
    let parts: Vec<&str> = name.split('.').collect();
    let mut best = None;
    let mut best_len = 0;
    for (i, known) in HIGHLIGHT_NAMES.iter().enumerate() {
        let len = known.split('.').count();
        if len > best_len && known.split('.').all(|part| parts.contains(&part)) {
            best = Some(i);
            best_len = len;
        }
    }
    best
}

/// Looks up the configuration for an injected language's name.
pub type Inject<'a> = dyn Fn(&str) -> Option<&'a LanguageConfig> + 'a;

/// How deep injections nest (Markdown → HTML → JavaScript is 2).
const MAX_DEPTH: usize = 4;

/// Highlights `rows` of `buffer`, whose syntax tree in `config`'s language
/// is `tree`. `parser` parses injected languages.
pub fn highlight_rows(
    buffer: &Buffer,
    tree: &Tree,
    config: &LanguageConfig,
    rows: Range<usize>,
    inject: &Inject<'_>,
    parser: &mut Parser,
) -> Highlights {
    let rows = rows.start.min(buffer.line_count())..rows.end.min(buffer.line_count());
    let bytes = buffer.row_to_byte(rows.start)..buffer.row_to_byte(rows.end);
    let mut layers = Layers {
        buffer,
        bytes: bytes.clone(),
        inject,
        parser,
        spans: Vec::new(),
    };
    let everything = vec![tree_sitter::Range {
        start_byte: 0,
        end_byte: usize::MAX,
        start_point: Point::new(0, 0),
        end_point: Point::new(usize::MAX, usize::MAX),
    }];
    layers.layer(tree, config, None, 0, &everything);
    let paint = layers.paint();
    Highlights {
        first: rows.start,
        lines: spans_by_line(buffer, rows, bytes.start, &paint),
    }
}

/// A highlighted byte range from one layer.
struct Painted {
    bytes: Range<usize>,
    depth: usize,
    highlight: usize,
}

struct Layers<'a, 'b> {
    buffer: &'a Buffer,
    /// The bytes being highlighted.
    bytes: Range<usize>,
    inject: &'a Inject<'b>,
    parser: &'a mut Parser,
    /// In the order found.
    spans: Vec<Painted>,
}

/// A capture, copied out of the query cursor so captures can be looked
/// ahead at.
struct Capture {
    node: usize,
    bytes: Range<usize>,
    pattern: usize,
    index: u32,
    /// For a definition: where its value ends (`local.definition-value`).
    value_end: usize,
}

struct Scope {
    end: usize,
    inherits: bool,
    definitions: Vec<Definition>,
}

struct Definition {
    name: String,
    value_end: usize,
    highlight: Option<usize>,
}

impl Layers<'_, '_> {
    /// Highlights one language layer (the document, or an injection whose
    /// text is `ranges`), then the injections inside it.
    fn layer(
        &mut self,
        tree: &Tree,
        config: &LanguageConfig,
        parent: Option<&str>,
        depth: usize,
        ranges: &[tree_sitter::Range],
    ) {
        let root = tree.root_node();
        // Locals are scoped, so start from the top-level node holding the
        // first row, to see the definitions in scope there.
        let mut query_bytes = self.bytes.clone();
        if config.has_locals() {
            let mut cursor = root.walk();
            if let Some(top) = root
                .children(&mut cursor)
                .find(|n| n.end_byte() > self.bytes.start)
            {
                query_bytes.start = query_bytes.start.min(top.start_byte());
            }
        }
        let captures = self.captures(tree, config, query_bytes);
        self.highlight_captures(config, &captures, depth);

        let Some(injections) = &config.injections else {
            return;
        };
        if depth >= MAX_DEPTH {
            return;
        }
        for (name, nodes, include_children) in self.injections(tree, config, injections, parent) {
            let Some(injected) = (self.inject)(&name) else {
                continue;
            };
            let included = intersect_ranges(ranges, &nodes, include_children);
            if included.is_empty()
                || self.parser.set_language(&injected.language).is_err()
                || self.parser.set_included_ranges(&included).is_err()
            {
                continue;
            }
            let buffer = self.buffer;
            let parsed = self.parser.parse_with_options(
                &mut |byte, _| buffer.chunk_from_byte(byte),
                None,
                None,
            );
            if let Some(injected_tree) = parsed {
                self.layer(
                    &injected_tree,
                    injected,
                    Some(config.name),
                    depth + 1,
                    &included,
                );
            }
        }
    }

    fn captures(&self, tree: &Tree, config: &LanguageConfig, bytes: Range<usize>) -> Vec<Capture> {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes);
        let buffer = self.buffer;
        let text = |node: Node| buffer.byte_chunks(node.byte_range());
        let mut found = Vec::new();
        let mut captures = cursor.captures(&config.query, tree.root_node(), text);
        while let Some((m, i)) = captures.next() {
            let capture = m.captures()[*i];
            let value_end = if Some(capture.index) == config.definition_capture {
                m.captures()
                    .iter()
                    .find(|c| Some(c.index) == config.definition_value_capture)
                    .map_or(0, |c| c.node.end_byte())
            } else {
                0
            };
            found.push(Capture {
                node: capture.node.id(),
                bytes: capture.node.byte_range(),
                pattern: m.pattern_index,
                index: capture.index,
                value_end,
            });
        }
        found
    }

    /// Turns captures into highlighted spans, tracking local scopes,
    /// definitions and references as `tree-sitter-highlight` does.
    fn highlight_captures(&mut self, config: &LanguageConfig, captures: &[Capture], depth: usize) {
        let mut scopes = vec![Scope {
            end: usize::MAX,
            inherits: false,
            definitions: Vec::new(),
        }];
        let same_node = |i: usize, node: usize| captures.get(i).is_some_and(|c| c.node == node);
        let mut i = 0;
        'captures: while i < captures.len() {
            let mut capture = &captures[i];
            while scopes.len() > 1 && capture.bytes.start > scopes.last().expect("root").end {
                scopes.pop();
            }

            // Locals patterns come first among a node's captures.
            let mut reference: Option<Option<usize>> = None;
            let mut definition: Option<(usize, usize)> = None;
            while capture.pattern < config.locals_patterns {
                let index = Some(capture.index);
                if index == config.scope_capture {
                    definition = None;
                    let inherits = config
                        .query
                        .property_settings(capture.pattern)
                        .iter()
                        .find(|p| &*p.key == "local.scope-inherits")
                        .is_none_or(|p| p.value.as_deref().is_none_or(|v| v == "true"));
                    scopes.push(Scope {
                        end: capture.bytes.end,
                        inherits,
                        definitions: Vec::new(),
                    });
                } else if index == config.definition_capture {
                    reference = None;
                    let name = self.buffer.byte_text(capture.bytes.clone());
                    let scope = scopes.len() - 1;
                    scopes[scope].definitions.push(Definition {
                        name,
                        value_end: capture.value_end,
                        highlight: None,
                    });
                    definition = Some((scope, scopes[scope].definitions.len() - 1));
                } else if index == config.reference_capture && definition.is_none() {
                    let name = self.buffer.byte_text(capture.bytes.clone());
                    for scope in scopes.iter().rev() {
                        let found = scope
                            .definitions
                            .iter()
                            .rev()
                            .find(|d| d.name == name && capture.bytes.start >= d.value_end);
                        if let Some(found) = found {
                            reference = Some(found.highlight);
                            break;
                        }
                        if !scope.inherits {
                            break;
                        }
                    }
                }
                i += 1;
                if !same_node(i, capture.node) {
                    continue 'captures;
                }
                capture = &captures[i];
            }

            // A highlight: the last pattern matching this node wins, except
            // that patterns for non-locals are skipped for locals.
            let is_local = definition.is_some() || reference.flatten().is_some();
            while same_node(i + 1, capture.node) {
                i += 1;
                if !(is_local && config.non_local[captures[i].pattern]) {
                    capture = &captures[i];
                }
            }
            i += 1;
            let highlight = config.highlight_indices[capture.index as usize];
            if let Some((scope, index)) = definition {
                scopes[scope].definitions[index].highlight = highlight;
            }
            if let Some(highlight) = reference.flatten().or(highlight) {
                self.spans.push(Painted {
                    bytes: capture.bytes.clone(),
                    depth,
                    highlight,
                });
            }
        }
    }

    /// The injections whose nodes touch the rows being highlighted: the
    /// language's name, the nodes holding its text, and whether their
    /// children's text counts. Matches of a combined pattern (one document
    /// spread over several nodes) are gathered into one.
    fn injections<'t>(
        &self,
        tree: &'t Tree,
        config: &LanguageConfig,
        query: &Query,
        parent: Option<&str>,
    ) -> Vec<(String, Vec<Node<'t>>, bool)> {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(self.bytes.clone());
        let buffer = self.buffer;
        let text = |node: Node| buffer.byte_chunks(node.byte_range());
        let mut found: Vec<(String, Vec<Node<'t>>, bool)> = Vec::new();
        let mut combined: Vec<Option<usize>> = vec![None; query.pattern_count()];
        let mut matches = cursor.matches(query, tree.root_node(), text);
        while let Some(m) = matches.next() {
            let (name, content, include_children) =
                injection_for_match(config, query, m, parent, buffer);
            let Some(content) = content else { continue };
            let is_combined = query
                .property_settings(m.pattern_index)
                .iter()
                .any(|p| &*p.key == "injection.combined");
            if is_combined && let Some(at) = combined[m.pattern_index] {
                let entry = &mut found[at];
                if entry.0.is_empty()
                    && let Some(name) = name
                {
                    entry.0 = name;
                }
                entry.1.push(content);
                continue;
            }
            if is_combined {
                combined[m.pattern_index] = Some(found.len());
            } else if name.is_none() {
                continue;
            }
            found.push((name.unwrap_or_default(), vec![content], include_children));
        }
        found.retain(|(name, _, _)| !name.is_empty());
        found
    }

    /// One highlight per byte of the rows being highlighted (`None` for
    /// plain text). Spans paint over earlier ones, in the order
    /// `tree-sitter-highlight` starts them: by where they start, deeper
    /// layers first, then as the query found them. A span exactly covering
    /// the previous one from a deeper layer is dropped, so injected text
    /// keeps its own highlight.
    fn paint(mut self) -> Vec<Option<u16>> {
        let start = self.bytes.start;
        let mut paint = vec![None; self.bytes.len()];
        // Painted in order, later over earlier: by start, and at the same
        // start outer layers first, so an injected language's span wins
        // over its host's even when the host's runs on further (a
        // Markdown title's over the code span in it). A stable sort keeps
        // the query's order within a layer.
        self.spans.sort_by_key(|p| (p.bytes.start, p.depth));
        for span in &self.spans {
            let from = span.bytes.start.max(start);
            let to = span.bytes.end.min(self.bytes.end);
            for byte in from..to {
                paint[byte - start] = Some(span.highlight as u16);
            }
        }
        paint
    }
}

/// The injected language's name, the node holding its text, and whether
/// that node's children's text counts, for one injection match.
fn injection_for_match<'t>(
    config: &LanguageConfig,
    query: &Query,
    m: &QueryMatch<'_, 't>,
    parent: Option<&str>,
    buffer: &Buffer,
) -> (Option<String>, Option<Node<'t>>, bool) {
    let mut name = None;
    let mut content = None;
    for capture in m.captures() {
        if Some(capture.index) == config.injection_language_capture {
            name = Some(buffer.byte_text(capture.node.byte_range()));
        } else if Some(capture.index) == config.injection_content_capture {
            content = Some(capture.node);
        }
    }
    let mut include_children = false;
    for property in query.property_settings(m.pattern_index) {
        match &*property.key {
            "injection.language" if name.is_none() => {
                name = property.value.as_deref().map(str::to_string);
            }
            "injection.self" if name.is_none() => name = Some(config.name.to_string()),
            "injection.parent" if name.is_none() => name = parent.map(str::to_string),
            "injection.include-children" => include_children = true,
            _ => {}
        }
    }
    (name, content, include_children)
}

/// The text an injection covers: its nodes' ranges (less their children's,
/// unless `include_children`), within the host layer's own `parents`.
/// (From `tree-sitter-highlight`.)
fn intersect_ranges(
    parents: &[tree_sitter::Range],
    nodes: &[Node],
    include_children: bool,
) -> Vec<tree_sitter::Range> {
    let mut result = Vec::new();
    let mut parent_iter = parents.iter();
    let Some(mut parent) = parent_iter.next() else {
        return result;
    };
    for node in nodes {
        let mut preceding = tree_sitter::Range {
            start_byte: 0,
            start_point: Point::new(0, 0),
            end_byte: node.start_byte(),
            end_point: node.start_position(),
        };
        let following = tree_sitter::Range {
            start_byte: node.end_byte(),
            start_point: node.end_position(),
            end_byte: usize::MAX,
            end_point: Point::new(usize::MAX, usize::MAX),
        };
        let mut cursor = node.walk();
        let children: Vec<tree_sitter::Range> = if include_children {
            Vec::new()
        } else {
            node.children(&mut cursor).map(|c| c.range()).collect()
        };
        for excluded in children.into_iter().chain(std::iter::once(following)) {
            let mut range = tree_sitter::Range {
                start_byte: preceding.end_byte,
                start_point: preceding.end_point,
                end_byte: excluded.start_byte,
                end_point: excluded.start_point,
            };
            preceding = excluded;
            if range.end_byte < parent.start_byte {
                continue;
            }
            while parent.start_byte <= range.end_byte {
                if parent.end_byte > range.start_byte {
                    if range.start_byte < parent.start_byte {
                        range.start_byte = parent.start_byte;
                        range.start_point = parent.start_point;
                    }
                    if parent.end_byte < range.end_byte {
                        if range.start_byte < parent.end_byte {
                            result.push(tree_sitter::Range {
                                start_byte: range.start_byte,
                                start_point: range.start_point,
                                end_byte: parent.end_byte,
                                end_point: parent.end_point,
                            });
                        }
                        range.start_byte = parent.end_byte;
                        range.start_point = parent.end_point;
                    } else {
                        if range.start_byte < range.end_byte {
                            result.push(range);
                        }
                        break;
                    }
                }
                match parent_iter.next() {
                    Some(next) => parent = next,
                    None => return result,
                }
            }
        }
    }
    result
}

/// Splits the painted bytes into spans per row, in char columns.
fn spans_by_line(
    buffer: &Buffer,
    rows: Range<usize>,
    first_byte: usize,
    paint: &[Option<u16>],
) -> Vec<Vec<Span>> {
    let mut lines = Vec::with_capacity(rows.len());
    for row in rows {
        let line = buffer.line(row);
        let mut byte = buffer.row_to_byte(row) - first_byte;
        let mut spans: Vec<Span> = Vec::new();
        for (col, c) in line.chars().enumerate() {
            if let Some(highlight) = paint.get(byte).copied().flatten() {
                let highlight = highlight as usize;
                match spans.last_mut() {
                    Some(span) if span.end == col && span.highlight == highlight => span.end += 1,
                    _ => spans.push(Span {
                        start: col,
                        end: col + 1,
                        highlight,
                    }),
                }
            }
            byte += c.len_utf8();
        }
        lines.push(spans);
    }
    lines
}
