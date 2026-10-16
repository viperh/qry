//! Unit tests for `exporter.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use super::*;

/// A path of its own per test, in a directory removed when the guard is
/// dropped.
struct TempFile {
    _dir: tempfile::TempDir,
    path: String,
}

impl TempFile {
    fn new(extension: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("rows.{extension}"));
        Self {
            path: path.display().to_string(),
            _dir: dir,
        }
    }

    fn path(&self) -> String {
        self.path.clone()
    }
}

fn result() -> QueryResult {
    QueryResult {
        columns: vec!["id".into(), "note".into()],
        rows: vec![
            vec![Some("1".into()), Some("plain".into())],
            vec![Some("2".into()), Some("has, comma and \"quotes\"".into())],
            vec![Some("3".into()), None],
        ],
        affected: Some(3),
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    }
}

fn config(path: &str, separator: &str, etype: ExportType) -> ExportConfig {
    ExportConfig {
        path: path.into(),
        separator: separator.into(),
        etype,
    }
}

#[test]
fn csv_has_a_header_quotes_what_it_must_and_writes_null_as_empty() {
    let file = TempFile::new("csv");
    let config = config(&file.path(), ",", ExportType::Csv);
    let rows = Exporter::from_result(&result(), &config)
        .write(config.etype)
        .unwrap();

    assert_eq!(rows, 3);
    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        "id,note\n1,plain\n2,\"has, comma and \"\"quotes\"\"\"\n3,\n"
    );
}

#[test]
fn text_separates_with_tabs_and_keeps_rows_on_one_line() {
    let file = TempFile::new("txt");
    let mut result = result();
    result.rows[0][1] = Some("two\nlines".into());
    let config = config(&file.path(), "\t", ExportType::Text);
    Exporter::from_result(&result, &config)
        .write(config.etype)
        .unwrap();

    let written = fs::read_to_string(file.path()).unwrap();
    assert_eq!(written.lines().count(), 4);
    assert!(written.starts_with("id\tnote\n1\ttwo lines\n"), "{written}");
}

#[test]
fn json_writes_one_object_per_row_with_null_and_escapes() {
    let file = TempFile::new("json");
    let mut result = result();
    result.rows[0][1] = Some("tab\there \"quoted\" and \\ back".into());
    let config = config(&file.path(), "", ExportType::Json);
    let rows = Exporter::from_result(&result, &config)
        .write(config.etype)
        .unwrap();

    assert_eq!(rows, 3);
    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        concat!(
            "[\n",
            "  {\"id\": \"1\", \"note\": \"tab\\there \\\"quoted\\\" and \\\\ back\"},\n",
            "  {\"id\": \"2\", \"note\": \"has, comma and \\\"quotes\\\"\"},\n",
            "  {\"id\": \"3\", \"note\": null}\n",
            "]\n"
        )
    );
}

#[test]
fn excel_writes_a_workbook() {
    let file = TempFile::new("xlsx");
    let config = config(&file.path(), "", ExportType::Excel);
    let rows = Exporter::from_result(&result(), &config)
        .write(config.etype)
        .unwrap();

    assert_eq!(rows, 3);
    // An .xlsx is a zip archive, so it starts with the zip magic bytes.
    assert_eq!(&fs::read(file.path()).unwrap()[..2], b"PK");
}

#[test]
fn a_path_that_cannot_be_written_is_an_error() {
    let config = config("no/such/directory/rows.csv", ",", ExportType::Csv);
    let err = Exporter::from_result(&result(), &config)
        .write(config.etype)
        .unwrap_err();
    assert!(err.to_string().contains("could not write"), "{err}");
}
