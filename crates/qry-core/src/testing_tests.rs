//! Unit tests for `testing.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::Driver;

#[tokio::test]
async fn fake_answers_in_script_order_and_records_calls() {
    let db = FakeDatabase::new(DatabaseType::Postgres)
        .reply(Reply::Rows(result(&["id"], &[&[Some("1")], &[None]])))
        .reply(Reply::Affected(3))
        .tables(&["users", "orders"]);
    let driver = Driver::with_database(Box::new(db.clone()));

    let first = driver.query("SELECT id FROM t").await.unwrap();
    assert_eq!(first.columns, ["id"]);
    assert_eq!(first.rows, [vec![Some("1".to_string())], vec![None]]);

    let second = driver.query("DELETE FROM t").await.unwrap();
    assert!(second.columns.is_empty());
    assert_eq!(second.affected, Some(3));

    // Listing tables uses no reply and is not a call.
    assert_eq!(driver.tables().await.unwrap(), ["users", "orders"]);

    // The script is used up, and the error says which statement was extra.
    let extra = driver.query("SELECT 1").await.unwrap_err().to_string();
    assert!(extra.contains("SELECT 1"), "{extra}");

    assert_eq!(
        db.calls(),
        ["SELECT id FROM t", "DELETE FROM t", "SELECT 1"]
    );
}

#[tokio::test(start_paused = true)]
async fn fake_delay_respects_a_paused_clock() {
    let db = FakeDatabase::default().reply(Reply::Delay(
        Duration::from_secs(30),
        Box::new(Reply::Affected(1)),
    ));
    let wall = std::time::Instant::now();
    let started = tokio::time::Instant::now();

    let query = tokio::spawn(async move { db.execute("UPDATE t SET x = 1").await });
    // Not answered before the delay is up.
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(!query.is_finished());

    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(query.await.unwrap().unwrap(), 1);
    assert!(started.elapsed() >= Duration::from_secs(30));
    assert!(
        wall.elapsed() < Duration::from_secs(1),
        "the delay waited in real time"
    );
}

#[tokio::test]
async fn fake_error_surfaces_as_query_error() {
    let db = FakeDatabase::default().reply(Reply::Error("no such table: nope".into()));
    let driver = Driver::with_database(Box::new(db));
    let error = driver.query("SELECT * FROM nope").await.unwrap_err();
    assert_eq!(error.to_string(), "no such table: nope");
}

#[test]
fn grid_names_columns_and_cells_by_position() {
    let grid = grid(2, 3);
    assert_eq!(grid.columns, ["c0", "c1", "c2"]);
    assert_eq!(grid.rows[1][2].as_deref(), Some("r1c2"));
    assert_eq!(grid.rows.len(), 2);
}
