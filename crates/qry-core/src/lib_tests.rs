//! Unit tests for `lib.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn in_memory() -> ConnectionConfig {
    ConnectionConfig::Sqlite(SqliteConfig::new(":memory:", false))
}

#[tokio::test]
async fn query_without_connection_is_err() {
    assert!(Driver::new().query("SELECT 1").await.is_err());
}

#[tokio::test]
async fn sqlite_round_trip() {
    let mut driver = Driver::new();
    assert_eq!(driver.connect(in_memory()).await.unwrap(), ":memory:");

    let result = driver
        .query("WITH t(id, name) AS (VALUES (1, 'alice'), (2, NULL)) SELECT * FROM t")
        .await
        .unwrap();
    assert_eq!(result.column_names(), ["id", "name"]);
    assert_eq!(
        result.rows,
        [
            vec![Some("1".to_string()), Some("alice".to_string())],
            vec![Some("2".to_string()), None],
        ]
    );
}

#[tokio::test]
async fn sqlite_reports_its_server_version() {
    let mut driver = Driver::new();
    assert!(driver.server_version().await.is_err(), "not connected yet");
    driver.connect(in_memory()).await.unwrap();
    let version = driver.server_version().await.unwrap();
    assert!(version.starts_with("SQLite 3."), "{version}");
}

#[tokio::test]
async fn a_fake_reports_the_version_it_was_given_without_a_reply() {
    let fake = testing::FakeDatabase::default().version("PostgreSQL 16.4");
    let driver = Driver::with_database(Box::new(fake.clone()));
    assert_eq!(driver.server_version().await.unwrap(), "PostgreSQL 16.4");
    assert!(fake.calls().is_empty());
}

fn postgres(ssl_mode: SslMode) -> ConnectionConfig {
    ConnectionConfig::Postgres(PostgresConfig {
        host: "db.invalid".into(),
        port: 5432,
        user: "alice".into(),
        password: "hunter2".into(),
        database: "app".into(),
        schema: None,
        ssl_mode,
    })
}

#[tokio::test]
async fn tls_only_ssl_modes_are_refused_before_connecting() {
    for mode in [SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull] {
        let err = Driver::new().connect(postgres(mode)).await.unwrap_err();
        assert!(
            err.to_string().contains("does not support"),
            "{mode}: {err}"
        );
    }
}

#[test]
fn each_type_has_an_extension_and_adds_it_once() {
    let extensions: Vec<&str> = ExportType::ALL.iter().map(|t| t.extension()).collect();
    assert_eq!(
        extensions,
        ["csv", "tsv", "json", "jsonl", "xlsx", "md", "sql", "txt"]
    );

    assert_eq!(ExportType::Csv.with_extension("rows"), "rows.csv");
    assert_eq!(ExportType::Csv.with_extension("rows.csv"), "rows.csv");
    // Already there, whatever the case.
    assert_eq!(ExportType::Csv.with_extension("ROWS.CSV"), "ROWS.CSV");
    assert_eq!(ExportType::Csv.with_extension("rows."), "rows.csv");
    // Another format's extension is replaced, as when the format changes
    // after the path was typed.
    assert_eq!(ExportType::Csv.with_extension("rows.txt"), "rows.csv");
    assert_eq!(
        ExportType::JsonLines.with_extension("all.xlsx"),
        "all.jsonl"
    );
    assert_eq!(ExportType::Json.with_extension("all.JSONL"), "all.json");
    // Any other extension is part of the name, and this one is added.
    assert_eq!(ExportType::Csv.with_extension("rows.2026"), "rows.2026.csv");
    assert_eq!(
        ExportType::Csv.with_extension("out.v1/rows"),
        "out.v1/rows.csv"
    );
    assert_eq!(
        ExportType::Excel.with_extension(r"C:\data\rows"),
        r"C:\data\rows.xlsx"
    );
    assert_eq!(ExportType::Json.with_extension("rows.json"), "rows.json");
}

#[test]
fn a_config_gets_the_extension_of_its_type() {
    let config = ExportConfig::new("rows", ",", ExportType::Json);
    assert_eq!(config.path, "rows.json");
    assert_eq!(config.separator, ",");
}

#[test]
fn debug_output_never_contains_the_password() {
    let shown = format!("{:?}", postgres(SslMode::Prefer));
    assert_eq!(shown, "Postgres(alice@db.invalid:5432/app)");
}

#[tokio::test]
async fn tables_lists_what_the_connection_can_see() {
    let mut driver = Driver::new();
    assert!(driver.tables().await.is_err(), "not connected");

    driver.connect(in_memory()).await.unwrap();
    assert!(driver.tables().await.unwrap().is_empty());

    driver
        .query("CREATE TABLE people (name TEXT)")
        .await
        .unwrap();
    driver
        .query("CREATE TABLE addresses (line TEXT)")
        .await
        .unwrap();
    driver
        .query("CREATE VIEW recent AS SELECT * FROM people")
        .await
        .unwrap();
    assert_eq!(
        driver.tables().await.unwrap(),
        ["addresses", "people", "recent"]
    );
}

