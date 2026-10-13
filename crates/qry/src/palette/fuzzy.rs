//! The palette's fuzzy match: is the query a subsequence of the text, and
//! how well does it fit? A pure function, so tests and the fuzzer can call
//! it directly.

/// Where a query matched, and how well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    /// The matched characters, as char indices into the text, increasing.
    pub positions: Vec<usize>,
}

const MATCHED: i32 = 16;
const CONSECUTIVE: i32 = 15;
const WORD_START: i32 = 10;
const FIRST_CHAR: i32 = 8;
const GAP: i32 = 1;

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '·' | '_' | '.' | ':' | '-' | '/')
}

/// `c` lower-cased to one char (the first, where it would be several).
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The score of matching at `positions` in `text`.
fn score_of(text: &[char], positions: &[usize]) -> i32 {
    let mut score = 0;
    let mut previous: Option<usize> = None;
    for &p in positions {
        score += MATCHED;
        if p == 0 {
            score += FIRST_CHAR + WORD_START;
        } else if is_separator(text[p - 1]) {
            score += WORD_START;
        }
        match previous {
            Some(q) if p == q + 1 => score += CONSECUTIVE,
            Some(q) => score -= GAP * i32::try_from(p - q - 1).unwrap_or(i32::MAX / 2),
            None => {}
        }
        previous = Some(p);
    }
    score
}

/// Matches `query` against `text`, ignoring case: `None` unless every
/// character of the query appears in order. Of the ways it can match, the
/// best scoring is kept: consecutive characters, word starts and the first
/// character count for it, gaps against it. An empty query matches
/// everything with no positions.
pub fn score(query: &str, text: &str) -> Option<Match> {
    let query: Vec<char> = query.chars().map(lower).collect();
    if query.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    let original: Vec<char> = text.chars().collect();
    // Lower-cased one char at a time, so indices stay char indices.
    let lower: Vec<char> = original.iter().copied().map(self::lower).collect();

    let mut best: Option<Match> = None;
    // Every place the first character can match, then greedily on: word
    // starts are tried first for each next character, then the nearest.
    for start in (0..lower.len()).filter(|&i| lower[i] == query[0]) {
        for prefer_words in [true, false] {
            let mut positions = vec![start];
            let mut at = start + 1;
            let mut ok = true;
            for &q in &query[1..] {
                let next = (at..lower.len()).filter(|&i| lower[i] == q);
                let pick = if prefer_words && lower.get(at) != Some(&q) {
                    next.clone()
                        .find(|&i| is_separator(original[i - 1]))
                        .or_else(|| next.clone().next())
                } else {
                    next.clone().next()
                };
                match pick {
                    Some(i) => {
                        positions.push(i);
                        at = i + 1;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            let score = score_of(&original, &positions);
            if best.as_ref().is_none_or(|b| score > b.score) {
                best = Some(Match { score, positions });
            }
        }
    }
    best
}

#[cfg(test)]
#[path = "fuzzy_tests.rs"]
mod fuzzy_tests;
