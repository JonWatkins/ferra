//! `<C-a>` and `<C-x>`: finding a number in a line and adding to it.
//!
//! Numbers are decimal (with a `-` sign right before the digits), hex
//! (`0x1f`, `0X1F`) or binary (`0b101`), as Neovim's default `nrformats`.
//! A hex or binary number keeps its width (zero-padded) and the case of its
//! letters; a decimal one with leading zeros keeps its width too
//! (`007` → `008`, `099` → `100`).

/// A number found in a line: chars `start..end` (including any sign or
/// `0x` prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberAt {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    Decimal,
    Hex,
    Binary,
}

/// A number written in the line.
struct Literal {
    start: usize,
    /// Where the digits start (after a sign or prefix).
    digits: usize,
    end: usize,
    base: Base,
    negative: bool,
}

/// The number under or after char column `col` in `chars[..limit]` (the
/// whole line, or a selection's part of it), starting no earlier than
/// `from`.
fn literal_at(chars: &[char], col: usize, from: usize, limit: usize) -> Option<Literal> {
    let limit = limit.min(chars.len());

    // A hex or binary number the cursor is in: look back for its `0x`.
    let mut i = col.min(limit.saturating_sub(1));
    while i > from && (chars[i].is_ascii_hexdigit() || matches!(chars[i], 'x' | 'X')) {
        i -= 1;
    }
    for start in i..=col.min(limit.saturating_sub(1)) {
        if let Some(literal) = prefixed(chars, start, limit)
            && literal.end > col.max(start)
        {
            return Some(literal);
        }
    }

    // Otherwise the decimal (or prefixed) number under or after the cursor.
    let mut at = col.max(from);
    // Back to the start of digits the cursor is in.
    while at > from && at < limit && chars[at].is_ascii_digit() && chars[at - 1].is_ascii_digit() {
        at -= 1;
    }
    while at < limit {
        if chars[at].is_ascii_digit() {
            if let Some(literal) = prefixed(chars, at, limit) {
                return Some(literal);
            }
            let mut end = at;
            while end < limit && chars[end].is_ascii_digit() {
                end += 1;
            }
            let negative = at > from && chars[at - 1] == '-';
            return Some(Literal {
                start: if negative { at - 1 } else { at },
                digits: at,
                end,
                base: Base::Decimal,
                negative,
            });
        }
        at += 1;
    }
    None
}

/// A `0x...` or `0b...` number starting at `start`, if there is one.
fn prefixed(chars: &[char], start: usize, limit: usize) -> Option<Literal> {
    if start + 2 >= limit || chars.get(start) != Some(&'0') {
        return None;
    }
    let (base, valid): (Base, fn(char) -> bool) = match chars.get(start + 1)? {
        'x' | 'X' => (Base::Hex, |c| c.is_ascii_hexdigit()),
        'b' | 'B' => (Base::Binary, |c| c == '0' || c == '1'),
        _ => return None,
    };
    let digits = start + 2;
    let mut end = digits;
    while end < limit && valid(chars[end]) {
        end += 1;
    }
    (end > digits).then_some(Literal {
        start,
        digits,
        end,
        base,
        negative: false,
    })
}

/// The number `<C-a>` at char column `col` of `line` would change: the one
/// under the cursor, or the next on the line.
pub fn find(line: &str, col: usize) -> Option<NumberAt> {
    let chars: Vec<char> = line.chars().collect();
    let literal = literal_at(&chars, col, 0, chars.len())?;
    Some(NumberAt {
        start: literal.start,
        end: literal.end,
    })
}