#[tokio::test]
async fn state_persists_between_queries() {
    let mut driver = Driver::new();
    driver.connect(in_memory()).await.unwrap();
    driver.query("CREATE TABLE t (x INTEGER)").await.unwrap();
    driver.query("INSERT INTO t VALUES (7)").await.unwrap();

    let result = driver.query("SELECT x FROM t").await.unwrap();
    assert_eq!(result.rows, [vec![Some("7".to_string())]]);
}

#[tokio::test]
async fn sqlite_reports_where_a_statement_went_wrong() {
    let mut driver = Driver::new();
    driver
        .connect(ConnectionConfig::Sqlite(sqlite::SqliteConfig::new(
            ":memory:", false,
        )))
        .await
        .unwrap();
    let sql = "select 1 frum t";
    let error = driver.query(sql).await.unwrap_err();
    let positioned = error.downcast_ref::<shared::QueryError>().unwrap();
    assert_eq!(
        positioned.position.map(|p| &sql[p..]),
        Some("t"),
        "{positioned:?}"
    );
}

#[test]
fn value_kinds_come_from_every_backends_type_names() {
    use shared::ValueKind as K;
    for (name, kind) in [
        ("int8", K::Integer),
        ("MYSQL_TYPE_LONGLONG", K::Integer),
        ("MYSQL_TYPE_LONG", K::Integer),
        // Oracle's LONG is text and LONG RAW binary; the driver reports an
        // inline CLOB and BLOB that way too.
        ("LONG", K::Text),
        ("LONG NVARCHAR", K::Text),
        ("LONG RAW", K::Binary),
        ("NUMBER(10,2)", K::Decimal),
        ("numeric", K::Decimal),
        ("MYSQL_TYPE_NEWDECIMAL", K::Decimal),
        ("float8", K::Float),
        ("BINARY_DOUBLE", K::Float),
        ("VARCHAR2(20)", K::Text),
        ("text", K::Text),
        ("bool", K::Bool),
        ("timestamptz", K::Timestamp),
        ("MYSQL_TYPE_DATETIME", K::Timestamp),
        ("date", K::Date),
        ("jsonb", K::Json),
        ("bytea", K::Binary),
        ("blob", K::Binary),
        ("interval", K::Other),
        ("", K::Other),
    ] {
        assert_eq!(K::from_type_name(name), kind, "{name}");
    }
    assert!(K::Decimal.is_numeric() && !K::Text.is_numeric());
}

#[tokio::test]
async fn sqlite_columns_take_the_type_of_their_first_value() {
    let mut driver = Driver::new();
    driver
        .connect(ConnectionConfig::Sqlite(sqlite::SqliteConfig::new(
            ":memory:", false,
        )))
        .await
        .unwrap();
    let result = driver
        .query("SELECT 1 AS n, 2.5 AS f, 'x' AS s, NULL AS z")
        .await
        .unwrap();
    let kinds: Vec<_> = result.columns.iter().map(|c| c.kind).collect();
    use shared::ValueKind as K;
    assert_eq!(kinds, [K::Integer, K::Float, K::Text, K::Other]);
}

#[tokio::test]
async fn sqlite_stops_reading_at_max_rows() {
    let mut driver = Driver::new();
    driver.connect(in_memory()).await.unwrap();
    let counting = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) \
                    SELECT i FROM n";
    // Without the limit this would never end.
    let result = driver.query_limited(counting, 1000).await.unwrap();
    assert_eq!(result.rows.len(), 1000);
    assert!(result.truncated);
    assert_eq!(result.rows[999], [Some("1000".to_string())]);

    let small = driver
        .query_limited("SELECT 1 UNION SELECT 2", 2)
        .await
        .unwrap();
    assert_eq!(small.rows.len(), 2);
    assert!(!small.truncated, "exactly max_rows is not truncated");
}

#[tokio::test]
async fn sqlite_cancel_interrupts_a_long_query() {
    let mut driver = Driver::new();
    driver.connect(in_memory()).await.unwrap();
    let handle = driver.cancel_handle().unwrap();
    let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) \
                   SELECT count(*) FROM n";
    let (outcome, ()) = tokio::join!(driver.query(endless), async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.cancel().await.unwrap();
    });
    let err = outcome.unwrap_err();
    assert!(format!("{err:#}").contains("interrupt"), "{err:#}");
    // The connection is still good.
    assert_eq!(driver.query("SELECT 1").await.unwrap().rows.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_fake_cancels_only_the_statement_in_progress() {
    use testing::{CANCELLED, FakeDatabase, Reply, grid};
    let db = FakeDatabase::default()
        .reply(Reply::Hang)
        .reply(Reply::Rows(grid(3, 1)));
    let driver = Driver::with_database(Box::new(db.clone()));
    let handle = driver.cancel_handle().unwrap();
    let (outcome, ()) = tokio::join!(driver.query("hang"), async {
        tokio::task::yield_now().await;
        handle.cancel().await.unwrap();
    });
    assert_eq!(outcome.unwrap_err().to_string(), CANCELLED);
    // Nothing is stored up for the next statement.
    handle.cancel().await.unwrap();
    let next = driver.query_limited("next", 2).await.unwrap();
    assert!(next.truncated && next.rows.len() == 2);
    assert_eq!(db.cancels(), 2);
}
