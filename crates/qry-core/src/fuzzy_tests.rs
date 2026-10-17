//! Unit tests for `fuzzy.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;

#[test]
fn subsequence_required() {
    assert!(score("xpt", "Export").is_some());
    assert!(score("zz", "Export").is_none());
    assert!(score("tpx", "Export").is_none(), "order matters");
}

#[test]
fn case_insensitive() {
    assert_eq!(
        score("EXP", "export").unwrap().positions,
        score("exp", "EXPORT").unwrap().positions
    );
}

#[test]
fn word_start_beats_middle() {
    let export = score("ec", "Export connections").unwrap();
    let select = score("ec", "Select columns").unwrap();
    assert!(export.score > select.score, "{export:?} vs {select:?}");
    assert_eq!(export.positions, [0, 7]);
}

#[test]
fn consecutive_beats_scattered() {
    let together = score("exp", "Export results").unwrap();
    let apart = score("exp", "Edit the xml parser").unwrap();
    assert!(together.score > apart.score);
    assert_eq!(together.positions, [0, 1, 2]);
}

#[test]
fn positions_are_char_indices_for_unicode_labels() {
    let m = score("ré", "Éditer · résultats").unwrap();
    let text: Vec<char> = "Éditer · résultats".chars().collect();
    assert_eq!(m.positions, [9, 10]);
    assert_eq!((text[9], text[10]), ('r', 'é'));
}

#[test]
fn prefix_beats_scattered() {
    let prefix = score("sel", "SELECT * FROM users").unwrap();
    let scattered = score("sel", "SHOW TABLES WHERE name LIKE 'x'").unwrap();
    assert!(
        prefix.score > scattered.score,
        "{prefix:?} vs {scattered:?}"
    );
    assert_eq!(prefix.positions, [0, 1, 2]);
}

#[test]
fn no_match_is_none() {
    assert_eq!(score("xyz", "SELECT 1"), None);
    assert_eq!(score("a", ""), None);
}

#[test]
fn match_positions_are_char_indices_not_bytes() {
    // `é` is two bytes: a byte index would put `u` at 2.
    let m = score("u", "éu").unwrap();
    assert_eq!(m.positions, [1]);
}

#[test]
fn empty_query_matches_everything() {
    assert_eq!(
        score("", "anything").unwrap().positions,
        Vec::<usize>::new()
    );
}

proptest! {
    /// The fuzz twin: any query and text, no panic, and the positions say
    /// where the query's characters are, in order.
    #[test]
    fn score_never_panics_and_positions_are_increasing_and_in_bounds(
        query in "\\PC{0,8}",
        text in "\\PC{0,40}",
    ) {
        if let Some(m) = score(&query, &text) {
            let len = text.chars().count();
            prop_assert!(m.positions.windows(2).all(|w| w[0] < w[1]));
            prop_assert!(m.positions.iter().all(|&p| p < len));
            prop_assert_eq!(m.positions.len(), query.chars().count());
        }
    }
}
