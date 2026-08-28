//! Minimal fuzzy matcher (subsequence with a scoring heuristic).
//!
//! Returns `(score, matched_char_indices)` where the indices refer to
//! character positions in `target` — used by the UI to highlight the
//! matched characters. Case-insensitive.

/// Score > 0 means better. Ties are broken by tab position upstream.
pub fn fuzzy_match(query: &str, target: &str) -> Option<(i64, Vec<usize>)> {
    let query: Vec<char> = lower_chars(query);
    if query.is_empty() {
        return Some((0, vec![]));
    }

    let lower: Vec<char> = lower_chars(target);
    // NOTE: lower_chars keeps a 1:1 mapping with the input string so the
    // returned indices line up with the original.

    let mut score: i64 = 0;
    let mut indices: Vec<usize> = Vec::new();
    let mut prev_match: Option<usize> = None;
    let mut qi = 0usize;

    for (ti, &lc) in lower.iter().enumerate() {
        if qi < query.len() && lc == query[qi] {
            let mut s = 1i64;
            // consecutive characters
            if prev_match == Some(ti.saturating_sub(1)) {
                s += 8;
            }
            // word starts (after a separator or at the beginning)
            if ti == 0 || is_sep(lower[ti - 1]) {
                s += 10;
            }
            // penalize gaps between matched characters
            if let Some(p) = prev_match {
                s -= (ti - p - 1).min(8) as i64;
            }
            score += s;
            indices.push(ti);
            prev_match = Some(ti);
            qi += 1;
        }
    }

    if qi == query.len() {
        Some((score, indices))
    } else {
        None
    }
}

fn lower_chars(s: &str) -> Vec<char> {
    // to_lowercase() can expand (e.g. 'ß' -> "ss"); we keep only the first
    // char to preserve index alignment.
    s.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

fn is_sep(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '/' | '.' | ':' | '~' | '=' | ',')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_subsequence() {
        let (score, idx) = fuzzy_match("srv", "server").unwrap();
        assert!(score > 0);
        assert_eq!(idx, vec![0, 2, 3]); // s(0) e r(2) v(3) e r
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("SRV", "server").is_some());
    }

    #[test]
    fn no_match() {
        assert!(fuzzy_match("xyz", "server").is_none());
    }

    #[test]
    fn prefers_word_starts() {
        let (start, _) = fuzzy_match("sb", "some-build").unwrap();
        let (mid, _) = fuzzy_match("oi", "some-build").unwrap();
        assert!(start > mid);
    }
}
