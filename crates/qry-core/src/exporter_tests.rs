//! Unit tests for `exporter.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use calamine::{Data, Reader, Xlsx, open_workbook};
use proptest::prelude::*;

use super::*;

/// A path of its own per test, in a directory removed when the guard is
/// dropped.
struct TempFile {
    dir: tempfile::TempDir,
    path: PathBuf,
}

impl TempFile {
    fn new(extension: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("rows.{extension}"));
        Self { path, dir }
    }

    fn read(&self) -> String {
        fs::read_to_string(&self.path).unwrap()
    }

    /// Every file in the directory, temporary ones included.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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

/// A result whose columns say what they hold.
fn typed() -> QueryResult {
    QueryResult {
        columns: vec![
            ColumnMeta::new("id", "int8"),
            ColumnMeta::new("price", "numeric"),
            ColumnMeta::new("name", "text"),
            ColumnMeta::new("active", "bool"),
            ColumnMeta::new("born", "date"),
        ],
        rows: vec![
            vec![
                Some("1".into()),
                Some("9.50".into()),
                Some("ann".into()),
                Some("true".into()),
                Some("2001-02-03".into()),
            ],
            vec![
                Some("2".into()),
                None,
                Some("o'brien".into()),
                Some("false".into()),
                None,
            ],
        ],
        affected: Some(2),
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    }
}

fn write(result: &QueryResult, etype: ExportType, opts: &ExportOptions) -> (TempFile, Written) {
    let file = TempFile::new(etype.extension());
    let written = write_result(result, etype, &file.path, opts).unwrap();
    (file, written)
}

fn csv() -> ExportOptions {
    ExportOptions::default()
}

// --- ported from the single-shot exporter -------------------------------

#[test]
fn csv_has_a_header_quotes_what_it_must_and_writes_null_as_empty() {
    let (file, written) = write(&result(), ExportType::Csv, &csv());
    assert_eq!(written.rows, 3);
    assert_eq!(
        file.read(),
        "id,note\n1,plain\n2,\"has, comma and \"\"quotes\"\"\"\n3,\n"
    );
}

#[test]
fn text_separates_with_tabs_and_keeps_rows_on_one_line() {
    let mut result = result();
    result.rows[0][1] = Some("two\nlines".into());
    let config = ExportConfig::new("rows", "\t", ExportType::Text);
    let (file, _) = write(
        &result,
        ExportType::Text,
        &ExportOptions::from_config(&config),
    );

    let written = file.read();
    assert_eq!(written.lines().count(), 4);
    assert!(written.starts_with("id\tnote\n1\ttwo lines\n"), "{written}");
}

#[test]
fn json_writes_one_object_per_row_with_null_and_escapes() {
    let mut result = result();
    result.rows[0][1] = Some("tab\there \"quoted\" and \\ back".into());
    let (file, written) = write(&result, ExportType::Json, &csv());

    assert_eq!(written.rows, 3);
    assert_eq!(
        file.read(),
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
    let (file, written) = write(&result(), ExportType::Excel, &csv());
    assert_eq!(written.rows, 3);
    // An .xlsx is a zip archive, so it starts with the zip magic bytes.
    assert_eq!(&fs::read(&file.path).unwrap()[..2], b"PK");
}

#[test]
fn a_path_that_cannot_be_written_is_an_error() {
    let err = write_result(
        &result(),
        ExportType::Csv,
        Path::new("no/such/directory/rows.csv"),
        &csv(),
    )
    .unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains("could not write"), "{message}");
    assert!(message.contains("no/such/directory"), "{message}");
}

#[test]
fn a_folder_is_not_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let err = write_result(&result(), ExportType::Csv, dir.path(), &csv()).unwrap_err();
    assert!(format!("{err:#}").contains("is a folder"), "{err:#}");
}

// --- paths ----------------------------------------------------------------

