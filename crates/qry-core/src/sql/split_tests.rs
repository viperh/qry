//! Unit tests for `split.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;

fn texts(dialect: Dialect, src: &str) -> Vec<String> {
    split(dialect, src).into_iter().map(|s| s.text).collect()
}

#[test]
fn splits_on_semicolons() {
    assert_eq!(
        texts(Dialect::Sqlite, "select 1; select 2;"),
        ["select 1", "select 2"]
    );
}

#[test]
fn ignores_semicolon_in_string_and_comment() {
    assert_eq!(
        texts(
            Dialect::Postgres,
            "select ';' -- a;b\n, 2; select /* ; */ 3"
        ),
        ["select ';' -- a;b\n, 2", "select /* ; */ 3"]
    );
}

#[test]
fn drops_empty_and_comment_only() {
    assert_eq!(
        texts(Dialect::Sqlite, " ; -- nothing\n; /* x */ ; select 1"),
        ["select 1"]
    );
    assert!(split(Dialect::Sqlite, "   \n").is_empty());
}

#[test]
fn trailing_statement_without_semicolon() {
    assert_eq!(
        texts(Dialect::MySql, "select 1;\nselect 2\n"),
        ["select 1", "select 2"]
    );
}

#[test]
fn spans_map_to_original_text() {
    let src = "-- head\nselect 1;\n\n  select 2 ;";
    for s in split(Dialect::Postgres, src) {
        assert_eq!(&src[s.span.clone()], s.text);
    }
    assert_eq!(split(Dialect::Postgres, src)[1].span, 21..29);
}

#[test]
fn dollar_quoted_function_body_is_one_statement() {
    let src = "create function f() returns int as $$ begin; return 1; end $$ language plpgsql; select f();";
    let parts = texts(Dialect::Postgres, src);
    assert_eq!(parts.len(), 2, "{parts:?}");
    assert!(parts[0].ends_with("language plpgsql"));
}

#[test]
fn oracle_plsql_block_until_slash() {
    let src = "begin\n  null;\n  dbms_output.put_line('x;');\nend;\n/\nselect 1 from dual;\n";
    let parts = texts(Dialect::Oracle, src);
    assert_eq!(parts.len(), 2, "{parts:?}");
    assert!(parts[0].starts_with("begin") && parts[0].ends_with("end;"));
    assert_eq!(parts[1], "select 1 from dual");
    // A plain statement ends at a slash line too.
    assert_eq!(
        texts(Dialect::Oracle, "select 1 from dual\n/\n"),
        ["select 1 from dual"]
    );
}

#[test]
fn oracle_create_procedure_not_split_inside() {
    let src = "create or replace procedure p as\nbegin\n  insert into t values (1);\nend;\n/\n";
    assert_eq!(split(Dialect::Oracle, src).len(), 1);
    // Elsewhere the same text splits on each `;`.
    assert!(split(Dialect::Postgres, src).len() > 1);
}

#[test]
fn mysql_delimiter_changes_terminator_and_is_stripped() {
    let src = "DELIMITER $$\ncreate procedure p() begin select 1; select 2; end$$\nDELIMITER ;\nselect 3;";
    let parts = texts(Dialect::MySql, src);
    assert_eq!(parts.len(), 2, "{parts:?}");
    assert!(parts[0].starts_with("create procedure") && parts[0].ends_with("end"));
    assert_eq!(parts[1], "select 3");
    assert!(!parts.iter().any(|p| p.contains("DELIMITER")));
}

#[test]
fn statement_at_cursor_inside() {
    let stmts = split(Dialect::Sqlite, "select 1;\nselect 2;");
    assert_eq!(statement_at(&stmts, 3), Some(0));
    assert_eq!(
        statement_at(&stmts, 8),
        Some(0),
        "right after it, on the `;`"
    );
    assert_eq!(statement_at(&stmts, 9), Some(0), "after the `;`");
    assert_eq!(statement_at(&stmts, 12), Some(1));
}

#[test]
fn statement_at_blank_line_picks_next() {
    let src = "select 1;\n\n\nselect 2;";
    let stmts = split(Dialect::Sqlite, src);
    assert_eq!(statement_at(&stmts, 10), Some(1));
}

#[test]
fn statement_at_eof_picks_previous() {
    let src = "select 1;\nselect 2;\n\n";
    let stmts = split(Dialect::Sqlite, src);
    assert_eq!(statement_at(&stmts, src.len()), Some(1));
    assert_eq!(statement_at(&[], 0), None);
}

#[test]
fn is_ddl_true_for_create_alter_drop_truncate() {
    for sql in [
        "create table t (a int)",
        "ALTER TABLE t ADD b int",
        "-- x\n drop table t",
        "truncate t",
    ] {
        assert!(is_ddl(Dialect::Postgres, sql), "{sql}");
    }
}

#[test]
fn is_ddl_false_for_select_with_drop_in_string() {
    assert!(!is_ddl(Dialect::Postgres, "select 'drop table t'"));
    assert!(!is_ddl(Dialect::Postgres, "insert into drops values (1)"));
}

/// The bytes outside comments and whitespace (an unterminated string's
/// trailing blanks included).
fn significant(dialect: Dialect, src: &str) -> Vec<usize> {
    lex(dialect, src)
        .into_iter()
        .filter(|t| {
            !matches!(
                t.kind,
                TokenKind::Whitespace | TokenKind::Comment | TokenKind::Delimiter
            )
        })
        .flat_map(|t| {
            let end = if t.kind == TokenKind::Unterminated {
                t.span.start + src[t.span.clone()].trim_end().len()
            } else {
                t.span.end
            };
            t.span.start..end
        })
        .collect()
}

proptest! {
    /// Every byte that is not trivia or a `;` is in some statement.
    #[test]
    fn prop_split_concat_covers_all_non_trivia(src in "[a-z '\";\\-\\n/*()0-9]{0,200}") {
        for dialect in [Dialect::Postgres, Dialect::Sqlite] {
            let stmts = split(dialect, &src);
            for byte in significant(dialect, &src) {
                prop_assert!(
                    stmts.iter().any(|s| s.span.contains(&byte)),
                    "{dialect:?}: byte {byte} of {src:?} in no statement: {stmts:?}"
                );
            }
        }
    }

    #[test]
    fn prop_split_is_stable_under_trailing_whitespace(src in "[a-z '\";\\-\\n]{0,200}", pad in "[ \\n\\t]{0,5}") {
        for dialect in Dialect::ALL {
            let padded = format!("{src}{pad}");
            let a: Vec<_> = split(dialect, &src).into_iter().map(|s| s.text).collect();
            let b: Vec<_> = split(dialect, &padded).into_iter().map(|s| s.text).collect();
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn prop_split_never_panics(src in ".{0,500}") {
        for dialect in Dialect::ALL {
            for s in split(dialect, &src) {
                prop_assert_eq!(&src[s.span.clone()], s.text);
            }
        }
    }
}
