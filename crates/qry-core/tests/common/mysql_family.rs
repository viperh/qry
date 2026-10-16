//! Tests MySQL and MariaDB share; `db_mysql.rs` and `db_mariadb.rs` run
//! each one against their own server.

use super::{Backend, TestDb, column, lower_names, q};

/// `0000-00-00`, which only a lax `sql_mode` lets in, comes back as text
/// instead of panicking in a date conversion.
pub async fn zero_dates_do_not_panic(db: &TestDb) {
    let driver = db.connect().await;
    // The session's mode: the driver keeps one connection.
    q(&driver, "SET SESSION sql_mode = ''").await;
    q(&driver, "CREATE TABLE zero_dates (d date, ts datetime)").await;
    q(
        &driver,
        "INSERT INTO zero_dates VALUES ('0000-00-00', '0000-00-00 00:00:00')",
    )
    .await;
    let result = q(&driver, "SELECT d, ts FROM zero_dates").await;
    assert_eq!(
        result.rows,
        [vec![
            Some("0000-00-00".to_string()),
            Some("0000-00-00 00:00:00".to_string())
        ]]
    );
}

/// Two statements in one call give the first one's result; the rest are
/// read and dropped. This documents today's behaviour: the editor splits
/// statements itself (T302), so it never sends two at once.
pub async fn multiple_result_sets_take_the_first(db: &TestDb) {
    assert!(matches!(db.backend, Backend::Mysql | Backend::MariaDb));
    let driver = db.connect().await;
    let result = q(&driver, "SELECT 1 AS a; SELECT 2 AS b").await;
    assert_eq!(lower_names(&result), ["a"]);
    assert_eq!(column(&result, 0), [Some("1")]);
    // The connection is still usable: the second result was drained.
    let after = q(&driver, "SELECT 3 AS c").await;
    assert_eq!(column(&after, 0), [Some("3")]);
}
