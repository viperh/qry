#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Exporting every row of a query straight to a file (T307), per backend.
//! SQLite always runs; the servers run when their `QRY_TEST_*_URL` is set.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use common::{Backend, TestDb, q, run_script};
use qry_core::{
    ExportType,
    exporter::{self, ExportOptions},
    shared::StreamItem,
};

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
    export_all_rows_streams_without_materialising,
    export_sql_insert_reimports_identically,
    cancelled_export_removes_temp_and_keeps_original,
    a_failure_part_way_comes_after_the_rows_before_it,
    a_reader_that_goes_stops_the_stream_and_the_connection_works_on,
);

const FETCH_SIZE: usize = 1000;

/// A query of `n` numbered rows, in each backend's words, and `n`.
fn series(backend: Backend) -> (String, u64) {
    match backend {
        Backend::Sqlite => (
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 200000) \
             SELECT n, 'row ' || n AS label FROM s"
                .into(),
            200_000,
        ),
        Backend::Postgres => (
            "SELECT n, 'row ' || n AS label FROM generate_series(1, 200000) AS n".into(),
            200_000,
        ),
        // A recursive CTE stops at 1 000 levels there: ten digits, five
        // times over.
        Backend::Mysql | Backend::MariaDb => (
            "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
             UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
             UNION ALL SELECT 8 UNION ALL SELECT 9) \
             SELECT a.x + 10 * b.x + 100 * c.x + 1000 * e.x + 10000 * f.x AS n, 'row' AS label \
             FROM d a, d b, d c, d e, d f"
                .into(),
            100_000,
        ),
        Backend::Oracle => (
            "SELECT LEVEL AS n, 'row ' || LEVEL AS label FROM dual CONNECT BY LEVEL <= 200000"
                .into(),
            200_000,
        ),
    }
}

/// The series with one row that fails, the 150 000th (the 99 999th for
/// MySQL and MariaDB, whose series is shorter), in each backend's words.
/// A backend that read the whole result before sending any of it would
/// fail with nothing sent.
fn failing_series(backend: Backend) -> String {
    match backend {
        // `abs` of the smallest integer is an overflow error.
        Backend::Sqlite => {
            "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 200000) \
             SELECT n, CASE WHEN n = 150000 THEN abs(n - 150000 - 9223372036854775807 - 1) \
             ELSE n END AS v FROM s"
        }
        Backend::Postgres => {
            "SELECT n, CASE WHEN n = 150000 THEN 1 / (n - 150000) ELSE n END AS v \
             FROM generate_series(1, 200000) AS n"
        }
        // An unsigned overflow is an error even in a SELECT. 99 999 is the
        // last row whichever table the join walks innermost.
        Backend::Mysql | Backend::MariaDb => {
            "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
             UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
             UNION ALL SELECT 8 UNION ALL SELECT 9), \
             s(n) AS (SELECT a.x + 10 * b.x + 100 * c.x + 1000 * e.x + 10000 * f.x \
             FROM d a, d b, d c, d e, d f) \
             SELECT n, IF(n = 99999, ~0 + n, n) AS v FROM s"
        }
        Backend::Oracle => {
            "SELECT LEVEL AS n, CASE WHEN LEVEL = 150000 THEN 1 / (LEVEL - 150000) \
             ELSE LEVEL END AS v FROM dual CONNECT BY LEVEL <= 200000"
        }
    }
    .into()
}

/// Every backend streams (T307): the batches read before a failure reach
/// the reader before the failure does.
async fn a_failure_part_way_comes_after_the_rows_before_it(db: &TestDb) {
    let driver = db.connect().await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let reader = tokio::spawn(async move {
        let mut rows = 0u64;
        while let Some(item) = rx.recv().await {
            if let StreamItem::Rows(batch) = item {
                rows += batch.len() as u64;
            }
        }
        rows
    });
    let outcome = driver
        .query_stream(&failing_series(db.backend), FETCH_SIZE, tx)
        .await;
    let rows = reader.await.unwrap();
    assert!(outcome.is_err(), "the failing row fails it: {outcome:?}");
    assert!(
        rows >= FETCH_SIZE as u64,
        "only {rows} rows arrived before the failure"
    );
    // The connection is still usable.
    let after = q(&driver, &format!("SELECT 1 AS one{}", dual(db.backend))).await;
    assert_eq!(after.rows, [vec![Some("1".to_string())]]);
}

/// Dropping the receiver stops the stream: the query fails at its next
/// batch rather than reading on into memory, and the connection can run
/// the next statement.
async fn a_reader_that_goes_stops_the_stream_and_the_connection_works_on(db: &TestDb) {
    let driver = db.connect().await;
    let (sql, n) = series(db.backend);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let reader = tokio::spawn(async move {
        let mut rows = 0;
        while let Some(item) = rx.recv().await {
            if let StreamItem::Rows(batch) = item {
                rows += batch.len();
                break;
            }
        }
        rows
    });
    let outcome = driver.query_stream(&sql, FETCH_SIZE, tx).await;
    assert!(reader.await.unwrap() <= FETCH_SIZE);
    let err = outcome.expect_err("the reader went after one batch");
    assert!(
        format!("{err:#}").contains("reader stopped"),
        "{err:#} (of {n} rows)"
    );
    let after = q(&driver, &format!("SELECT 1 AS one{}", dual(db.backend))).await;
    assert_eq!(after.rows, [vec![Some("1".to_string())]]);
}

