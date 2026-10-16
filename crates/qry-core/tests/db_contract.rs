#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The contract every backend keeps (T004). Each `async fn <name>(db)` runs
//! once per backend, as the test `<name>::<backend>`, against a fresh copy
//! of the seed (`tests/fixtures/sql/seed_<backend>.sql`). Where a dialect
//! really differs, the expectation says so in a `match`, with the reason;
//! nothing is skipped silently.
//!
//! SQLite always runs. The servers run when their `QRY_TEST_*_URL` is set
//! (see `common/mod.rs`).

mod common;

use std::time::Duration;

use common::{Backend, TestDb, assert_no_rows, column, lower, lower_names, q, run_script};
use qry_core::{
    ConnectionConfig, Driver, ValueKind,
    copy::{self, CopyFormat, Selection},
    shared::QueryError,
    sql,
};

/// Emits `mod <name> { fn sqlite(), fn postgres(), … }` for each contract
/// function: one `#[tokio::test]` per backend, each with its own database.
macro_rules! contract {
    ($($name:ident),+ $(,)?) => {$(
        mod $name {
            contract!(@backend $name, sqlite, Sqlite);
            contract!(@backend $name, postgres, Postgres);
            contract!(@backend $name, mysql, Mysql);
            contract!(@backend $name, mariadb, MariaDb);
            contract!(@backend $name, oracle, Oracle);
        }
    )+};
    (@backend $name:ident, $test:ident, $backend:ident) => {
        #[tokio::test]
        async fn $test() {
            let db = $crate::db_or_skip!($crate::common::Backend::$backend);
            super::$name(&db).await;
            db.cleanup().await;
        }
    };
}

contract!(
    connects_and_reports_a_label,
    select_returns_columns_and_rows_in_order,
    null_is_none_and_empty_string_is_some_empty,
    unicode_round_trips,
    numeric_keeps_scale_and_sign,
    dates_render_as_iso_8601,
    booleans_render_as_true_false,
    binary_renders_as_hex,
    dml_reports_affected_rows,
    ddl_succeeds_without_rows,
    syntax_error_is_an_error_with_the_server_message,
    tables_lists_tables_and_views_sorted,
    a_wrong_password_fails_with_an_auth_error,
    a_closed_port_fails_fast,
    two_selects_produce_two_results,
    error_position_reported,
    column_kinds_mapped,
    copy_insert_round_trip,
);

/// `FROM dual` where Oracle needs a table to select from.
fn dual(db: &TestDb) -> &'static str {
    if db.backend == Backend::Oracle {
        " FROM dual"
    } else {
        ""
    }
}

async fn connects_and_reports_a_label(db: &TestDb) {
    let mut driver = Driver::new();
    let label = driver.connect(db.config.clone()).await.unwrap();
    let expected: Vec<String> = match &db.config {
        ConnectionConfig::Sqlite(c) => {
            vec![c.path.file_name().unwrap().to_string_lossy().into_owned()]
        }
        ConnectionConfig::Postgres(c) => vec![c.host.clone(), c.database.clone()],
        ConnectionConfig::Mysql(c) => vec![c.host.clone(), c.database.clone()],
        ConnectionConfig::MariaDb(c) => vec![c.host.clone(), c.database.clone()],
        // The service is Oracle's database; the test database is the user.
        ConnectionConfig::Oracle(c) => vec![c.host.clone(), c.service.clone(), c.user.clone()],
    };
    for part in expected {
        assert!(label.contains(&part), "{label:?} lacks {part:?}");
    }
}

async fn select_returns_columns_and_rows_in_order(db: &TestDb) {
    let driver = db.connect().await;
    let result = q(&driver, "SELECT id, name FROM people ORDER BY id").await;
    assert_eq!(lower_names(&result), ["id", "name"]);
    assert_eq!(
        column(&result, 0),
        [Some("1"), Some("2"), Some("3"), Some("4"), Some("5")]
    );
    assert_eq!(
        column(&result, 1),
        [
            Some("Ada"),
            Some("Zoë"),
            Some("東京"),
            Some("Bob"),
            Some("Eve")
        ]
    );
}

async fn null_is_none_and_empty_string_is_some_empty(db: &TestDb) {
    let driver = db.connect().await;
    let result = q(
        &driver,
        "SELECT note FROM people WHERE id IN (2, 3) ORDER BY id",
    )
    .await;
    let expected = match db.backend {
        // Oracle stores '' as NULL: VARCHAR2 and CLOB have no empty string.
        Backend::Oracle => [None, None],
        _ => [Some(""), None],
    };
    assert_eq!(column(&result, 0), expected);
}