#[test]
fn resolve_path_expands_tilde_and_adds_extension() {
    let home = Path::new("/home/ann");
    assert_eq!(
        resolve_path("~/exports/orders", ExportType::Csv, home),
        PathBuf::from("/home/ann/exports/orders.csv")
    );
    assert_eq!(
        resolve_path("  rows ", ExportType::JsonLines, home),
        PathBuf::from("rows.jsonl")
    );
    // Only `~/`: neither `~user` nor a Windows path is expanded.
    assert_eq!(
        resolve_path("~bob/rows", ExportType::Csv, home),
        PathBuf::from("~bob/rows.csv")
    );
    assert_eq!(
        resolve_path(r"C:\data\rows", ExportType::Excel, home),
        PathBuf::from(r"C:\data\rows.xlsx")
    );
    assert_eq!(
        display_path(Path::new("/home/ann/exports/orders.csv"), home),
        "~/exports/orders.csv"
    );
    assert_eq!(display_path(Path::new("/tmp/x.csv"), home), "/tmp/x.csv");
}

#[test]
fn resolve_path_keeps_existing_extension_case_insensitive() {
    let home = Path::new("/home/ann");
    assert_eq!(
        resolve_path("ROWS.CSV", ExportType::Csv, home),
        PathBuf::from("ROWS.CSV")
    );
    assert_eq!(
        resolve_path("~/Book.XLSX", ExportType::Excel, home),
        PathBuf::from("/home/ann/Book.XLSX")
    );
    assert_eq!(
        resolve_path("dump.Sql", ExportType::SqlInsert, home),
        PathBuf::from("dump.Sql")
    );
}

// --- text formats -----------------------------------------------------------

#[test]
fn csv_rfc4180_quoting_and_crlf_option() {
    let mut result = result();
    result.rows[0][1] = Some("line\nbreak".into());
    result
        .rows
        .push(vec![Some("4".into()), Some(String::new())]);
    let opts = ExportOptions {
        crlf: true,
        ..csv()
    };
    let (file, _) = write(&result, ExportType::Csv, &opts);
    assert_eq!(
        file.read(),
        concat!(
            "id,note\r\n",
            "1,\"line\nbreak\"\r\n",
            "2,\"has, comma and \"\"quotes\"\"\"\r\n",
            // NULL is nothing; an empty string is quoted.
            "3,\r\n",
            "4,\"\"\r\n",
        )
    );

    let opts = ExportOptions {
        separator: ';',
        ..csv()
    };
    let (file, _) = write(&result, ExportType::Csv, &opts);
    assert!(
        file.read()
            .contains("2;\"has, comma and \"\"quotes\"\"\"\n"),
        "{}",
        file.read()
    );
}

#[test]
fn csv_custom_separator_validation() {
    assert_eq!(parse_separator("|"), Ok('|'));
    assert_eq!(parse_separator(" "), Ok(' '));
    assert_eq!(parse_separator("¦"), Ok('¦'));
    assert!(parse_separator("").is_err());
    assert!(parse_separator("::").is_err());
    for refused in ["\"", "\n", "\r"] {
        assert!(parse_separator(refused).is_err(), "{refused:?}");
    }
    // A value holding the separator is quoted.
    let opts = ExportOptions {
        separator: '|',
        ..csv()
    };
    let mut result = result();
    result.rows[0][1] = Some("a|b".into());
    let (file, _) = write(&result, ExportType::Csv, &opts);
    assert!(file.read().contains("1|\"a|b\"\n"), "{}", file.read());
}

#[test]
fn tsv_escapes_tab_and_newline() {
    let mut result = result();
    result.rows[0][1] = Some("a\tb\nc\\d".into());
    let (file, _) = write(&result, ExportType::Tsv, &csv());
    assert_eq!(
        file.read(),
        "id\tnote\n1\ta\\tb\\nc\\\\d\n2\thas, comma and \"quotes\"\n3\t\n"
    );
}

#[test]
fn json_numbers_unquoted_by_kind() {
    let (file, _) = write(&typed(), ExportType::Json, &csv());
    let text = file.read();
    assert!(
        text.contains(
            "{\"id\": 1, \"price\": 9.50, \"name\": \"ann\", \"active\": true, \"born\": \"2001-02-03\"}"
        ),
        "{text}"
    );
    assert!(text.contains("\"price\": null"), "{text}");
    // It reads back as JSON.
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed[0]["id"], 1);
    assert_eq!(parsed[1]["active"], false);

    // Text that is not a JSON number stays a string, even in a number
    // column.
    for (text, number) in [
        ("0", true),
        ("-12.5e+3", true),
        ("1E9", true),
        ("+1", false),
        (".5", false),
        ("01", false),
        ("NaN", false),
        ("1.", false),
        ("", false),
    ] {
        assert_eq!(is_json_number(text), number, "{text:?}");
    }
}

