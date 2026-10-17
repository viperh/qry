#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Oracle-only database tests (T004). They run when `QRY_TEST_ORACLE_URL`
//! is set; see `common/mod.rs`. Each test's database is an Oracle user
//! (a schema) of its own.

mod common;

use common::{Backend, column, leftover_test_databases, q, skip_or_fail};
use qry_core::ValueKind;

#[tokio::test]
async fn clob_is_read_as_text() {
    let db = db_or_skip!(Backend::Oracle);
    let driver = db.connect().await;
    let result = q(
        &driver,
        "SELECT note FROM people WHERE id IN (1, 4) ORDER BY id",
    )
    .await;
    // The driver reads LOBs inline and reports the column as LONG; what
    // matters is that it is text, not the number `long` means for MySQL.
    assert!(
        ["CLOB", "LONG"].contains(&result.columns[0].type_name.as_str()),
        "{}",
        result.columns[0].type_name
    );
    assert_eq!(result.columns[0].kind, ValueKind::Text);
    assert_eq!(column(&result, 0), [Some("first"), Some("likes 'quotes'")]);
    drop(driver);
    db.cleanup().await;
}

#[tokio::test]
async fn number_without_scale_is_an_integer_string() {
    let db = db_or_skip!(Backend::Oracle);
    let driver = db.connect().await;
    // `id` is NUMBER(10); COUNT(*) is a NUMBER without precision or scale.
    let result = q(
        &driver,
        "SELECT id, COUNT(*) OVER () AS n FROM people WHERE id = 2",
    )
    .await;
    assert_eq!(result.rows, [vec![Some("2".into()), Some("1".into())]]);
    drop(driver);
    db.cleanup().await;
}

#[tokio::test]
async fn tables_uses_user_tables_and_views() {
    let db = db_or_skip!(Backend::Oracle);
    let driver = db.connect().await;
    // The `oracle.rs` override: the current schema's tables and views, as
    // Oracle names them (upper case), and nothing of other schemas.
    assert_eq!(
        driver.tables().await.unwrap(),
        ["ACTIVE_PEOPLE", "ORDERS", "PEOPLE"]
    );
    drop(driver);
    db.cleanup().await;
}

#[tokio::test]
async fn zz_no_leftover_test_databases() {
    let Some(leftovers) = leftover_test_databases(Backend::Oracle).await else {
        skip_or_fail(Backend::Oracle);
        return;
    };
    assert!(
        leftovers.is_empty(),
        "test users older than an hour: {leftovers:?}"
    );
}