/// Adds `delta` to the number under or after `col` in `line`, looking only
/// at chars `from..limit`. Returns the new line and where the number now
/// is, or `None` if there's no number there.
pub fn add(
    line: &str,
    col: usize,
    from: usize,
    limit: usize,
    delta: i128,
) -> Option<(String, NumberAt)> {
    let chars: Vec<char> = line.chars().collect();
    let literal = literal_at(&chars, col.max(from), from, limit)?;
    let digits: String = chars[literal.digits..literal.end].iter().collect();
    let width = digits.len();
    let replacement = match literal.base {
        Base::Decimal => {
            let magnitude: i128 = digits.parse().unwrap_or(i128::MAX);
            let value = if literal.negative {
                -magnitude
            } else {
                magnitude
            }
            .saturating_add(delta);
            let text = value.unsigned_abs().to_string();
            // Leading zeros: keep the width.
            let text = if digits.starts_with('0') && width > 1 && text.len() < width {
                format!("{text:0>width$}")
            } else {
                text
            };
            if value < 0 { format!("-{text}") } else { text }
        }
        Base::Hex | Base::Binary => {
            let radix = if literal.base == Base::Hex { 16 } else { 2 };
            let value = u128::from_str_radix(&digits, radix).unwrap_or(0);
            // Wraps like an unsigned number of its width, as in Vim.
            let bits = (width * if radix == 16 { 4 } else { 1 }).min(128);
            let modulus = if bits >= 128 {
                None
            } else {
                Some(1u128 << bits)
            };
            let raw = (value as i128).wrapping_add(delta);
            let value = match modulus {
                Some(m) => raw.rem_euclid(m as i128) as u128,
                None => raw as u128,
            };
            let mut text = if radix == 16 {
                format!("{value:0width$x}")
            } else {
                format!("{value:0width$b}")
            };
            // Hex letters take the case of the last letter there was.
            let upper = digits
                .chars()
                .rev()
                .find(char::is_ascii_alphabetic)
                .is_some_and(|c| c.is_ascii_uppercase());
            if upper {
                text = text.to_ascii_uppercase();
            }
            let prefix: String = chars[literal.start..literal.digits].iter().collect();
            format!("{prefix}{text}")
        }
    };
    let before: String = chars[..literal.start].iter().collect();
    let after: String = chars[literal.end..].iter().collect();
    let start = literal.start;
    let end = start + replacement.chars().count();
    Some((
        format!("{before}{replacement}{after}"),
        NumberAt { start, end },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bump(line: &str, col: usize, delta: i128) -> Option<String> {
        add(line, col, 0, usize::MAX, delta).map(|(text, _)| text)
    }

    #[test]
    fn decimal_under_or_after_the_cursor() {
        assert_eq!(bump("x = 41;", 0, 1).as_deref(), Some("x = 42;"));
        assert_eq!(
            bump("x = 41;", 5, 1).as_deref(),
            Some("x = 42;"),
            "on the 1"
        );
        assert_eq!(bump("v1.9", 3, 1).as_deref(), Some("v1.10"));
        assert_eq!(bump("no numbers", 0, 1), None);
        assert_eq!(
            bump("1 and 2", 2, 1).as_deref(),
            Some("1 and 3"),
            "after, not before"
        );
    }

    #[test]
    fn signs_and_crossing_zero() {
        assert_eq!(bump("-1", 0, 2).as_deref(), Some("1"));
        assert_eq!(bump("1", 0, -3).as_deref(), Some("-2"));
        assert_eq!(bump("a -5 b", 0, 10).as_deref(), Some("a 5 b"));
        assert_eq!(
            bump("x-1", 2, 1).as_deref(),
            Some("x0"),
            "a - right before is a sign"
        );
    }

    #[test]
    fn leading_zeros_keep_the_width() {
        assert_eq!(bump("007", 0, 1).as_deref(), Some("008"));
        assert_eq!(bump("099", 0, 1).as_deref(), Some("100"));
        assert_eq!(bump("010", 0, -10).as_deref(), Some("000"));
    }

    #[test]
    fn hex_and_binary() {
        assert_eq!(bump("0x0f", 0, 1).as_deref(), Some("0x10"));
        assert_eq!(
            bump("0xFF", 3, 1).as_deref(),
            Some("0x00"),
            "wraps at its width"
        );
        assert_eq!(bump("0x9", 0, 1).as_deref(), Some("0xa"));
        assert_eq!(
            bump("0X1E", 0, 1).as_deref(),
            Some("0X1F"),
            "keeps upper case"
        );
        assert_eq!(bump("0b0111", 0, 1).as_deref(), Some("0b1000"));
        assert_eq!(bump("0b00", 0, -1).as_deref(), Some("0b11"));
        assert_eq!(
            bump("c = 0xff;", 7, 1).as_deref(),
            Some("c = 0x00;"),
            "cursor in it"
        );
    }

    #[test]
    fn within_part_of_the_line() {
        let (text, at) = add("1 2 3", 0, 2, 5, 5).unwrap();
        assert_eq!(text, "1 7 3");
        assert_eq!(at, NumberAt { start: 2, end: 3 });
        assert_eq!(add("1 2 3", 0, 1, 2, 5), None, "nothing in the space");
    }

    #[test]
    fn find_reports_the_span() {
        assert_eq!(find("a -12 b", 0), Some(NumberAt { start: 2, end: 5 }));
        assert_eq!(find("0x1f", 2), Some(NumberAt { start: 0, end: 4 }));
    }
}
