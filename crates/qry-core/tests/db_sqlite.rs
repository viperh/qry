#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! SQLite-only database tests (T004). SQLite needs no server, so these
//! always run, each on a file in its own scratch directory.

mod common;

use common::{Backend, column, q};
use qry_core::{ConnectionConfig, Driver, sqlite::SqliteConfig};

#[tokio::test]
async fn file_database_persists_across_connections() {
    let db = db_or_skip!(Backend::Sqlite);
    {
        let driver = db.connect().await;
        q(&driver, "INSERT INTO people (id, name) VALUES (6, 'Kept')").await;
    }
    let path = db.sqlite_path().unwrap();
    assert!(path.is_file(), "{} is not a file", path.display());

    let again = db.connect().await;
    let result = q(&again, "SELECT name FROM people WHERE id = 6").await;
    assert_eq!(column(&result, 0), [Some("Kept")]);
    db.cleanup().await;
}

#[tokio::test]
async fn read_only_rejects_writes() {
    let db = db_or_skip!(Backend::Sqlite);
    let path = db.sqlite_path().unwrap();
    let mut driver = Driver::new();
    driver
        .connect(ConnectionConfig::Sqlite(SqliteConfig::new(&path, true)))
        .await
        .unwrap();

    // Reading works…
    let result = q(&driver, "SELECT COUNT(*) FROM people").await;
    assert_eq!(column(&result, 0), [Some("5")]);
    // …writing does not, and says why.
    let error = driver
        .query("UPDATE people SET name = 'nope' WHERE id = 1")
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("readonly"), "{message:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn read_only_does_not_create_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.sqlite");
    let opened = Driver::new()
        .connect(ConnectionConfig::Sqlite(SqliteConfig::new(&path, true)))
        .await;
    assert!(opened.is_err());
    assert!(!path.exists());
}
