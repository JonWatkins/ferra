//! Line ranges in front of `:` commands: `%`, `5`, `5,10`, `.`, `$`,
//! `.+3`, `$-1`, `'<,'>`.

/// The first and last rows of a range, 0-based and inclusive.
pub type LineRange = (usize, usize);

/// What addresses refer to. Rows are 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeContext {
    pub cursor_row: usize,
    pub line_count: usize,
    /// First and last rows of the last visual selection, for `'<` and `'>`.
    pub visual: Option<LineRange>,
}

/// Splits a leading range off `input`. Returns the rows it covers
/// (inclusive, 0-based, ordered), if there was one, and the rest of the
/// input. Line numbers past the end are clamped to the last line.
pub fn parse_range<'a>(
    input: &'a str,
    cx: &RangeContext,
) -> Result<(Option<LineRange>, &'a str), String> {
    let input = input.trim_start();
    if let Some(rest) = input.strip_prefix('%') {
        return Ok((Some((0, cx.line_count.saturating_sub(1))), rest));
    }

    let (first, rest) = match parse_address(input, cx)? {
        Some(found) => found,
        None => return Ok((None, input)),
    };
    let (last, rest) = match rest.strip_prefix(',') {
        Some(after) => parse_address(after, cx)?.ok_or("Missing address after ','")?,
        None => (first, rest),
    };
    let (first, last) = if first <= last {
        (first, last)
    } else {
        (last, first)
    };
    Ok((Some((first, last)), rest))
}

/// One address with any `+N`/`-N` offsets, as a clamped 0-based row.
fn parse_address<'a>(
    input: &'a str,
    cx: &RangeContext,
) -> Result<Option<(usize, &'a str)>, String> {
    let last_row = cx.line_count.saturating_sub(1) as i64;
    let (mut row, mut rest): (i64, &str) = if let Some(rest) = input.strip_prefix('.') {
        (cx.cursor_row as i64, rest)
    } else if let Some(rest) = input.strip_prefix('$') {
        (last_row, rest)
    } else if let Some(rest) = input.strip_prefix("'<") {
        (
            cx.visual.ok_or("No previous visual selection")?.0 as i64,
            rest,
        )
    } else if let Some(rest) = input.strip_prefix("'>") {
        (
            cx.visual.ok_or("No previous visual selection")?.1 as i64,
            rest,
        )
    } else if input.starts_with(|c: char| c.is_ascii_digit()) {
        let (number, rest) = split_number(input);
        (number as i64 - 1, rest)
    } else if input.starts_with(['+', '-']) {
        (cx.cursor_row as i64, input)
    } else {
        return Ok(None);
    };

    while let Some(sign) = rest.chars().next().filter(|c| matches!(c, '+' | '-')) {
        let (amount, after) = split_number(&rest[1..]);
        let amount = if after.len() == rest.len() - 1 {
            1
        } else {
            amount as i64
        };
        row += if sign == '+' { amount } else { -amount };
        rest = after;
    }
    Ok(Some((row.clamp(0, last_row) as usize, rest)))
}

/// A leading run of digits as a number (0 if none), and the rest.
fn split_number(input: &str) -> (u64, &str) {
    let end = input
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(input.len());
    (input[..end].parse().unwrap_or(0), &input[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx() -> RangeContext {
        RangeContext {
            cursor_row: 4,
            line_count: 20,
            visual: Some((2, 6)),
        }
    }

    fn range(input: &str) -> (Option<(usize, usize)>, &str) {
        parse_range(input, &cx()).unwrap()
    }

    #[test]
    fn no_range() {
        assert_eq!(range("w"), (None, "w"));
        assert_eq!(range("s/a/b/"), (None, "s/a/b/"));
    }

    #[test]
    fn whole_file_and_single_lines() {
        assert_eq!(range("%s/a/b/"), (Some((0, 19)), "s/a/b/"));
        assert_eq!(range("5"), (Some((4, 4)), ""));
        assert_eq!(range("."), (Some((4, 4)), ""));
        assert_eq!(range("$d"), (Some((19, 19)), "d"));
    }

    #[test]
    fn pairs_offsets_and_ordering() {
        assert_eq!(range("2,5y"), (Some((1, 4)), "y"));
        assert_eq!(range(".,.+3d"), (Some((4, 7)), "d"));
        assert_eq!(range("$-2,$"), (Some((17, 19)), ""));
        assert_eq!(range("+2"), (Some((6, 6)), ""));
        assert_eq!(range(".-"), (Some((3, 3)), "",), "a bare sign means 1");
        assert_eq!(
            range("8,3"),
            (Some((2, 7)), ""),
            "backwards ranges are swapped"
        );
    }

    #[test]
    fn visual_marks() {
        assert_eq!(range("'<,'>s/x/y/"), (Some((2, 6)), "s/x/y/"));
        let no_visual = RangeContext {
            visual: None,
            ..cx()
        };
        assert!(parse_range("'<,'>d", &no_visual).is_err());
    }

    #[test]
    fn clamped_to_the_buffer() {
        assert_eq!(range("999"), (Some((19, 19)), ""));
        assert_eq!(range("0"), (Some((0, 0)), ""));
        assert_eq!(range("1-5"), (Some((0, 0)), ""));
    }

    #[test]
    fn missing_second_address_is_an_error() {
        assert!(parse_range("3,", &cx()).is_err());
    }
}
