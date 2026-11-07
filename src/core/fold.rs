//! Folding by indentation: where the fold that a line heads (or is in)
//! starts and ends.
//!
//! A line heads a fold when the lines after it are indented more than it
//! (blank lines among them count as theirs). The fold runs to the last of
//! those lines and, when the next line is at the header's indent and starts
//! by closing a bracket (`}`, `)`, `]`), that line too, so a function folds
//! with its closing brace. This works the same in every language, Python
//! and YAML included.

use super::Buffer;
use super::columns::display_width;

/// The indentation of `row` in display columns, or `None` for a blank line.
fn indent(buffer: &Buffer, row: usize, tab_width: usize) -> Option<usize> {
    let line = buffer.line(row);
    let rest = line.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return None;
    }
    Some(display_width(&line[..line.len() - rest.len()], tab_width))
}

/// The fold `row` heads: `row` and the last line folded under it.
pub fn region_from(buffer: &Buffer, row: usize, tab_width: usize) -> Option<(usize, usize)> {
    let own = indent(buffer, row, tab_width)?;
    let count = buffer.line_count();
    let mut end = row;
    let mut next = row + 1;
    while next < count {
        match indent(buffer, next, tab_width) {
            None => {}
            Some(i) if i > own => end = next,
            Some(_) => break,
        }
        next += 1;
    }
    if end == row {
        return None;
    }
    // The closing line at the header's indent, after any blank lines.
    let after = (end + 1..count).find(|&r| indent(buffer, r, tab_width).is_some());
    if let Some(after) = after
        && indent(buffer, after, tab_width) == Some(own)
        && buffer.line(after).trim_start().starts_with(['}', ')', ']'])
    {
        end = after;
    }
    Some((row, end))
}

/// The innermost fold `row` is in: the one it heads, or else that of the
/// nearest line above it, no deeper than it, whose fold reaches it.
pub fn region_around(buffer: &Buffer, row: usize, tab_width: usize) -> Option<(usize, usize)> {
    if let Some(region) = region_from(buffer, row, tab_width) {
        return Some(region);
    }
    let own = indent(buffer, row, tab_width);
    (0..row).rev().find_map(|above| {
        let i = indent(buffer, above, tab_width)?;
        if own.is_some_and(|own| i > own) {
            return None;
        }
        region_from(buffer, above, tab_width).filter(|&(_, end)| end >= row)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "fn a() {\n    one();\n\n    if x {\n        two();\n    }\n}\nfn b() {}\n";

    fn buffer() -> Buffer {
        Buffer::from_text(CODE)
    }

    #[test]
    fn a_line_heads_the_more_indented_lines_after_it() {
        let b = buffer();
        assert_eq!(
            region_from(&b, 0, 4),
            Some((0, 6)),
            "with its closing brace"
        );
        assert_eq!(region_from(&b, 3, 4), Some((3, 5)));
        assert_eq!(region_from(&b, 1, 4), None, "nothing indented under it");
        assert_eq!(region_from(&b, 7, 4), None);
    }

    #[test]
    fn a_line_inside_belongs_to_the_nearest_header_above() {
        let b = buffer();
        assert_eq!(region_around(&b, 1, 4), Some((0, 6)));
        assert_eq!(region_around(&b, 4, 4), Some((3, 5)));
        assert_eq!(
            region_around(&b, 5, 4),
            Some((3, 5)),
            "a closing brace: its block"
        );
        assert_eq!(region_around(&b, 2, 4), Some((0, 6)), "a blank line");
        assert_eq!(region_around(&b, 7, 4), None);
    }

    #[test]
    fn indentation_alone_works_without_brackets() {
        let b = Buffer::from_text("def f():\n    a\n    b\nx = 1\n");
        assert_eq!(region_from(&b, 0, 4), Some((0, 2)));
    }
}
