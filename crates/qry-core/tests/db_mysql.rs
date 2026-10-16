#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! MySQL-only database tests (T004). They run when `QRY_TEST_MYSQL_URL` is
//! set; see `common/mod.rs`. `db_mariadb.rs` runs the same bodies against
//! MariaDB.

mod common;

use common::{Backend, leftover_test_databases, mysql_family, skip_or_fail};

#[tokio::test]
async fn zero_dates_do_not_panic() {
    let db = db_or_skip!(Backend::Mysql);
    mysql_family::zero_dates_do_not_panic(&db).await;
    db.cleanup().await;
}

#[tokio::test]
async fn multiple_result_sets_take_the_first() {
    let db = db_or_skip!(Backend::Mysql);
    mysql_family::multiple_result_sets_take_the_first(&db).await;
    db.cleanup().await;
}

#[tokio::test]
async fn zz_no_leftover_test_databases() {
    let Some(leftovers) = leftover_test_databases(Backend::Mysql).await else {
        skip_or_fail(Backend::Mysql);
        return;
    };
    assert!(
        leftovers.is_empty(),
        "test databases older than an hour: {leftovers:?}"
    );
}
