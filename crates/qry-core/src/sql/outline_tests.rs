//! Unit tests for `outline.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;

/// What the outline must hold for `text`: a whole lex and split of it.
fn check(outline: &Outline, dialect: Dialect, text: &str) -> Result<(), TestCaseError> {
    prop_assert_eq!(outline.text(), text);
    let tokens = lex(dialect, text);
    prop_assert_eq!(outline.tokens(), tokens.as_slice());
    let spans: Vec<_> = split(dialect, text).into_iter().map(|s| s.span).collect();
    prop_assert_eq!(outline.statements(), spans.as_slice());
    // The boundaries too, so the next edit starts from the right ones.
    prop_assert_eq!(outline, &Outline::new(dialect, text.to_string()));
    Ok(())
}

/// `text` with the bytes from `at` (a fraction of its length) for `len`
/// replaced by `with`, on character boundaries.
fn edit(text: &str, at: f64, len: usize, with: &str) -> String {
    let mut start = (text.len() as f64 * at) as usize;
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + len).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    format!("{}{with}{}", &text[..start], &text[end..])
}

#[test]
fn an_edit_in_one_statement_keeps_the_others() {
    let mut outline = Outline::new(Dialect::Postgres, "select 1;\nselect 2;\nselect 3;".into());
    outline.update(Dialect::Postgres, "select 1;\nselect 22;\nselect 3;".into());
    assert_eq!(outline.statements(), [0..8, 10..19, 21..29]);
}

#[test]
fn opening_and_closing_a_string_relexes_to_the_end_and_back() {
    let text = "select 1;\nselect 2;\nselect 3;";
    let mut outline = Outline::new(Dialect::Postgres, text.into());
    outline.update(Dialect::Postgres, "select '1;\nselect 2;\nselect 3;".into());
    assert_eq!(outline.statements().len(), 1);
    assert_eq!(
        outline.tokens().last().unwrap().kind,
        TokenKind::Unterminated
    );
    outline.update(
        Dialect::Postgres,
        "select '1';\nselect 2;\nselect 3;".into(),
    );
    assert_eq!(outline.statements().len(), 3);
}

#[test]
fn a_delimiter_line_changes_what_follows() {
    let text = "select 1; select 2$$";
    let mut outline = Outline::new(Dialect::MySql, text.into());
    assert_eq!(outline.statements().len(), 2);
    let with = format!("DELIMITER $$\n{text}");
    outline.update(Dialect::MySql, with.clone());
    assert_eq!(outline, Outline::new(Dialect::MySql, with));
    assert_eq!(outline.statements().len(), 1);
}

#[test]
fn another_dialect_starts_again() {
    let mut outline = Outline::new(Dialect::Postgres, "select `a;b`".into());
    assert_eq!(outline.statements().len(), 2);
    outline.update(Dialect::MySql, "select `a;b`".into());
    assert_eq!(outline.statements().len(), 1);
    assert_eq!(outline.dialect(), Dialect::MySql);
}

#[test]
fn tokens_in_a_range_and_at_an_offset() {
    let outline = Outline::new(Dialect::Postgres, "select a,\n  b from t".into());
    let texts: Vec<_> = outline
        .tokens_in(10..13)
        .iter()
        .map(|t| &outline.text()[t.span.clone()])
        .collect();
    assert_eq!(texts, ["\n  ", "b"]);
    assert_eq!(outline.token_at(7).unwrap().span, 7..8);
    assert_eq!(outline.token_at(99), None);
}

#[test]
fn statement_at_matches_the_free_function() {
    let text = "select 1;\n\nselect 2; select 3";
    let outline = Outline::new(Dialect::Sqlite, text.into());
    let statements = split(Dialect::Sqlite, text);
    for offset in 0..=text.len() + 1 {
        assert_eq!(
            outline.statement_at(offset),
            statement_at(&statements, offset)
        );
    }
}

/// SQL-shaped text: every construct that spans lines or changes the
/// splitting, in every dialect.
const SQL: &str = "(select |from |begin |end|create |procedure |DELIMITER |/|\\$\\$|\\$a\\$|q'\\[|\\]'|\\[|\\]|\
     [a-z]{1,3}|[0-9.e]{1,3}|[ \\t]{1,3}|\\n|;|'|\"|`|/\\*|\\*/|--|#|\\\\|\\(|\\)|é){0,60}";

proptest! {
    #[test]
    fn prop_update_matches_a_whole_lex_and_split(
        text in SQL,
        edits in prop::collection::vec((0.0..=1.0f64, 0usize..12, SQL), 1..4),
    ) {
        for dialect in Dialect::ALL {
            let mut outline = Outline::new(dialect, text.clone());
            let mut now = text.clone();
            for (at, len, with) in &edits {
                // Short insertions, as typing makes, as well as long ones.
                let with: String = with.chars().take(1 + len % 7).collect();
                now = edit(&now, *at, *len, &with);
                outline.update(dialect, now.clone());
                check(&outline, dialect, &now)?;
            }
        }
    }

    #[test]
    fn prop_tokens_in_a_window_are_the_whole_lex_there(
        text in SQL,
        a in 0.0..=1.0f64,
        b in 0.0..=1.0f64,
    ) {
        for dialect in Dialect::ALL {
            let outline = Outline::new(dialect, text.clone());
            let (a, b) = (a.min(b), a.max(b));
            let window = (text.len() as f64 * a) as usize..(text.len() as f64 * b) as usize;
            let all = lex(dialect, &text);
            let want: Vec<_> = all
                .iter()
                .filter(|t| t.span.end > window.start && t.span.start < window.end)
                .cloned()
                .collect();
            if window.is_empty() {
                // An empty window is between tokens or inside one.
                prop_assert!(outline.tokens_in(window).len() <= 1);
            } else {
                prop_assert_eq!(outline.tokens_in(window), want.as_slice());
            }
        }
    }
}
