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
    assert_eq!(result.columns, ["id", "name"]);
    assert_eq!(
        result.rows,
        [
            vec![Some("1".to_string()), Some("alice".to_string())],
            vec![Some("2".to_string()), None],
        ]
    );
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
    assert_eq!(extensions, ["csv", "xlsx", "txt", "json"]);

    assert_eq!(ExportType::Csv.with_extension("rows"), "rows.csv");
    assert_eq!(ExportType::Csv.with_extension("rows.csv"), "rows.csv");
    // Already there, whatever the case.
    assert_eq!(ExportType::Csv.with_extension("ROWS.CSV"), "ROWS.CSV");
    assert_eq!(ExportType::Csv.with_extension("rows."), "rows.csv");
    // A different extension is kept, and the right one added after it.
    assert_eq!(ExportType::Csv.with_extension("rows.txt"), "rows.txt.csv");
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
