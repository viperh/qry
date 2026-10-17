#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Postgres-only database tests (T004). They run when
//! `QRY_TEST_POSTGRES_URL` is set; see `common/mod.rs`.

mod common;

use common::{Backend, leftover_test_databases, q, skip_or_fail};
use qry_core::{
    ConnectionConfig, Driver, SslMode,
    schema::{Children, Index, ObjectKind, SchemaPath},
};

#[tokio::test]
async fn ssl_mode_require_against_a_non_tls_server_fails_clearly() {
    let db = db_or_skip!(Backend::Postgres);
    let mut config = db.config.clone();
    let ConnectionConfig::Postgres(c) = &mut config else {
        unreachable!()
    };
    c.ssl_mode = SslMode::Require;
    let error = Driver::new().connect(config).await.unwrap_err();
    let message = format!("{error:#}");
    // qry has no TLS for Postgres yet, so it refuses before connecting and
    // names the mode, rather than failing somewhere in the handshake.
    assert!(
        message.contains("require") && message.contains("TLS"),
        "{message:?}"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn search_path_decides_current_schema_for_tables() {
    let db = db_or_skip!(Backend::Postgres);
    let driver = db.connect().await;
    q(&driver, "CREATE SCHEMA other").await;
    q(&driver, "CREATE TABLE other.only_here (x integer)").await;

    // Without a schema, `public`: the seed's tables, not the other one.
    let public = driver.tables().await.unwrap();
    assert_eq!(public, ["active_people", "orders", "people"]);

    let mut config = db.config.clone();
    let ConnectionConfig::Postgres(c) = &mut config else {
        unreachable!()
    };
    c.schema = Some("other".into());
    let mut other = Driver::new();
    other.connect(config).await.unwrap();
    assert_eq!(other.tables().await.unwrap(), ["only_here"]);
    drop((driver, other));
    db.cleanup().await;
}

/// The catalog queries' Postgres-only parts: `unnest(indkey)` gives an
/// expression's slot as `(expression)`, in index order, and materialized
/// views come from `pg_matviews`.
#[tokio::test]
async fn expression_indexes_and_materialized_views_load() {
    let db = db_or_skip!(Backend::Postgres);
    let driver = db.connect().await;
    q(
        &driver,
        "CREATE INDEX people_lower_name ON people (lower(name), id)",
    )
    .await;
    q(
        &driver,
        "CREATE MATERIALIZED VIEW people_count AS SELECT count(*) AS n FROM people",
    )
    .await;

    let Children::Details(details) = driver
        .load_schema(&SchemaPath::details("public", "people"))
        .await
        .unwrap()
    else {
        panic!("not details")
    };
    let index = details
        .indexes
        .iter()
        .find(|i| i.name == "people_lower_name")
        .unwrap_or_else(|| panic!("{:?}", details.indexes));
    assert_eq!(
        *index,
        Index {
            name: "people_lower_name".into(),
            columns: vec!["(expression)".into(), "id".into()],
            unique: false,
        }
    );

    let Children::Objects(objects) = driver
        .load_schema(&SchemaPath::objects("public"))
        .await
        .unwrap()
    else {
        panic!("not objects")
    };
    let matview = objects
        .iter()
        .find(|o| o.name == "people_count")
        .unwrap_or_else(|| panic!("{objects:?}"));
    assert_eq!(matview.kind, ObjectKind::MaterializedView);
    drop(driver);
    db.cleanup().await;
}

#[tokio::test]
async fn zz_no_leftover_test_databases() {
    let Some(leftovers) = leftover_test_databases(Backend::Postgres).await else {
        skip_or_fail(Backend::Postgres);
        return;
    };
    assert!(
        leftovers.is_empty(),
        "test databases older than an hour: {leftovers:?}"
    );
}