#[test]
fn jsonl_one_object_per_line() {
    let (file, written) = write(&typed(), ExportType::JsonLines, &csv());
    assert_eq!(written.rows, 2);
    let text = file.read();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(value.is_object());
    }
    assert!(text.starts_with("{\"id\": 1, "), "{text}");
}

#[test]
fn markdown_escapes_pipes() {
    let mut result = result();
    result.rows[0][1] = Some("a|b".into());
    let opts = ExportOptions {
        null_as: "NULL".into(),
        ..csv()
    };
    let (file, _) = write(&result, ExportType::Markdown, &opts);
    assert_eq!(
        file.read(),
        concat!(
            "| id | note |\n",
            "|---|---|\n",
            "| 1 | a\\|b |\n",
            "| 2 | has, comma and \"quotes\" |\n",
            "| 3 | NULL |\n",
        )
    );
}

#[test]
fn header_off_omits_first_line() {
    let opts = ExportOptions {
        header: false,
        ..csv()
    };
    for etype in [ExportType::Csv, ExportType::Tsv, ExportType::Text] {
        let (file, _) = write(&result(), etype, &opts);
        let text = file.read();
        assert!(text.starts_with("1"), "{etype}: {text}");
        assert_eq!(text.lines().count(), 3, "{etype}");
    }
    // Markdown keeps the table's shape with blank names.
    let (file, _) = write(&result(), ExportType::Markdown, &opts);
    assert!(file.read().starts_with("|  |  |\n|---|---|\n| 1 |"));
    // INSERTs leave out the column list.
    let (file, _) = write(&result(), ExportType::SqlInsert, &opts);
    assert!(
        file.read().starts_with("INSERT INTO result VALUES\n"),
        "{}",
        file.read()
    );
}

#[test]
fn null_as_applies_to_text_formats() {
    let opts = ExportOptions {
        null_as: "\\N".into(),
        ..csv()
    };
    for (etype, last) in [
        (ExportType::Csv, "3,\\N"),
        (ExportType::Tsv, "3\t\\\\N"),
        (ExportType::Text, "3,\\N"),
        (ExportType::Markdown, "| 3 | \\N |"),
    ] {
        let (file, _) = write(&result(), etype, &opts);
        let text = file.read();
        assert_eq!(text.lines().last(), Some(last), "{etype}: {text}");
    }
    // JSON keeps its own null.
    let (file, _) = write(&result(), ExportType::Json, &opts);
    assert!(file.read().contains("\"note\": null"));
}

#[test]
fn utf8_bom_goes_first_when_asked() {
    let opts = ExportOptions { bom: true, ..csv() };
    let (file, _) = write(&result(), ExportType::Csv, &opts);
    assert!(
        fs::read(&file.path)
            .unwrap()
            .starts_with(b"\xEF\xBB\xBFid,note")
    );
}

// --- SQL --------------------------------------------------------------------

#[test]
fn sql_insert_batches_and_quotes_per_dialect() {
    let mut result = typed();
    for i in 3..=5 {
        result.rows.push(vec![
            Some(i.to_string()),
            Some("1".into()),
            Some("x".into()),
            None,
            None,
        ]);
    }
    let opts = ExportOptions {
        table: Some("public.order".into()),
        batch: 2,
        ..csv()
    };
    let (file, written) = write(&result, ExportType::SqlInsert, &opts);
    assert_eq!(written.rows, 5);
    let text = file.read();
    assert_eq!(text.matches("INSERT INTO").count(), 3, "{text}");
    assert!(
        text.starts_with(concat!(
            "INSERT INTO public.\"order\" (id, price, name, active, born) VALUES\n",
            "  (1, 9.50, 'ann', 'true', '2001-02-03'),\n",
            "  (2, NULL, 'o''brien', 'false', NULL);\n",
        )),
        "{text}"
    );

    // MySQL quotes with backticks and escapes backslashes.
    let mut mysql = typed();
    mysql.columns[2].name = "select".into();
    mysql.rows[0][2] = Some("a\\b".into());
    let opts = ExportOptions {
        dialect: Dialect::MySql,
        table: Some("t".into()),
        ..csv()
    };
    let (file, _) = write(&mysql, ExportType::SqlInsert, &opts);
    let text = file.read();
    assert!(text.contains("`select`"), "{text}");
    assert!(text.contains("'a\\\\b'"), "{text}");

    // Oracle has no multi-row VALUES.
    let opts = ExportOptions {
        dialect: Dialect::Oracle,
        table: Some("t".into()),
        ..csv()
    };
    let (file, _) = write(&typed(), ExportType::SqlInsert, &opts);
    let text = file.read();
    assert!(text.starts_with("INSERT ALL\n  INTO t (id, "), "{text}");
    assert!(text.ends_with("SELECT 1 FROM DUAL;\n"), "{text}");
}

