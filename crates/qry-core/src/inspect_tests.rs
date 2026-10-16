//! Unit tests for `inspect.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;
use crate::testing::result;

/// A one-column result holding `cells`.
fn column(cells: &[Option<&str>]) -> QueryResult {
    let rows: Vec<&[Option<&str>]> = cells.iter().map(std::slice::from_ref).collect();
    result(&["c"], &rows)
}

/// Every token's text on each line, joined.
fn json_text(rendered: &Rendered) -> Vec<String> {
    match rendered {
        Rendered::Json(lines) => lines
            .iter()
            .map(|line| line.iter().map(|(_, t)| t.as_str()).collect())
            .collect(),
        other => panic!("not JSON: {other:?}"),
    }
}

#[test]
fn profile_counts_nulls_and_distinct() {
    let r = column(&[Some("a"), None, Some("b"), Some("a"), None]);
    let mut p = Profiler::new(0, 100);
    let profile = p.step(&r, 100).unwrap();
    assert_eq!(profile.nulls, 2);
    assert_eq!(profile.distinct, 2);
    assert_eq!(profile.sampled, 5);
    assert!(profile.complete);
    assert_eq!(p.column(), 0);
}

#[test]
fn profile_top_values_sorted_by_count_then_value() {
    let cells: Vec<Option<&str>> = ["c", "b", "a", "b", "c", "d", "e", "f", "g", "c"]
        .into_iter()
        .map(Some)
        .collect();
    let mut p = Profiler::new(0, 100);
    let top = p.step(&column(&cells), 100).unwrap().top.clone();
    let expected: Vec<(String, u64)> = [("c", 3), ("b", 2), ("a", 1), ("d", 1), ("e", 1)]
        .into_iter()
        .map(|(v, n)| (v.to_string(), n))
        .collect();
    assert_eq!(top, expected, "five at most, NULLs never among them");
}

#[test]
fn profile_step_respects_budget() {
    let r = crate::testing::grid(25, 2);
    let mut p = Profiler::new(1, 100);
    assert!(p.step(&r, 10).is_none());
    assert_eq!(p.profile().sampled, 10);
    assert!(p.step(&r, 10).is_none());
    assert_eq!(p.profile().sampled, 20);
    let done = p.step(&r, 10).unwrap();
    assert_eq!((done.sampled, done.distinct), (25, 25));
    assert_eq!(p.steps(), 3);
    // Done stays done.
    assert!(p.step(&r, 10).is_some());
}

#[test]
fn profile_cap_marks_sampled() {
    let r = crate::testing::grid(10, 1);
    let mut p = Profiler::new(0, 4);
    let profile = p.step(&r, 100).unwrap();
    assert!(profile.complete);
    assert_eq!(profile.sampled, 4, "fewer than the 10 rows");
    assert_eq!(profile.distinct, 4);
}

#[test]
fn profile_of_an_empty_result_is_complete() {
    let mut p = Profiler::new(0, 10);
    let profile = p.step(&column(&[]), 10).unwrap();
    assert_eq!(
        profile,
        &ColumnProfile {
            complete: true,
            ..ColumnProfile::default()
        }
    );
}