async fn unicode_round_trips(db: &TestDb) {
    let driver = db.connect().await;
    let result = q(
        &driver,
        "SELECT name FROM people WHERE id IN (2, 3) ORDER BY id",
    )
    .await;
    let names = column(&result, 0);
    assert_eq!(names[0].map(str::as_bytes), Some("Zoë".as_bytes()));
    assert_eq!(names[1].map(str::as_bytes), Some("東京".as_bytes()));
    // And back to the server, inside a literal.
    let found = q(&driver, "SELECT id FROM people WHERE name = '東京'").await;
    assert_eq!(column(&found, 0), [Some("3")]);
}

async fn numeric_keeps_scale_and_sign(db: &TestDb) {
    let driver = db.connect().await;
    let result = q(&driver, "SELECT balance FROM people ORDER BY id").await;
    let expected = match db.backend {
        // SQLite has no fixed-point type: NUMERIC affinity stores -12.50 as
        // the REAL -12.5, and 1000.00 and 0.00 as integers.
        Backend::Sqlite => [Some("-12.5"), Some("1000"), Some("0"), Some("42.1"), None],
        _ => [
            Some("-12.50"),
            Some("1000.00"),
            Some("0.00"),
            Some("42.10"),
            None,
        ],
    };
    assert_eq!(column(&result, 0), expected);
}

async fn dates_render_as_iso_8601(db: &TestDb) {
    let driver = db.connect().await;
    let born = q(&driver, "SELECT born FROM people ORDER BY id").await;
    assert_eq!(
        column(&born, 0),
        [
            Some("1990-04-01"),
            Some("1985-12-31"),
            None,
            Some("2000-02-29"),
            Some("1970-01-01")
        ]
    );

    let (whole, fraction) = match db.backend {
        // SQLite has no timestamp type; its date functions write this form.
        Backend::Sqlite => ("SELECT datetime('2001-02-03 04:05:06')", None),
        _ => (
            "SELECT TIMESTAMP '2001-02-03 04:05:06'",
            Some("SELECT TIMESTAMP '2001-02-03 04:05:06.5'"),
        ),
    };
    let whole = q(&driver, &format!("{whole}{}", dual(db))).await;
    assert_eq!(column(&whole, 0), [Some("2001-02-03 04:05:06")]);
    if let Some(fraction) = fraction {
        let fraction = q(&driver, &format!("{fraction}{}", dual(db))).await;
        assert_eq!(column(&fraction, 0), [Some("2001-02-03 04:05:06.5")]);
    }
}

async fn booleans_render_as_true_false(db: &TestDb) {
    let driver = db.connect().await;
    let result = q(&driver, "SELECT active FROM people ORDER BY id").await;
    let expected = match db.backend {
        // SQLite has no boolean type: TRUE is stored, and read, as 1.
        Backend::Sqlite => [Some("1"), Some("0"), Some("1"), Some("0"), None],
        // MySQL and MariaDB included: BOOL is tinyint(1).
        _ => [
            Some("true"),
            Some("false"),
            Some("true"),
            Some("false"),
            None,
        ],
    };
    assert_eq!(column(&result, 0), expected);
}

async fn binary_renders_as_hex(db: &TestDb) {
    let driver = db.connect().await;
    let result = q(&driver, "SELECT avatar FROM people ORDER BY id").await;
    let hex = |bytes: &[u8]| Some(format!("\\x{}", hex::encode(bytes)));
    assert_eq!(
        result.rows.iter().map(|r| r[0].clone()).collect::<Vec<_>>(),
        [
            hex(&[0x00, 0xff, 0x10]),
            None,
            hex(&[0xde, 0xad, 0xbe, 0xef]),
            None,
            None
        ]
    );
}

async fn dml_reports_affected_rows(db: &TestDb) {
    let driver = db.connect().await;
    // People 2 and 4 are inactive, so both rows really change (MySQL counts
    // changed rows, not matched ones).
    let sql = "UPDATE people SET active = TRUE WHERE id IN (2, 4)";
    let result = q(&driver, sql).await;
    assert_eq!(result.affected, Some(2), "{result:?}");
    assert_no_rows(&result, sql);

    let active = q(&driver, "SELECT COUNT(*) FROM active_people").await;
    assert_eq!(column(&active, 0), [Some("4")]);
}

async fn ddl_succeeds_without_rows(db: &TestDb) {
    let driver = db.connect().await;
    for sql in ["CREATE TABLE ddl_probe (x INT)", "DROP TABLE ddl_probe"] {
        let result = q(&driver, sql).await;
        assert_no_rows(&result, sql);
    }
}

