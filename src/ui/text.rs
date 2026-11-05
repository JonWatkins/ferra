//! Small text-fitting helpers shared by widgets.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ELLIPSIS: char = '…';

/// Shortens `text` to at most `max_width` cells by cutting from the start,
/// marking the cut with an ellipsis: `/very/long/path/file.rs` → `…path/file.rs`.
pub fn truncate_start(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut kept = Vec::new();
    let mut width = 1; // the ellipsis
    for c in text.chars().rev() {
        let w = c.width().unwrap_or(0);
        if width + w > max_width {
            break;
        }
        width += w;
        kept.push(c);
    }
    std::iter::once(ELLIPSIS)
        .chain(kept.into_iter().rev())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_start_cases() {
        assert_eq!(truncate_start("short", 10), "short");
        assert_eq!(truncate_start("abcdefgh", 5), "…efgh");
        assert_eq!(truncate_start("日本語", 4), "…語");
        assert_eq!(truncate_start("abc", 0), "");
    }
}