#[test]
fn render_json_pretty_two_space() {
    let raw = r#"{"a":1,"b":[true,null,"x"],"c":{},"d":-1.5e3}"#;
    let rendered = render_value(Some(raw), ValueKind::Text, 80, true, true);
    assert_eq!(
        json_text(&rendered),
        [
            "{",
            r#"  "a": 1,"#,
            r#"  "b": ["#,
            "    true,",
            "    null,",
            r#"    "x""#,
            "  ],",
            r#"  "c": {},"#,
            r#"  "d": -1.5e3"#,
            "}",
        ]
    );
    let Rendered::Json(lines) = rendered else {
        unreachable!()
    };
    assert_eq!(lines[1][1], (JsonTok::Key, r#""a""#.into()));
    assert_eq!(lines[1][3], (JsonTok::Number, "1".into()));
    assert_eq!(lines[3][1], (JsonTok::Literal, "true".into()));
    assert_eq!(lines[5][1], (JsonTok::String, r#""x""#.into()));
    // Raw shows the text as it is.
    assert_eq!(
        render_value(Some(raw), ValueKind::Json, 80, true, false),
        Rendered::Text(vec![raw.into()])
    );
    // A JSON column's scalar is JSON too; text's is not.
    assert!(matches!(
        render_value(Some("42"), ValueKind::Json, 80, true, true),
        Rendered::Json(_)
    ));
    assert!(matches!(
        render_value(Some("42"), ValueKind::Text, 80, true, true),
        Rendered::Text(_)
    ));
}

#[test]
fn render_invalid_json_falls_back_to_text() {
    for raw in [
        "{not json",
        "{\"a\":}",
        "[1,]",
        "{\"a\":1} trailing",
        "[01]",
    ] {
        assert_eq!(
            render_value(Some(raw), ValueKind::Json, 80, true, true),
            Rendered::Text(vec![raw.into()]),
            "{raw}"
        );
    }
    let deep = "[".repeat(10_000) + &"]".repeat(10_000);
    assert!(matches!(
        render_value(Some(&deep), ValueKind::Json, 80, false, true),
        Rendered::Text(_)
    ));
}

#[test]
fn hex_dump_offsets_and_ascii_column() {
    let lines = hex_dump(b"Hello world.\n\x00\x01\x02Hello");
    assert_eq!(
        lines,
        [
            "00000000  48 65 6c 6c 6f 20 77 6f  72 6c 64 2e 0a 00 01 02  |Hello world.....|",
            "00000010  48 65 6c 6c 6f                                    |Hello|",
        ]
    );
    assert!(hex_dump(&[]).is_empty());
    // A backend's hex text is decoded first.
    assert_eq!(binary_bytes("x'4869'"), b"Hi");
    assert_eq!(binary_bytes("\\x4869"), b"Hi");
    assert_eq!(binary_bytes("0x4869"), b"Hi");
    assert_eq!(binary_bytes("x'zz'"), b"x'zz'");
    assert!(matches!(
        render_value(Some("x'4869'"), ValueKind::Binary, 80, true, true),
        Rendered::Hex(lines) if lines[0].ends_with("|Hi|")
    ));
}

#[test]
fn wrap_words_respects_unicode_width() {
    // Each CJK character takes two cells.
    assert_eq!(wrap_words("日本 語語 x", 6), ["日本", "語語 x"]);
    assert_eq!(wrap_words("a  b\t c", 80), ["a b c"]);
    assert_eq!(wrap_words("", 10), [""]);
}

#[test]
fn wrap_words_breaks_overlong_word() {
    assert_eq!(wrap_words("ab abcdefgh c", 4), ["ab", "abcd", "efgh", "c"]);
    assert_eq!(wrap_words("日本語", 3), ["日", "本", "語"]);
}

#[test]
fn render_null_is_null_variant() {
    for kind in [ValueKind::Text, ValueKind::Json, ValueKind::Binary] {
        assert_eq!(render_value(None, kind, 10, true, true), Rendered::Null);
    }
    assert_eq!(Rendered::Null.len(), 1);
}

#[test]
fn render_text_keeps_lines_and_shows_control_characters() {
    assert_eq!(
        render_value(Some("one\r\ntwo\tx\u{7}"), ValueKind::Text, 80, false, true),
        Rendered::Text(vec!["one".into(), "two    x·".into()])
    );
    assert_eq!(
        render_value(Some("aa bb cc"), ValueKind::Text, 5, true, true),
        Rendered::Text(vec!["aa bb".into(), "cc".into()])
    );
    assert_eq!(
        render_value(Some("aa bb cc"), ValueKind::Text, 5, false, true),
        Rendered::Text(vec!["aa bb cc".into()])
    );
}

/// Every kind there is, for the fuzz twin.
fn kinds() -> [ValueKind; 11] {
    [
        ValueKind::Integer,
        ValueKind::Decimal,
        ValueKind::Float,
        ValueKind::Text,
        ValueKind::Bool,
        ValueKind::Date,
        ValueKind::Time,
        ValueKind::Timestamp,
        ValueKind::Json,
        ValueKind::Binary,
        ValueKind::Other,
    ]
}

proptest! {
    #[test]
    fn prop_wrap_words_never_exceeds_width_and_preserves_text(
        text in "[a-z日 \t\n]{0,80}",
        width in 2usize..30,
    ) {
        let lines = wrap_words(&text, width);
        for line in &lines {
            prop_assert!(grid::width(line) <= width, "{line:?} wider than {width}");
        }
        let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        prop_assert_eq!(squeeze(&lines.concat()), squeeze(&text));
    }

    #[test]
    fn prop_hex_dump_line_count(bytes in prop::collection::vec(any::<u8>(), 0..200)) {
        let lines = hex_dump(&bytes);
        prop_assert_eq!(lines.len(), bytes.len().div_ceil(16));
        for line in &lines {
            prop_assert!(line.len() <= 79);
        }
    }

    /// The twin of the `render_value` fuzz target: any bytes, as any kind,
    /// at any width, never panic.
    #[test]
    fn fuzz_render_value(
        bytes in prop::collection::vec(any::<u8>(), 0..300),
        kind in 0usize..11,
        width in 0u16..200,
        wrap: bool,
        pretty: bool,
    ) {
        let text = String::from_utf8_lossy(&bytes);
        let rendered = render_value(Some(&text), kinds()[kind], width, wrap, pretty);
        if let Rendered::Text(lines) = rendered && wrap && width > 1 {
            prop_assert!(!lines.is_empty());
        }
    }
}