async fn syntax_error_is_an_error_with_the_server_message(db: &TestDb) {
    let driver = db.connect().await;
    let error = driver.query("SELEC 1").await.unwrap_err();
    let message = format!("{error:#}");
    let wording = match db.backend {
        Backend::Postgres | Backend::Sqlite => "syntax error",
        Backend::Mysql | Backend::MariaDb => "SQL syntax",
        Backend::Oracle => "ORA-00900",
    };
    assert!(message.contains(wording), "{message:?} lacks {wording:?}");
}

async fn tables_lists_tables_and_views_sorted(db: &TestDb) {
    let driver = db.connect().await;
    let tables = driver.tables().await.unwrap();
    assert_eq!(lower(&tables), ["active_people", "orders", "people"]);
}

async fn a_wrong_password_fails_with_an_auth_error(db: &TestDb) {
    const WRONG: &str = "not-the-password-4711";
    let mut config = db.config.clone();
    match &mut config {
        // SQLite has no accounts, so nothing can be refused.
        ConnectionConfig::Sqlite(_) => return,
        ConnectionConfig::Postgres(c) => c.password = WRONG.into(),
        ConnectionConfig::Mysql(c) => c.password = WRONG.into(),
        ConnectionConfig::MariaDb(c) => c.password = WRONG.into(),
        ConnectionConfig::Oracle(c) => c.password = WRONG.into(),
    }
    let error = Driver::new().connect(config).await.unwrap_err();
    let message = format!("{error:#}");
    let lowered = message.to_lowercase();
    assert!(
        lowered.contains("password")
            || lowered.contains("authentication")
            || message.contains("ORA-01017"),
        "{message:?}"
    );
    assert!(!message.contains(WRONG), "the password leaked: {message:?}");
}

async fn a_closed_port_fails_fast(db: &TestDb) {
    let mut config = db.config.clone();
    match &mut config {
        // SQLite opens a file; there is no port.
        ConnectionConfig::Sqlite(_) => return,
        ConnectionConfig::Postgres(c) => (c.host, c.port) = ("127.0.0.1".into(), 1),
        ConnectionConfig::Mysql(c) => (c.host, c.port) = ("127.0.0.1".into(), 1),
        ConnectionConfig::MariaDb(c) => (c.host, c.port) = ("127.0.0.1".into(), 1),
        ConnectionConfig::Oracle(c) => (c.host, c.port) = ("127.0.0.1".into(), 1),
    }
    let attempt = tokio::time::timeout(Duration::from_secs(5), Driver::new().connect(config)).await;
    match attempt {
        Ok(result) => assert!(result.is_err(), "connected to port 1"),
        Err(_) => panic!("connecting to a closed port took more than 5 s"),
    }
}

/// T302: the editor splits a script and runs each statement on its own.
async fn two_selects_produce_two_results(db: &TestDb) {
    let driver = db.connect().await;
    let from = dual(db);
    let script = format!("SELECT 1 AS a{from}; SELECT 2 AS b{from};");
    let statements = sql::split(db.backend.dialect(), &script);
    assert_eq!(statements.len(), 2, "{statements:?}");
    let mut results = Vec::new();
    for statement in &statements {
        results.push(q(&driver, &statement.text).await);
    }
    assert_eq!(lower_names(&results[0]), ["a"]);
    assert_eq!(column(&results[0], 0), [Some("1")]);
    assert_eq!(lower_names(&results[1]), ["b"]);
    assert_eq!(column(&results[1], 0), [Some("2")]);
}

/// T302: the server's error position, as a byte offset into the statement,
/// rides inside the error as a `QueryError`.
async fn error_position_reported(db: &TestDb) {
    let driver = db.connect().await;
    // `WHER` reads as an alias of `people`, so `id` is where it goes wrong.
    let sql = "SELECT name FROM people WHER id = 1";
    let error = driver.query(sql).await.unwrap_err();
    let positioned = error.downcast_ref::<QueryError>();
    match db.backend {
        Backend::Postgres | Backend::Sqlite | Backend::Oracle => {
            let positioned = positioned.unwrap_or_else(|| panic!("no QueryError in {error:#}"));
            assert_eq!(
                positioned.position.map(|p| &sql[p..]),
                Some("id = 1"),
                "{positioned:?}"
            );
        }
        // MySQL and MariaDB report no position (T302 notes).
        Backend::Mysql | Backend::MariaDb => {
            assert!(
                positioned.is_none_or(|p| p.position.is_none()),
                "{positioned:?}"
            );
        }
    }
}