// --- Excel ------------------------------------------------------------------

#[test]
fn excel_types_cells_and_freezes_header() {
    let mut result = typed();
    result.rows[0][2] = Some("bell\u{7} here".into());
    let file = TempFile::new("xlsx");
    let written = write_result(&result, ExportType::Excel, &file.path, &csv()).unwrap();
    assert_eq!(written.rows, 2);
    assert!(written.warning.is_some(), "a control character was removed");

    let mut book: Xlsx<_> = open_workbook(&file.path).unwrap();
    let sheet = book.worksheet_range_at(0).unwrap().unwrap();
    assert_eq!(sheet.get((0, 0)), Some(&Data::String("id".into())));
    assert_eq!(sheet.get((1, 0)), Some(&Data::Float(1.0)));
    assert_eq!(sheet.get((1, 1)), Some(&Data::Float(9.5)));
    assert_eq!(sheet.get((1, 2)), Some(&Data::String("bell here".into())));
    assert_eq!(sheet.get((1, 3)), Some(&Data::Bool(true)));
    assert!(
        matches!(sheet.get((1, 4)), Some(Data::DateTime(_))),
        "{:?}",
        sheet.get((1, 4))
    );
    assert_eq!(sheet.get((2, 3)), Some(&Data::Bool(false)));
    assert!(matches!(sheet.get((2, 1)), None | Some(Data::Empty)));

    // The header is bold, frozen and filtered: look in the sheet's XML.
    let xml = unzip(&file.path, "xl/worksheets/sheet1.xml");
    assert!(xml.contains("<pane ySplit=\"1\""), "{xml}");
    assert!(xml.contains("<autoFilter ref=\"A1:E3\""), "{xml}");
    assert!(xml.contains("<col "), "column widths: {xml}");
    let styles = unzip(&file.path, "xl/styles.xml");
    assert!(styles.contains("<b/>"), "{styles}");
}

/// One file from the zip archive an .xlsx is, as text.
fn unzip(path: &Path, name: &str) -> String {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn excel_too_many_rows_errors_before_writing() {
    let file = TempFile::new("xlsx");
    let mut sink = excel_sink_with_limit(&file.path, 2).unwrap();
    sink.begin(&result().columns).unwrap();
    let err = sink.rows(&result().rows).unwrap_err();
    assert!(err.to_string().contains("do not fit"), "{err}");
    drop(sink);
    assert!(file.files().is_empty(), "{:?}", file.files());

    // The whole result is checked before anything is opened.
    let mut big = result();
    big.rows = vec![vec![None, None]; EXCEL_MAX_ROWS + 1];
    let err = write_result(&big, ExportType::Excel, &file.path, &csv()).unwrap_err();
    assert!(
        err.to_string().contains("1\u{2009}048\u{2009}576 rows"),
        "{err}"
    );
    assert!(file.files().is_empty());
}

// --- atomicity --------------------------------------------------------------

#[test]
fn atomic_write_leaves_original_on_failure() {
    let file = TempFile::new("csv");
    fs::write(&file.path, "the original\n").unwrap();

    // A sink that is abandoned half-way, as a failed stream leaves it.
    let mut sink = sink(ExportType::Csv, &file.path, &csv()).unwrap();
    sink.begin(&result().columns).unwrap();
    sink.rows(&result().rows[..1]).unwrap();
    assert_eq!(file.files().len(), 2, "the rows go to a file of their own");
    drop(sink);
    assert_eq!(file.read(), "the original\n");
    assert_eq!(file.files(), ["rows.csv"]);

    // An Excel sink refusing rows leaves it too.
    let xlsx = file.dir.path().join("rows.xlsx");
    fs::write(&xlsx, "old").unwrap();
    let mut sink = excel_sink_with_limit(&xlsx, 1).unwrap();
    sink.begin(&result().columns).unwrap();
    assert!(sink.rows(&result().rows).is_err());
    drop(sink);
    assert_eq!(fs::read_to_string(&xlsx).unwrap(), "old");

    // Finished, it replaces the original.
    let mut sink = sink_for(&file.path);
    sink.begin(&result().columns).unwrap();
    sink.rows(&result().rows).unwrap();
    assert_eq!(sink.finish().unwrap(), 3);
    assert!(file.read().starts_with("id,note\n"));
    assert_eq!(file.files(), ["rows.csv", "rows.xlsx"]);
}

fn sink_for(path: &Path) -> Box<dyn RowSink> {
    sink(ExportType::Csv, path, &csv()).unwrap()
}

// --- round trip -------------------------------------------------------------

/// Values with what CSV has to quote: separators, quotes, line breaks.
fn value() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        1 => Just(None),
        1 => Just(Some(String::new())),
        6 => "[a-z ,;\"\n\r|é]{0,8}".prop_map(Some),
    ]
}

