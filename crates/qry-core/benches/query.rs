//! `Driver::query` against an in-memory SQLite, through to the
//! `QueryResult`, so the type conversion is included.
//!
//! - `sqlite_select/100k_rows` fetches a whole 100k-row table.
//! - `stream_first_1000_of_1m_sqlite` is T305's gate. Streaming does not
//!   exist yet, so for now it is a `SELECT ... LIMIT 1000` over a 1M-row
//!   table: the same rows reach the UI, and SQLite stops after them. When
//!   T305 adds the row stream, this bench should read the first 1000 rows
//!   from the stream of an unlimited `SELECT` instead, under the same id.
//!
//! The tables are filled by SQLite itself from deterministic expressions of
//! the row number, so every run sees the same data.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use qry_core::{ConnectionConfig, Driver, sqlite::SqliteConfig};

/// A table of `rows` rows with an integer key, text, a real, a timestamp,
/// a flag and a nullable note.
fn seed(table: &str, rows: u64) -> String {
    format!(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
         INSERT INTO {table} (id, name, amount, created, flag, note)
         SELECT i,
                'user_' || (i * 2654435761 % 1000003),
                (i * 40503 % 100000) / 100.0,
                datetime(1700000000 + i * 37, 'unixepoch'),
                i % 2,
                CASE WHEN i % 7 = 0 THEN NULL ELSE 'note ' || (i % 977) END
           FROM n"
    )
}

fn create(table: &str) -> String {
    format!(
        "CREATE TABLE {table} (id INTEGER PRIMARY KEY, name TEXT, amount REAL, \
         created TEXT, flag INTEGER, note TEXT)"
    )
}

fn query(c: &mut Criterion) {
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        panic!("cannot start a tokio runtime");
    };
    let driver = rt.block_on(async {
        let mut driver = Driver::new();
        let config = ConnectionConfig::Sqlite(SqliteConfig::new(":memory:", false));
        if let Err(e) = driver.connect(config).await {
            panic!("cannot open an in-memory SQLite: {e:#}");
        }
        for sql in [
            create("small"),
            seed("small", 100_000),
            create("big"),
            seed("big", 1_000_000),
        ] {
            if let Err(e) = driver.query(&sql).await {
                panic!("cannot seed the database: {e:#}\n{sql}");
            }
        }
        driver
    });

    let run = |sql: &'static str, rows: usize| {
        let driver = &driver;
        let rt = &rt;
        move || {
            let result = rt.block_on(driver.query(sql));
            match result {
                Ok(result) if result.rows.len() == rows => black_box(result),
                Ok(result) => panic!("{sql}: {} rows, not {rows}", result.rows.len()),
                Err(e) => panic!("{sql}: {e:#}"),
            }
        }
    };

    let mut group = c.benchmark_group("sqlite_select");
    group.sample_size(20);
    let select_all = run("SELECT * FROM small", 100_000);
    group.bench_function("100k_rows", |b| b.iter(&select_all));
    group.finish();

    let first_1000 = run("SELECT * FROM big LIMIT 1000", 1000);
    c.bench_function("stream_first_1000_of_1m_sqlite", |b| b.iter(&first_1000));
}

criterion_group!(benches, query);
criterion_main!(benches);
