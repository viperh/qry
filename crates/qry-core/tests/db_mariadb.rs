#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! MariaDB-only database tests (T004). They run when
//! `QRY_TEST_MARIADB_URL` is set; see `common/mod.rs`. The bodies are
//! shared with `db_mysql.rs`.

mod common;

use common::{Backend, leftover_test_databases, mysql_family, skip_or_fail};

#[tokio::test]
async fn zero_dates_do_not_panic() {
    let db = db_or_skip!(Backend::MariaDb);
    mysql_family::zero_dates_do_not_panic(&db).await;
    db.cleanup().await;
}

#[tokio::test]
async fn multiple_result_sets_take_the_first() {
    let db = db_or_skip!(Backend::MariaDb);
    mysql_family::multiple_result_sets_take_the_first(&db).await;
    db.cleanup().await;
}

#[tokio::test]
async fn zz_no_leftover_test_databases() {
    let Some(leftovers) = leftover_test_databases(Backend::MariaDb).await else {
        skip_or_fail(Backend::MariaDb);
        return;
    };
    assert!(
        leftovers.is_empty(),
        "test databases older than an hour: {leftovers:?}"
    );
}