fn table() -> impl Strategy<Value = (usize, Vec<Vec<Option<String>>>)> {
    (1usize..5).prop_flat_map(|cols| {
        (
            Just(cols),
            proptest::collection::vec(proptest::collection::vec(value(), cols), 0..12),
        )
    })
}

proptest! {
    #[test]
    fn prop_csv_export_round_trips(
        (cols, rows) in table(),
        separator in prop::sample::select(vec![',', ';', '|', '\t']),
        crlf in any::<bool>(),
    ) {
        let result = QueryResult {
            columns: (0..cols).map(|i| ColumnMeta::untyped(format!("c{i}"))).collect(),
            rows: rows.clone(),
            affected: None,
            elapsed: std::time::Duration::ZERO,
            truncated: false,
        };
        // A marker no value holds tells NULL from the empty string.
        let opts = ExportOptions {
            separator,
            null_as: "\\N".into(),
            crlf,
            ..ExportOptions::default()
        };
        let file = TempFile::new("csv");
        write_result(&result, ExportType::Csv, &file.path, &opts).unwrap();

        let mut reader = csv::ReaderBuilder::new()
            .delimiter(u8::try_from(separator).unwrap())
            .has_headers(true)
            .from_path(&file.path)
            .unwrap();
        let read: Vec<Vec<Option<String>>> = reader
            .records()
            .map(|record| {
                record
                    .unwrap()
                    .iter()
                    .map(|v| (v != "\\N").then(|| v.to_string()))
                    .collect()
            })
            .collect();
        // csv reads `\r` alone as a line end; quoted, it is kept.
        prop_assert_eq!(read, rows);
    }
}

/// A dropped export (a cancelled job, a closed session) stops writing and
/// removes its temporary file, even while the query has rows to come and
/// nothing cancels it on the server.
#[tokio::test]
async fn a_dropped_export_stops_and_removes_its_temporary_file() {
    use std::sync::Arc;

    let mut driver = crate::Driver::new();
    driver
        .connect(crate::ConnectionConfig::Sqlite(
            crate::sqlite::SqliteConfig::new(":memory:", false),
        ))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("endless.csv");
    let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) \
                   SELECT i FROM n";
    let started = Arc::new(tokio::sync::Notify::new());
    let heard = started.clone();
    let opts = ExportOptions::default();
    let mut export = Box::pin(export_query(
        &driver,
        endless,
        100,
        ExportType::Csv,
        &path,
        &opts,
        move |_| heard.notify_one(),
    ));
    tokio::select! {
        outcome = &mut export => panic!("an endless query finished: {outcome:?}"),
        () = started.notified() => {}
    }
    drop(export);
    let names = || fs::read_dir(dir.path()).unwrap().count();
    for _ in 0..200 {
        if names() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(names(), 0, "no temporary file is left, and no target made");
    // The connection is free again: the reader stopped.
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), driver.query("SELECT 1"));
    assert_eq!(
        next.await.expect("the reader stopped").unwrap().rows.len(),
        1
    );
}
