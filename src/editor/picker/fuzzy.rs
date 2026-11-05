//! A small fuzzy matcher for the picker.
//!
//! A query matches a candidate if its characters appear in order (not
//! necessarily together). Matches score higher when they start words or run
//! consecutively, so `bn` ranks `buffer_next` above `fabian`. Matching is
//! case-insensitive unless the query has an uppercase letter.

/// Bonus for each matched character.
const MATCH: i64 = 16;
/// Bonus when a match directly follows the previous one.
const CONSECUTIVE: i64 = 12;
/// Bonus when a match starts a word (`_`, `-`, `/`, ` `, `.` or camelCase).
const WORD_START: i64 = 10;
/// Penalty per skipped character between matches.
const GAP: i64 = 1;

/// A successful match: its score, and the char positions in the candidate
/// that matched (for highlighting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i64,
    pub positions: Vec<usize>,
}

/// Matches `query` against `candidate`. An empty query matches everything
/// with a score of zero.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let case_sensitive = query.chars().any(char::is_uppercase);
    let fold = |c: char| {
        if case_sensitive {
            c
        } else {
            c.to_ascii_lowercase()
        }
    };
    let chars: Vec<char> = candidate.chars().collect();
    let query: Vec<char> = query.chars().map(fold).collect();

    // Try starting the match at each occurrence of the first query char
    // and keep the best, so `bn` against "a_bn_b_n" finds the tight run.
    let starts = chars
        .iter()
        .enumerate()
        .filter(|&(_, &c)| query.first().is_none_or(|&q| fold(c) == q))
        .map(|(i, _)| i);

    let mut best: Option<FuzzyMatch> = None;
    for start in starts {
        let Some(found) = match_from(&query, &chars, start, fold) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| found.score > b.score) {
            best = Some(found);
        }
        if query.is_empty() {
            break;
        }
    }
    if chars.is_empty() && query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    best
}

/// Greedily matches `query` in `chars`, starting at `start`.
fn match_from(
    query: &[char],
    chars: &[char],
    start: usize,
    fold: impl Fn(char) -> char,
) -> Option<FuzzyMatch> {
    let mut positions = Vec::with_capacity(query.len());
    let mut score = 0;
    let mut next = start;

    for &q in query {
        let at = (next..chars.len()).find(|&i| fold(chars[i]) == q)?;
        score += MATCH;
        if positions.last().is_some_and(|&last| last + 1 == at) {
            score += CONSECUTIVE;
        } else if let Some(&last) = positions.last() {
            score -= GAP * (at - last - 1) as i64;
        }
        if is_word_start(chars, at) {
            score += WORD_START;
        }
        positions.push(at);
        next = at + 1;
    }
    // Earlier matches are slightly better.
    score -= start as i64 / 4;
    Some(FuzzyMatch { score, positions })
}

fn is_word_start(chars: &[char], i: usize) -> bool {
    let Some(&prev) = i.checked_sub(1).and_then(|p| chars.get(p)) else {
        return true;
    };
    matches!(prev, '_' | '-' | '/' | ' ' | '.') || (prev.is_lowercase() && chars[i].is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, candidate: &str) -> Option<i64> {
        fuzzy_match(query, candidate).map(|m| m.score)
    }

    #[test]
    fn matches_letters_in_order() {
        assert!(fuzzy_match("bfn", "buffer_next").is_some());
        assert!(fuzzy_match("nb", "buffer_next").is_none(), "wrong order");
        assert!(fuzzy_match("xyz", "buffer_next").is_none());
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(fuzzy_match("", "anything").unwrap().score, 0);
        assert!(fuzzy_match("", "").is_some());
    }

    #[test]
    fn reports_matched_positions() {
        let m = fuzzy_match("bn", "buffer_next").unwrap();
        assert_eq!(m.positions, vec![0, 7]);
    }

    #[test]
    fn smart_case() {
        assert!(fuzzy_match("readme", "README.md").is_some());
        assert!(fuzzy_match("Read", "readme").is_none());
        assert!(fuzzy_match("REA", "README").is_some());
    }

    #[test]
    fn word_starts_and_runs_rank_higher() {
        // "bn" = Buffer_Next beats a scattered match.
        assert!(score("bn", "buffer_next") > score("bn", "abandon"));
        // A consecutive run beats a spread-out one.
        assert!(score("wri", "write") > score("wri", "w_r_i"));
    }

    #[test]
    fn finds_the_best_start() {
        let m = fuzzy_match("next", "nope_next").unwrap();
        assert_eq!(m.positions, vec![5, 6, 7, 8]);
    }

    #[test]
    fn camel_case_counts_as_word_start() {
        assert!(score("fb", "fooBar") > score("fb", "fobar"));
    }
}
