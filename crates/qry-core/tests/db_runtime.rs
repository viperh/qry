#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! What the app asks of a connection besides plain queries: the server's
//! version (T308), a row limit (T305) and cancelling a running statement
//! (T305), per backend. SQLite always runs; the servers run when their
//! `QRY_TEST_*_URL` is set (see `common/mod.rs`).

mod common;

use std::time::{Duration, Instant};

use common::{Backend, TestDb, column, q};

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
    server_version_names_the_server,
    query_limited_keeps_the_first_rows_and_says_there_are_more,
    a_limited_query_leaves_the_connection_usable,
    cancel_stops_a_running_statement,
);

async fn server_version_names_the_server(db: &TestDb) {
    let driver = db.connect().await;
    let version = driver.server_version().await.unwrap();
    let prefix = match db.backend {
        Backend::Sqlite => "SQLite 3.",
        Backend::Postgres => "PostgreSQL ",
        Backend::Mysql => "MySQL ",
        Backend::MariaDb => "MariaDB ",
        Backend::Oracle => "Oracle ",
    };
    assert!(version.starts_with(prefix), "{version:?}");
    assert!(
        version.chars().any(|c| c.is_ascii_digit()),
        "a number in {version:?}"
    );
    assert!(!version.contains('\n'), "one line: {version:?}");
}

async fn query_limited_keeps_the_first_rows_and_says_there_are_more(db: &TestDb) {
    let driver = db.connect().await;
    let sql = "SELECT id, name FROM people ORDER BY id";
    let two = driver.query_limited(sql, 2).await.unwrap();
    assert_eq!(column(&two, 0), [Some("1"), Some("2")]);
    assert_eq!(two.columns.len(), 2);
    assert!(two.truncated);

    // Exactly as many as the limit, or fewer: nothing more to say.
    let five = driver.query_limited(sql, 5).await.unwrap();
    assert_eq!(five.rows.len(), 5);
    assert!(!five.truncated);
    let all = driver.query_limited(sql, 0).await.unwrap();
    assert_eq!(all.rows.len(), 5);
    assert!(!all.truncated);
}

/// A query of many rows, in each backend's words.
fn many_rows(backend: Backend) -> &'static str {
    match backend {
        Backend::Sqlite => {
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 50000) \
             SELECT n FROM s"
        }
        Backend::Postgres => "SELECT n FROM generate_series(1, 50000) AS n",
        Backend::Mysql | Backend::MariaDb => {
            "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
             UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
             UNION ALL SELECT 8 UNION ALL SELECT 9) \
             SELECT a.x + 10 * b.x + 100 * c.x + 1000 * e.x AS n FROM d a, d b, d c, d e"
        }
        Backend::Oracle => "SELECT LEVEL AS n FROM dual CONNECT BY LEVEL <= 50000",
    }
}

/// The rows past the limit are left unread or drained, never left on the
/// wire where the next statement would trip over them.
async fn a_limited_query_leaves_the_connection_usable(db: &TestDb) {
    let driver = db.connect().await;
    let limited = driver
        .query_limited(many_rows(db.backend), 10)
        .await
        .unwrap();
    assert_eq!(limited.rows.len(), 10);
    assert!(limited.truncated);
    let next = q(&driver, "SELECT name FROM people WHERE id = 3").await;
    assert_eq!(column(&next, 0), [Some("東京")]);
    let again = driver
        .query_limited(many_rows(db.backend), 3)
        .await
        .unwrap();
    assert_eq!(again.rows.len(), 3);
}

/// A statement that takes a while, in each backend's words.
fn slow(backend: Backend) -> &'static str {
    match backend {
        // No sleep: a count that takes far longer than the test waits.
        Backend::Sqlite => {
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s) \
             SELECT count(*) FROM s"
        }
        Backend::Postgres => "SELECT pg_sleep(30)",
        Backend::Mysql | Backend::MariaDb => "SELECT SLEEP(30)",
        Backend::Oracle => "SELECT COUNT(*) FROM dual CONNECT BY LEVEL <= 1000000000",
    }
}

async fn cancel_stops_a_running_statement(db: &TestDb) {
    let driver = db.connect().await;
    let Some(cancel) = driver.cancel_handle() else {
        // Oracle has no cancel handle yet; every other backend must have one.
        assert_eq!(db.backend, Backend::Oracle, "no cancel handle");
        return;
    };
    let started = Instant::now();
    let (outcome, cancelled) = tokio::time::timeout(
        Duration::from_secs(15),
        futures::future::join(driver.query(slow(db.backend)), async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            cancel.cancel().await
        }),
    )
    .await
    .expect("the statement stopped once cancelled");
    cancelled.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    match db.backend {
        // `KILL QUERY` interrupts `SLEEP()`, which then returns 1 rather
        // than failing.
        Backend::Mysql | Backend::MariaDb => {
            if let Ok(result) = &outcome {
                assert_eq!(column(result, 0), [Some("1")]);
            }
        }
        _ => {
            let error = outcome.expect_err("a cancelled statement fails");
            let message = format!("{error:#}").to_lowercase();
            assert!(
                message.contains("cancel") || message.contains("interrupt"),
                "{message:?}"
            );
        }
    }
    // The connection outlives the statement.
    let after = q(&driver, "SELECT name FROM people WHERE id = 1").await;
    assert_eq!(column(&after, 0), [Some("Ada")]);
}