/// `FROM dual` where Oracle needs a table to select from.
fn dual(backend: Backend) -> &'static str {
    if backend == Backend::Oracle {
        " FROM dual"
    } else {
        ""
    }
}

async fn export_all_rows_streams_without_materialising(db: &TestDb) {
    let driver = db.connect().await;
    let (sql, n) = series(db.backend);

    // The stream itself: no batch is bigger than the fetch size.
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let reader = tokio::spawn(async move {
        let (mut peak, mut total, mut columns) = (0, 0u64, 0);
        while let Some(item) = rx.recv().await {
            match item {
                StreamItem::Columns(c) => columns = c.len(),
                StreamItem::Rows(rows) => {
                    peak = peak.max(rows.len());
                    total += rows.len() as u64;
                }
            }
        }
        (peak, total, columns)
    });
    let sent = driver.query_stream(&sql, FETCH_SIZE, tx).await.unwrap();
    let (peak, total, columns) = reader.await.unwrap();
    assert_eq!((sent, total, columns), (n, n, 2));
    assert!(peak <= FETCH_SIZE, "a batch of {peak} rows");

    // And through to a file, with progress on the way.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("series.csv");
    let seen = Arc::new(AtomicU64::new(0));
    let heard = seen.clone();
    let written = exporter::export_query(
        &driver,
        &sql,
        FETCH_SIZE,
        ExportType::Csv,
        &path,
        &ExportOptions::default(),
        move |rows| heard.store(rows, Ordering::Relaxed),
    )
    .await
    .unwrap();
    assert_eq!(written.rows, n);
    assert_eq!(seen.load(Ordering::Relaxed), n);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count() as u64, n + 1, "the rows and a header");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

async fn export_sql_insert_reimports_identically(db: &TestDb) {
    let driver = db.connect().await;
    // Dates are left out: a quoted date reads with Oracle's NLS format.
    let select = "SELECT id, name, balance, note FROM people";
    q(
        &driver,
        &format!("CREATE TABLE people_copy AS {select} WHERE 1 = 0"),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("people.sql");
    let opts = ExportOptions {
        table: Some("people_copy".into()),
        dialect: db.backend.dialect(),
        batch: 2,
        ..ExportOptions::default()
    };
    let sql = format!("{select} ORDER BY id");
    let written = exporter::export_query(
        &driver,
        &sql,
        FETCH_SIZE,
        ExportType::SqlInsert,
        &path,
        &opts,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(written.rows, 5);
    let script = std::fs::read_to_string(&path).unwrap();
    run_script(&driver, db.backend.dialect(), &script)
        .await
        .unwrap_or_else(|e| panic!("{e:#}\n{script}"));
    let original = q(&driver, &sql).await;
    let copied = q(
        &driver,
        "SELECT id, name, balance, note FROM people_copy ORDER BY id",
    )
    .await;
    assert_eq!(copied.rows, original.rows, "{script}");
}

async fn cancelled_export_removes_temp_and_keeps_original(db: &TestDb) {
    let driver = db.connect().await;
    let (sql, _) = series(db.backend);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("series.csv");
    std::fs::write(&path, "the original\n").unwrap();

    // Cancelled through the connection's handle once rows are coming; a
    // driver without one has the export dropped instead, as closing the
    // session does.
    let cancel = driver.cancel_handle();
    let started = Arc::new(tokio::sync::Notify::new());
    let heard = started.clone();
    let opts = ExportOptions::default();
    let mut export = Box::pin(exporter::export_query(
        &driver,
        &sql,
        FETCH_SIZE,
        ExportType::Csv,
        &path,
        &opts,
        move |_| heard.notify_one(),
    ));
    tokio::select! {
        outcome = &mut export => panic!("finished before the cancel: {outcome:?}"),
        () = started.notified() => {}
    }
    match cancel {
        Some(cancel) => {
            cancel.cancel().await.unwrap();
            let outcome = export.await;
            if db.backend == Backend::Sqlite {
                assert!(outcome.is_err(), "SQLite streams, so it stops part-way");
            }
            // A cancel that reaches the server after its last row (MySQL's
            // `KILL QUERY` on a result already in the socket) is too late;
            // the file is then whole, never half-written.
            if outcome.is_ok() {
                assert!(
                    std::fs::read_to_string(&path)
                        .unwrap()
                        .starts_with("n,label")
                );
                return;
            }
        }
        None => drop(export),
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "the original\n");
    // A dropped export's writer lets go of its file on its own thread.
    let names = || -> Vec<_> {
        std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect()
    };
    for _ in 0..200 {
        if names().len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let names = names();
    assert_eq!(names, ["series.csv"], "no temporary file is left");
}
