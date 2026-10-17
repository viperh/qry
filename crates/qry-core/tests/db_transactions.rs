#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Autocommit and kept streams (T305), per backend. SQLite always runs;
//! the servers run when their `QRY_TEST_*_URL` is set.

mod common;

use std::sync::Arc;

use common::{Backend, TestDb, q};
use qry_core::{Driver, shared::StreamItem};

/// One `#[tokio::test]` per backend for each function, as in
/// `db_contract.rs`.
macro_rules! per_backend {
    ($($name:ident),+ $(,)?) => {$(
        mod $name {
            per_backend!(@backend $name, sqlite, Sqlite);
            per_backend!(@backend $name, postgres, Postgres);
            per_backend!(@backend $name, mysql, Mysql);
            per_backend!(@backend $name, mariadb, MariaDb);
            per_backend!(@backend $name, oracle, Oracle);
        }
    )+};
    (@backend $name:ident, $test:ident, $backend:ident) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $test() {
            let db = $crate::db_or_skip!($crate::common::Backend::$backend);
            super::$name(&db).await;
            db.cleanup().await;
        }
    };
}

per_backend!(
    autocommit_off_rollback_discards,
    autocommit_off_commit_keeps,
    autocommit_back_on_commits,
    stream_kept_continues_where_it_stopped,
);

/// A table of its own for each test, made while autocommit is still on
/// (DDL commits on MySQL and Oracle anyway).
async fn probe_table(driver: &Driver, name: &str) {
    q(driver, &format!("CREATE TABLE {name} (n INTEGER)")).await;
}

/// How many rows `table` has, as another connection sees them.
async fn count(driver: &Driver, table: &str) -> String {
    let result = q(driver, &format!("SELECT COUNT(*) FROM {table}")).await;
    result.rows[0][0].clone().unwrap_or_default()
}

/// With autocommit off, a write is not seen elsewhere, and a ROLLBACK
/// undoes it. On Oracle this is the driver not committing after the
/// statement.
async fn autocommit_off_rollback_discards(db: &TestDb) {
    let driver = db.connect().await;
    let other = db.connect().await;
    probe_table(&driver, "tx_rollback").await;
    driver.set_autocommit(false).await.unwrap();
    q(&driver, "INSERT INTO tx_rollback VALUES (1)").await;
    assert_eq!(count(&other, "tx_rollback").await, "0", "not committed yet");
    q(&driver, "ROLLBACK").await;
    assert_eq!(count(&driver, "tx_rollback").await, "0");
    assert_eq!(count(&other, "tx_rollback").await, "0");
}

/// With autocommit off, a COMMIT makes the write seen elsewhere.
async fn autocommit_off_commit_keeps(db: &TestDb) {
    let driver = db.connect().await;
    let other = db.connect().await;
    probe_table(&driver, "tx_commit").await;
    driver.set_autocommit(false).await.unwrap();
    q(&driver, "INSERT INTO tx_commit VALUES (1)").await;
    q(&driver, "COMMIT").await;
    assert_eq!(count(&other, "tx_commit").await, "1");
}

/// Turning autocommit back on commits what was pending, and writes after
/// it commit as they go.
async fn autocommit_back_on_commits(db: &TestDb) {
    let driver = db.connect().await;
    let other = db.connect().await;
    probe_table(&driver, "tx_back_on").await;
    driver.set_autocommit(false).await.unwrap();
    q(&driver, "INSERT INTO tx_back_on VALUES (1)").await;
    driver.set_autocommit(true).await.unwrap();
    assert_eq!(count(&other, "tx_back_on").await, "1");
    q(&driver, "INSERT INTO tx_back_on VALUES (2)").await;
    assert_eq!(count(&other, "tx_back_on").await, "2");
}

/// Rows `1..=n` in each backend's words.
fn numbers(backend: Backend, n: u32) -> String {
    match backend {
        Backend::Sqlite => format!(
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < {n}) \
             SELECT n FROM s"
        ),
        Backend::Postgres => format!("SELECT n FROM generate_series(1, {n}) AS n"),
        // A recursive CTE stops at 1 000 levels there: four digits.
        Backend::Mysql | Backend::MariaDb => format!(
            "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
             UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
             UNION ALL SELECT 8 UNION ALL SELECT 9) \
             SELECT n FROM (SELECT 1 + a.x + 10 * b.x + 100 * c.x + 1000 * e.x AS n \
             FROM d a, d b, d c, d e) t WHERE n <= {n} ORDER BY n"
        ),
        Backend::Oracle => format!("SELECT LEVEL AS n FROM dual CONNECT BY LEVEL <= {n}"),
    }
}

/// What the app's worker does to keep a cursor: a stream nobody reads for
/// a while waits, and reading on gives the next rows with none missed or
/// repeated. Dropping the reader ends it, and the connection works after.
async fn stream_kept_continues_where_it_stopped(db: &TestDb) {
    let driver = Arc::new(db.connect().await);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let streaming = driver.clone();
    let sql = numbers(db.backend, 5000);
    let task = tokio::spawn(async move { streaming.query_stream(&sql, 500, tx).await });

    async fn take(rx: &mut tokio::sync::mpsc::Receiver<StreamItem>, n: usize) -> Vec<u32> {
        let mut seen = Vec::new();
        while seen.len() < n {
            match rx.recv().await.expect("the stream ended early") {
                StreamItem::Columns(_) => {}
                StreamItem::Rows(rows) => seen.extend(
                    rows.into_iter()
                        .map(|r| r[0].as_deref().unwrap().parse::<u32>().unwrap()),
                ),
            }
        }
        seen
    }
    let first = take(&mut rx, 1000).await;
    assert_eq!(first, (1..=1000).collect::<Vec<_>>());
    // Nothing reads for now; then on from where it stopped.
    tokio::task::yield_now().await;
    let next = take(&mut rx, 1000).await;
    assert_eq!(next, (1001..=2000).collect::<Vec<_>>());

    drop(rx);
    assert!(task.await.unwrap().is_err(), "the reader went away");
    let one = q(&driver, &numbers(db.backend, 1)).await;
    assert_eq!(one.rows.len(), 1);
}