/// T303: each result column's `ValueKind`, from the type the server reports.
async fn column_kinds_mapped(db: &TestDb) {
    use ValueKind as K;
    let driver = db.connect().await;
    let (create, insert) = match db.backend {
        Backend::Postgres => (
            "CREATE TABLE kinds (i integer, d numeric(10, 2), t text, b boolean, \
             dt date, ts timestamp, j jsonb, bl bytea)",
            "INSERT INTO kinds VALUES (1, 1.50, 'x', TRUE, DATE '2001-02-03', \
             TIMESTAMP '2001-02-03 04:05:06', '{\"a\": 1}', '\\x01')",
        ),
        Backend::Mysql | Backend::MariaDb => (
            "CREATE TABLE kinds (i int, d decimal(10, 2), t text, b bool, \
             dt date, ts timestamp NULL, j json, bl blob)",
            "INSERT INTO kinds VALUES (1, 1.50, 'x', TRUE, DATE '2001-02-03', \
             TIMESTAMP '2001-02-03 04:05:06', '{\"a\": 1}', X'01')",
        ),
        Backend::Sqlite => (
            "CREATE TABLE kinds (i integer, d numeric(10, 2), t text, b boolean, \
             dt date, ts timestamp, j json, bl blob)",
            "INSERT INTO kinds VALUES (1, 1.5, 'x', TRUE, '2001-02-03', \
             '2001-02-03 04:05:06', '{\"a\": 1}', X'01')",
        ),
        Backend::Oracle => (
            "CREATE TABLE kinds (i NUMBER(10), d NUMBER(10, 2), t VARCHAR2(10), b BOOLEAN, \
             dt DATE, ts TIMESTAMP, j JSON, bl RAW(16))",
            "INSERT INTO kinds VALUES (1, 1.50, 'x', TRUE, DATE '2001-02-03', \
             TIMESTAMP '2001-02-03 04:05:06', '{\"a\": 1}', HEXTORAW('01'))",
        ),
    };
    q(&driver, create).await;
    q(&driver, insert).await;
    let result = q(&driver, "SELECT i, d, t, b, dt, ts, j, bl FROM kinds").await;
    let kinds: Vec<ValueKind> = result.columns.iter().map(|c| c.kind).collect();
    let expected = match db.backend {
        Backend::Postgres | Backend::Mysql => {
            vec![
                K::Integer,
                K::Decimal,
                K::Text,
                K::Bool,
                K::Date,
                K::Timestamp,
                K::Json,
                K::Binary,
            ]
        }
        // MariaDB's JSON is an alias of LONGTEXT, and says so on the wire.
        Backend::MariaDb => {
            vec![
                K::Integer,
                K::Decimal,
                K::Text,
                K::Bool,
                K::Date,
                K::Timestamp,
                K::Text,
                K::Binary,
            ]
        }
        // SQLite types values, not columns: a column takes its first
        // value's type, and only integer, real, text and blob exist.
        Backend::Sqlite => {
            vec![
                K::Integer,
                K::Float,
                K::Text,
                K::Integer,
                K::Text,
                K::Text,
                K::Text,
                K::Binary,
            ]
        }
        // Oracle's integers are NUMBERs too.
        Backend::Oracle => {
            vec![
                K::Decimal,
                K::Decimal,
                K::Text,
                K::Bool,
                K::Date,
                K::Timestamp,
                K::Json,
                K::Binary,
            ]
        }
    };
    assert_eq!(kinds, expected, "{:?}", result.columns);
}

/// T303: rows copied as SQL INSERTs go back in unchanged.
async fn copy_insert_round_trip(db: &TestDb) {
    let driver = db.connect().await;
    // Dates are left out: a quoted date reads with Oracle's NLS format.
    let select = "SELECT id, name, balance, note FROM people";
    q(
        &driver,
        &format!("CREATE TABLE people_copy AS {select} WHERE 1 = 0"),
    )
    .await;
    let original = q(&driver, &format!("{select} ORDER BY id")).await;
    let rows = (0..original.rows.len()).collect();
    let inserts = copy::render(
        &original,
        &Selection::all(&original, rows),
        CopyFormat::Insert,
        db.backend.dialect(),
        "people_copy",
    );
    assert_eq!(inserts.lines().count(), 5, "{inserts}");
    run_script(&driver, db.backend.dialect(), &inserts)
        .await
        .unwrap_or_else(|e| panic!("{e:#}\n{inserts}"));
    let copied = q(
        &driver,
        "SELECT id, name, balance, note FROM people_copy ORDER BY id",
    )
    .await;
    assert_eq!(copied.rows, original.rows, "{inserts}");
}
