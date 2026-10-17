//! Unit tests for `lex.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::{keywords, *};

/// The tokens that are not whitespace, as `(kind, text)`.
fn tokens(dialect: Dialect, src: &str) -> Vec<(TokenKind, &str)> {
    lex(dialect, src)
        .into_iter()
        .filter(|t| t.kind != TokenKind::Whitespace)
        .map(|t| (t.kind, &src[t.span]))
        .collect()
}

fn kinds(dialect: Dialect, src: &str) -> Vec<TokenKind> {
    tokens(dialect, src).into_iter().map(|(k, _)| k).collect()
}

use TokenKind as K;

#[test]
fn keyword_lists_are_sorted_for_the_binary_search() {
    for list in keywords::lists() {
        assert!(list.windows(2).all(|w| w[0] < w[1]), "{list:?}");
    }
}

#[test]
fn keywords_case_insensitive() {
    assert_eq!(
        kinds(Dialect::Postgres, "select SeLeCt SELECT"),
        [K::Keyword; 3]
    );
    assert_eq!(kinds(Dialect::Sqlite, "pragma"), [K::Keyword]);
    assert_eq!(
        kinds(Dialect::Postgres, "pragma"),
        [K::Ident],
        "SQLite's only"
    );
}

#[test]
fn identifier_vs_keyword_boundary() {
    assert_eq!(
        tokens(Dialect::Postgres, "selected from_date select_1"),
        [
            (K::Ident, "selected"),
            (K::Ident, "from_date"),
            (K::Ident, "select_1")
        ]
    );
}

#[test]
fn quoted_identifiers_per_dialect() {
    assert_eq!(
        tokens(Dialect::Postgres, r#""my col""#),
        [(K::QuotedIdent, r#""my col""#)]
    );
    assert_eq!(
        tokens(Dialect::Oracle, r#""A""B""#),
        [(K::QuotedIdent, r#""A""B""#)]
    );
    assert_eq!(tokens(Dialect::MySql, "`t`"), [(K::QuotedIdent, "`t`")]);
    assert_eq!(
        tokens(Dialect::Sqlite, "[a b] `c`"),
        [(K::QuotedIdent, "[a b]"), (K::QuotedIdent, "`c`")]
    );
    assert_eq!(
        kinds(Dialect::Postgres, "[1]"),
        [K::Punct, K::Number, K::Punct],
        "arrays"
    );
}

#[test]
fn single_quote_doubling() {
    assert_eq!(
        tokens(Dialect::Postgres, "'it''s' x"),
        [(K::String, "'it''s'"), (K::Ident, "x")]
    );
}

#[test]
fn postgres_e_string_backslash() {
    assert_eq!(
        tokens(Dialect::Postgres, r"E'a\'b' c"),
        [(K::String, r"E'a\'b'"), (K::Ident, "c")]
    );
    // Without the E a backslash is just a character.
    assert_eq!(
        tokens(Dialect::Postgres, r"'a\' c"),
        [(K::String, r"'a\'"), (K::Ident, "c")]
    );
}

#[test]
fn dollar_quoted_with_tag_and_nested_dollar() {
    let src = "$fn$ select '$$'; $x$ $fn$ ;";
    assert_eq!(
        tokens(Dialect::Postgres, src),
        [
            (K::String, "$fn$ select '$$'; $x$ $fn$"),
            (K::Delimiter, ";")
        ]
    );
    assert_eq!(tokens(Dialect::Postgres, "$$a$$"), [(K::String, "$$a$$")]);
    assert_eq!(kinds(Dialect::Postgres, "$1"), [K::Operator], "a parameter");
}

#[test]
fn oracle_q_quote_all_delimiters() {
    for src in [
        "q'[it's]'",
        "q'{it's}'",
        "q'<it's>'",
        "q'(it's)'",
        "q'!it's!'",
        "Q'#a#'",
    ] {
        assert_eq!(tokens(Dialect::Oracle, src), [(K::String, src)], "{src}");
    }
    assert_eq!(
        kinds(Dialect::Postgres, "q'a'"),
        [K::Ident, K::String],
        "Oracle only"
    );
}

#[test]
fn mysql_double_quote_is_string() {
    assert_eq!(
        tokens(Dialect::MySql, r#""a\"b""#),
        [(K::String, r#""a\"b""#)]
    );
    assert_eq!(kinds(Dialect::Postgres, r#""a""#), [K::QuotedIdent]);
}

#[test]
fn line_and_block_comments() {
    assert_eq!(
        tokens(Dialect::Sqlite, "a -- note\n/* b */ c"),
        [
            (K::Ident, "a"),
            (K::Comment, "-- note"),
            (K::Comment, "/* b */"),
            (K::Ident, "c")
        ]
    );
}

#[test]
fn postgres_nested_block_comment() {
    let src = "/* a /* b */ c */ d";
    assert_eq!(
        tokens(Dialect::Postgres, src)[0],
        (K::Comment, "/* a /* b */ c */")
    );
    // Elsewhere the first */ ends it.
    assert_eq!(
        tokens(Dialect::Sqlite, src)[0],
        (K::Comment, "/* a /* b */")
    );
}

#[test]
fn mysql_hash_comment() {
    assert_eq!(
        tokens(Dialect::MySql, "# hi\nx"),
        [(K::Comment, "# hi"), (K::Ident, "x")]
    );
    assert_ne!(kinds(Dialect::Postgres, "# hi")[0], K::Comment);
}

#[test]
fn numbers_hex_exponent_decimal() {
    for n in ["42", "4.5", ".5", "1e10", "2.5E-3", "0x1F"] {
        assert_eq!(tokens(Dialect::Postgres, n), [(K::Number, n)], "{n}");
    }
    assert_eq!(kinds(Dialect::Postgres, "1e"), [K::Number, K::Ident]);
}

#[test]
fn unterminated_string_runs_to_eof() {
    assert_eq!(
        tokens(Dialect::Postgres, "select 'abc; x"),
        [(K::Keyword, "select"), (K::Unterminated, "'abc; x")]
    );
    assert_eq!(
        tokens(Dialect::Sqlite, "/* open"),
        [(K::Unterminated, "/* open")]
    );
    assert_eq!(
        tokens(Dialect::Postgres, "$$ open"),
        [(K::Unterminated, "$$ open")]
    );
}

#[test]
fn spans_tile_input_exactly() {
    let src = "SELECT é, \"ü\" FROM t -- 日本\nWHERE x >= 1.5e3; /* end";
    for dialect in Dialect::ALL {
        let tokens = lex(dialect, src);
        let mut at = 0;
        for t in &tokens {
            assert_eq!(t.span.start, at, "{dialect:?}");
            assert!(t.span.end > t.span.start);
            at = t.span.end;
        }
        assert_eq!(at, src.len());
    }
}

proptest! {
    #[test]
    fn prop_lex_never_panics_and_tiles(src in ".{0,2000}") {
        for dialect in Dialect::ALL {
            let mut at = 0;
            for t in lex(dialect, &src) {
                prop_assert_eq!(t.span.start, at);
                prop_assert!(t.span.end > t.span.start);
                prop_assert!(src.is_char_boundary(t.span.end));
                at = t.span.end;
            }
            prop_assert_eq!(at, src.len());
        }
    }

    #[test]
    fn prop_lex_tiles_sql_shaped_text(src in "[a-z '\"`$;/*\\-\\n\\[\\]()0-9.#qE]{0,300}") {
        for dialect in Dialect::ALL {
            let end = lex(dialect, &src).last().map_or(0, |t| t.span.end);
            prop_assert_eq!(end, src.len());
        }
    }
}
