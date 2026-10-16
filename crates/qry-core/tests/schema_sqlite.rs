//! The schema methods against a real SQLite database in a temporary file.
//! These need no server, so they always run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{
    Database,
    schema::{Children, ObjectKind, SchemaPath, load},
    sqlite::{Sqlite, SqliteConfig},
};

/// A fresh database file, kept alive with its directory.
async fn open() -> (Sqlite, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Sqlite::connect(SqliteConfig::new(dir.path().join("t.db"), false))
        .await
        .unwrap();
    (db, dir)
}

/// `users(id pk)` and `orders(user_id fk)`, as the spec's fixture.
async fn fixture(db: &Sqlite) {
    for sql in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE, name TEXT)",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), \
         total NUMERIC DEFAULT 0, note TEXT)",
        "CREATE INDEX orders_user_total ON orders (user_id, total)",
        "CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100",
    ] {
        db.execute(sql).await.unwrap();
    }
}

#[tokio::test]
async fn catalogs_lists_default_first_and_flags_system() {
    let (db, dir) = open().await;
    let other = dir.path().join("other.db");
    db.execute(&format!("ATTACH DATABASE '{}' AS other", other.display()))
        .await
        .unwrap();
    db.execute("CREATE TEMP TABLE scratch (x)").await.unwrap();

    let catalogs = db.catalogs().await.unwrap();
    assert_eq!(catalogs[0].name, "main");
    assert!(catalogs[0].is_default && !catalogs[0].is_system);
    let other = catalogs.iter().find(|c| c.name == "other").unwrap();
    assert!(!other.is_default && !other.is_system);
    let temp = catalogs.iter().find(|c| c.name == "temp").unwrap();
    assert!(temp.is_system, "temp is SQLite's own");
}

#[tokio::test]
async fn objects_distinguishes_tables_and_views() {
    let (db, _dir) = open().await;
    fixture(&db).await;
    let objects = db.objects("main").await.unwrap();
    let kinds: Vec<(&str, ObjectKind)> =
        objects.iter().map(|o| (o.name.as_str(), o.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("big_orders", ObjectKind::View),
            ("orders", ObjectKind::Table),
            ("users", ObjectKind::Table),
        ]
    );
    assert!(objects.iter().all(|o| o.catalog == "main"));
    assert_eq!(
        db.tables().await.unwrap(),
        ["big_orders", "orders", "users"]
    );
}

#[tokio::test]
async fn columns_report_type_nullability_pk_fk() {
    let (db, _dir) = open().await;
    fixture(&db).await;
    let columns = db.columns("main", "orders").await.unwrap();
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "user_id", "total", "note"]);

    let id = &columns[0];
    assert!(id.primary_key && !id.nullable);
    assert_eq!(id.data_type, "INTEGER");
    assert_eq!(id.ordinal, 1);

    let user_id = &columns[1];
    assert!(!user_id.primary_key && !user_id.nullable);
    assert_eq!(user_id.references, Some(("users".into(), "id".into())));

    let total = &columns[2];
    assert!(total.nullable);
    assert_eq!(total.default.as_deref(), Some("0"));
    assert_eq!(total.data_type, "NUMERIC");
}

#[tokio::test]
async fn indexes_report_unique_and_columns() {
    let (db, _dir) = open().await;
    fixture(&db).await;
    let indexes = db.indexes("main", "orders").await.unwrap();
    let total = indexes
        .iter()
        .find(|i| i.name == "orders_user_total")
        .unwrap();
    assert_eq!(total.columns, ["user_id", "total"]);
    assert!(!total.unique);

    let users = db.indexes("main", "users").await.unwrap();
    assert!(
        users.iter().any(|i| i.unique && i.columns == ["email"]),
        "{users:?}"
    );
}

#[tokio::test]
async fn hostile_names_are_bound_not_interpolated() {
    let (db, _dir) = open().await;
    db.execute("CREATE TABLE t (x INTEGER)").await.unwrap();
    let hostile = "x'; DROP TABLE t; --";
    db.execute(&format!(
        "CREATE TABLE \"{}\" (\"a'b\" TEXT)",
        hostile.replace('"', "\"\"")
    ))
    .await
    .unwrap();

    let columns = db.columns("main", hostile).await.unwrap();
    assert_eq!(columns[0].name, "a'b");
    assert!(db.indexes("main", hostile).await.unwrap().is_empty());
    assert!(db.objects(hostile).await.unwrap().is_empty());

    let names: Vec<String> = db
        .objects("main")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.name)
        .collect();
    assert!(
        names.contains(&"t".to_string()),
        "t is still there: {names:?}"
    );
    assert!(names.contains(&hostile.to_string()));
}

#[tokio::test]
async fn mixed_case_and_spaced_names_round_trip() {
    let (db, _dir) = open().await;
    db.execute("CREATE TABLE \"Order Items\" (\"Line No\" INTEGER PRIMARY KEY, \"Qty ✓\" INT)")
        .await
        .unwrap();
    let objects = db.objects("main").await.unwrap();
    assert_eq!(objects[0].name, "Order Items");
    let columns = db.columns("main", "Order Items").await.unwrap();
    assert_eq!(columns[0].name, "Line No");
    assert_eq!(columns[1].name, "Qty ✓");

    let Children::Details(details) = load(&db, &SchemaPath::details("main", "Order Items"))
        .await
        .unwrap()
    else {
        panic!("details");
    };
    assert_eq!(details.columns.len(), 2);
}

#[tokio::test]
async fn in_memory_scratch_works_like_a_file() {
    let db = Sqlite::connect(SqliteConfig::new(":memory:", false))
        .await
        .unwrap();
    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let catalogs = db.catalogs().await.unwrap();
    assert_eq!(catalogs[0].name, "main");
    assert_eq!(db.objects("main").await.unwrap()[0].name, "users");
}
