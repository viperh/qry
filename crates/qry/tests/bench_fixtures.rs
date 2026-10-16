//! The benchmarks' generated fixtures (`benches/common.rs`) are the same on
//! every run. The file is included here because `cargo test` does not build
//! bench targets.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry::import::{Format, from_bytes};

#[path = "../benches/common.rs"]
mod common;

#[test]
fn benches_common_fixtures_are_deterministic() {
    assert_eq!(common::wide_result(1000, 12), common::wide_result(1000, 12));
    let result = common::wide_result(1000, 12);
    assert_eq!(result.rows.len(), 1000);
    assert!(result.rows.iter().all(|row| row.len() == 12));

    let sql = common::mixed_sql(10 * 1024);
    assert_eq!(sql, common::mixed_sql(10 * 1024));
    assert!(
        (10 * 1024..11 * 1024).contains(&sql.len()),
        "{} bytes",
        sql.len()
    );

    let lines = common::sql_lines(10_000);
    assert_eq!(lines.lines().count(), 10_000);
    assert_eq!(lines, common::sql_lines(10_000));

    assert_eq!(common::dbeaver_file(10), common::dbeaver_file(10));
}

#[test]
fn generated_connection_files_import_whole() {
    for (format, text) in [
        (Format::DBeaver, common::dbeaver_file(50)),
        (Format::SqlDeveloper, common::sqldeveloper_file(50)),
        (Format::Qry, common::qry_file(50)),
    ] {
        let imported = from_bytes(text.as_bytes(), Format::Auto).unwrap();
        assert_eq!(imported.format, format);
        assert_eq!(imported.connections.len(), 50, "{:?}", imported.skipped);
    }
}
