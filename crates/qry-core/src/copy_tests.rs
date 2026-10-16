//! Unit tests for `copy.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;
use crate::testing::{result, typed_result};

fn all(r: &QueryResult) -> Selection {
    Selection::all(r, (0..r.rows.len()).collect())
}

/// A small CSV reader, enough to read back what `render` writes.
fn read_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![vec![String::new()]];
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let field = rows.last_mut().unwrap().last_mut().unwrap();
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => rows.last_mut().unwrap().push(String::new()),
            ('\n', false) => rows.push(vec![String::new()]),
            (c, _) => field.push(c),
        }
    }
    rows
}

#[test]
fn csv_quotes_separator_and_newlines() {
    let r = result(
        &["a", "b"],
        &[&[Some("x,y"), Some("line\n2")], &[Some("say \"hi\""), None]],
    );
    let text = render(&r, &all(&r), CopyFormat::Csv, Dialect::Postgres, "t");
    assert_eq!(text, "a,b\n\"x,y\",\"line\n2\"\n\"say \"\"hi\"\"\",");
}

#[test]
fn tsv_escapes_tabs() {
    let r = result(&["a"], &[&[Some("x\ty")]]);
    let text = render(
        &r,
        &Selection::row(&r, 0),
        CopyFormat::Tsv,
        Dialect::Postgres,
        "t",
    );
    assert_eq!(text, "x\\ty");
}

#[test]
fn json_null_is_null_not_string() {
    let r = result(&["a", "b"], &[&[None, Some("NULL")]]);
    let text = render(&r, &all(&r), CopyFormat::Json, Dialect::Postgres, "t");
    assert_eq!(text, "[{\"a\": null, \"b\": \"NULL\"}]");
}

#[test]
fn markdown_escapes_pipes() {
    let r = result(&["a|b"], &[&[Some("x|y")]]);
    let text = render(&r, &all(&r), CopyFormat::Markdown, Dialect::Postgres, "t");
    assert_eq!(text, "| a\\|b |\n|---|\n| x\\|y |");
}

#[test]
fn insert_quotes_identifiers_and_literals_per_dialect() {
    let r = result(
        &["id", "my col", "select"],
        &[&[Some("1"), Some("it's"), Some("a\\b")]],
    );
    let pg = render(
        &r,
        &all(&r),
        CopyFormat::Insert,
        Dialect::Postgres,
        "my table",
    );
    assert_eq!(
        pg,
        "INSERT INTO \"my table\" (id, \"my col\", \"select\") VALUES ('1', 'it''s', 'a\\b');"
    );
    let my = render(&r, &all(&r), CopyFormat::Insert, Dialect::MySql, "t");
    assert_eq!(
        my,
        "INSERT INTO t (id, `my col`, `select`) VALUES ('1', 'it''s', 'a\\\\b');"
    );
}

#[test]
fn insert_null_and_numbers_unquoted() {
    let r = typed_result(
        &[("n", "int4"), ("d", "numeric"), ("s", "text")],
        &[&[Some("7"), None, Some("7")]],
    );
    let text = render(&r, &all(&r), CopyFormat::Insert, Dialect::Sqlite, "t");
    assert_eq!(text, "INSERT INTO t (n, d, s) VALUES (7, NULL, '7');");
}

#[test]
fn a_cell_is_just_its_text() {
    let r = result(&["a", "b"], &[&[Some("x"), Some("y")]]);
    assert_eq!(
        render(
            &r,
            &Selection::cell(0, 1),
            CopyFormat::Tsv,
            Dialect::Postgres,
            "t"
        ),
        "y"
    );
    assert_eq!(
        render(
            &r,
            &Selection::column(vec![0], 1),
            CopyFormat::Csv,
            Dialect::Postgres,
            "t"
        ),
        "b\ny"
    );
}

proptest! {
    #[test]
    fn prop_csv_round_trips_through_csv_reader(
        cells in prop::collection::vec(prop::collection::vec("[a-z,\"\n ]{0,6}", 3), 1..10)
    ) {
        let mut r = result(&["a", "b", "c"], &[]);
        r.rows = cells.iter().map(|row| row.iter().cloned().map(Some).collect()).collect();
        let text = render(&r, &all(&r), CopyFormat::Csv, Dialect::Postgres, "t");
        let read = read_csv(&text);
        prop_assert_eq!(&read[0], &vec!["a".to_string(), "b".into(), "c".into()]);
        prop_assert_eq!(&read[1..], &cells[..]);
    }
}
