//! Folding by indentation: where the fold that a line heads (or is in)
//! starts and ends.
//!
//! A line heads a fold when the lines after it are indented more than it
//! (blank lines among them count as theirs). The fold runs to the last of
//! those lines and, when the next line is at the header's indent and starts
//! by closing a bracket (`}`, `)`, `]`), that line too, so a function folds
//! with its closing brace. This works the same in every language, Python
//! and YAML included.
//!
//! Fold regions come from indentation (`core/fold.rs`): `region_from(row)` is
//! the row and the lines after it indented more (blank lines among them
//! included), plus a closing-bracket line at the row's own indent;
//! `region_around(row)` is that, or the region of the nearest line above, no
//! deeper, that reaches the row. `za`/`zc` close the region around the cursor,
//! `zM` every outermost one, `zf` any operator range (`Operator::Fold`).
//! The fold column (`fold-column`) marks each drawn line with `Column`,
//! which gives the same answers as those two without searching back from
//! every row.

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

/// What the fold column shows beside a line, for indentation folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// In no fold.
    None,
    /// Heads a fold (`region_from` has one).
    Head,
    /// Inside a fold that starts above it (`region_around` has one).
    Inside,
}

/// The fold column's marks for rows going down the screen, without
/// `region_around`'s search back for every row: one scan up from the first
/// row, then the lines between the rows asked for (hidden ones included).
///
/// A line indented `i` is inside a fold when a line above it is indented
/// less (the nearest such heads a fold over everything since, which is all
/// deeper), or when it's a closing bracket at a fold header's indent, right
/// after deeper lines, with nothing above it shallower. A blank line is in
/// the fold the next line that isn't blank is in.
#[derive(Debug)]
pub struct Column<'a> {
    buffer: &'a Buffer,
    tab_width: usize,
    /// The first row not yet taken into `least` and `previous`.
    next: usize,
    /// The least indent of the lines above `next` that aren't blank.
    least: Option<usize>,
    /// The indent of the last line above `next` that isn't blank.
    previous: Option<usize>,
}

impl<'a> Column<'a> {
    /// The column from `top` down.
    pub fn new(buffer: &'a Buffer, top: usize, tab_width: usize) -> Self {
        let mut least = None;
        let mut previous = None;
        for row in (0..top).rev() {
            let Some(i) = indent(buffer, row, tab_width) else {
                continue;
            };
            previous.get_or_insert(i);
            least = Some(least.map_or(i, |l: usize| l.min(i)));
            if i == 0 {
                break;
            }
        }
        Self {
            buffer,
            tab_width,
            next: top,
            least,
            previous,
        }
    }

    /// The mark beside `row`, which must be at or below the last asked for.
    pub fn mark(&mut self, row: usize) -> Mark {
        while self.next < row {
            if let Some(i) = indent(self.buffer, self.next, self.tab_width) {
                self.previous = Some(i);
                self.least = Some(self.least.map_or(i, |l| l.min(i)));
            }
            self.next += 1;
        }
        let after = |row: usize| {
            (row + 1..self.buffer.line_count())
                .find_map(|r| indent(self.buffer, r, self.tab_width).map(|i| (r, i)))
        };
        let (line, own) = match indent(self.buffer, row, self.tab_width) {
            Some(own) => {
                if after(row).is_some_and(|(_, i)| i > own) {
                    return Mark::Head;
                }
                (row, own)
            }
            None => match after(row) {
                Some(next) => next,
                None => return Mark::None,
            },
        };
        let closer = || {
            self.buffer
                .line(line)
                .trim_start()
                .starts_with(['}', ')', ']'])
        };
        let inside = self.least.is_some_and(|l| l < own)
            || (self.least == Some(own) && self.previous.is_some_and(|p| p > own) && closer());
        if inside { Mark::Inside } else { Mark::None }
    }
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

#[cfg(test)]
mod column_tests {
    use super::*;

    /// The mark by definition: from `region_from` and `region_around`.
    fn expected(buffer: &Buffer, row: usize) -> Mark {
        if region_from(buffer, row, 4).is_some() {
            Mark::Head
        } else if region_around(buffer, row, 4).is_some() {
            Mark::Inside
        } else {
            Mark::None
        }
    }

    const TEXTS: &[&str] = &[
        "fn a() {\n    one();\n\n    if x {\n        two();\n    }\n}\nfn b() {}\n",
        "def f():\n    a\n\n    b\n\nx = 1\n",
        "    indented\n        deeper\n    back\n  less\n}\n",
        "a\n\n\n  b\n\n\nc\n  d\n    e\n  }\n}\n\n",
        "impl X {\n    fn a() {\n        x\n    }\n\n    fn b() {\n    }\n}\n",
        "",
        "\n\n",
    ];

    #[test]
    fn column_marks_match_the_fold_regions_from_any_first_row() {
        for text in TEXTS {
            let b = Buffer::from_text(text);
            for top in 0..b.line_count() {
                let mut column = Column::new(&b, top, 4);
                for row in top..b.line_count() {
                    assert_eq!(
                        column.mark(row),
                        expected(&b, row),
                        "row {row} from {top} of {text:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn column_skips_rows() {
        let b = Buffer::from_text(TEXTS[0]);
        let mut column = Column::new(&b, 0, 4);
        assert_eq!(column.mark(0), Mark::Head);
        assert_eq!(
            column.mark(6),
            Mark::Inside,
            "the closing brace, rows hidden"
        );
        assert_eq!(column.mark(7), Mark::None);
    }
}
