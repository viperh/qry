#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The schema browser's catalog queries (T301) against every backend's own
//! catalog, on the T004 seed. SQLite always runs; the servers run when their
//! `QRY_TEST_*_URL` is set (see `common/mod.rs`).

mod common;

use common::{Backend, TestDb, q};
use qry_core::{
    Driver,
    schema::{Catalog, Children, Column, DbObject, Details, Index, ObjectKind, SchemaPath},
    sql,
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
        #[tokio::test]
        async fn $test() {
            let db = $crate::db_or_skip!($crate::common::Backend::$backend);
            super::$name(&db).await;
            db.cleanup().await;
        }
    };
}

per_backend!(
    catalogs_put_the_connections_own_first,
    objects_lists_the_seed_tables_and_view,
    columns_come_in_order_with_keys_and_nullability,
    indexes_list_their_columns_in_index_order,
    a_name_that_needs_quoting_loads_like_any_other,
    tables_is_the_default_catalogs_objects,
    defaults_are_none_unless_the_column_has_one,
);

/// The catalog the seed is in: the connection's own.
fn own_catalog(db: &TestDb) -> String {
    match db.backend {
        Backend::Sqlite => "main".into(),
        Backend::Postgres => "public".into(),
        Backend::Mysql | Backend::MariaDb => db.name.clone(),
        Backend::Oracle => db.name.to_uppercase(),
    }
}

/// Oracle folds unquoted names to upper case; the others keep them.
fn folded(db: &TestDb, name: &str) -> String {
    if db.backend == Backend::Oracle {
        name.to_uppercase()
    } else {
        name.to_string()
    }
}

async fn catalogs(driver: &Driver) -> Vec<Catalog> {
    match driver.load_schema(&SchemaPath::Catalogs).await.unwrap() {
        Children::Catalogs(c) => c,
        other => panic!("not catalogs: {other:?}"),
    }
}

async fn objects(driver: &Driver, catalog: &str) -> Vec<DbObject> {
    match driver
        .load_schema(&SchemaPath::objects(catalog))
        .await
        .unwrap()
    {
        Children::Objects(o) => o,
        other => panic!("not objects: {other:?}"),
    }
}

async fn details(driver: &Driver, catalog: &str, object: &str) -> Details {
    match driver
        .load_schema(&SchemaPath::details(catalog, object))
        .await
        .unwrap_or_else(|e| panic!("details of {catalog}.{object}: {e:#}"))
    {
        Children::Details(d) => d,
        other => panic!("not details: {other:?}"),
    }
}

fn column<'a>(columns: &'a [Column], name: &str) -> &'a Column {
    columns
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("no column {name} in {columns:?}"))
}

fn lower_columns(index: &Index) -> Vec<String> {
    index.columns.iter().map(|c| c.to_lowercase()).collect()
}

async fn catalogs_put_the_connections_own_first(db: &TestDb) {
    let driver = db.connect().await;
    let catalogs = catalogs(&driver).await;
    let own = own_catalog(db);
    assert_eq!(catalogs[0].name, own, "{catalogs:?}");
    assert!(catalogs[0].is_default && !catalogs[0].is_system);
    assert_eq!(catalogs.iter().filter(|c| c.is_default).count(), 1);
    // The server's own bookkeeping is there, flagged as such.
    let system = match db.backend {
        Backend::Sqlite => None,
        Backend::Postgres => Some("pg_catalog"),
        Backend::Mysql | Backend::MariaDb => Some("information_schema"),
        Backend::Oracle => None,
    };
    if let Some(system) = system {
        let found = catalogs
            .iter()
            .find(|c| c.name == system)
            .unwrap_or_else(|| panic!("no {system} in {catalogs:?}"));
        assert!(found.is_system && !found.is_default);
    }
}

async fn objects_lists_the_seed_tables_and_view(db: &TestDb) {
    let driver = db.connect().await;
    let own = own_catalog(db);
    let objects = objects(&driver, &own).await;
    let got: Vec<(String, ObjectKind)> = objects
        .iter()
        .map(|o| (o.name.to_lowercase(), o.kind))
        .collect();
    assert_eq!(
        got,
        [
            ("active_people".to_string(), ObjectKind::View),
            ("orders".to_string(), ObjectKind::Table),
            ("people".to_string(), ObjectKind::Table),
        ]
    );
    assert!(objects.iter().all(|o| o.catalog == own));
}

async fn columns_come_in_order_with_keys_and_nullability(db: &TestDb) {
    let driver = db.connect().await;
    let own = own_catalog(db);
    let orders = details(&driver, &own, &folded(db, "orders")).await.columns;
    let names: Vec<String> = orders.iter().map(|c| c.name.to_lowercase()).collect();
    assert_eq!(names, ["id", "person_id", "total"]);
    let ordinals: Vec<u32> = orders.iter().map(|c| c.ordinal).collect();
    assert_eq!(ordinals, [1, 2, 3]);
    assert!(orders.iter().all(|c| !c.data_type.is_empty()), "{orders:?}");

    let id = column(&orders, "id");
    assert!(id.primary_key && !id.nullable, "{id:?}");
    assert_eq!(id.references, None);
    let person = column(&orders, "person_id");
    assert!(!person.primary_key && !person.nullable, "{person:?}");
    let (table, to) = person.references.clone().expect("a foreign key");
    assert_eq!(table.to_lowercase(), "people");
    assert_eq!(to.to_lowercase(), "id");
    let total = column(&orders, "total");
    assert!(!total.primary_key && !total.nullable && total.references.is_none());

    let people = details(&driver, &own, &folded(db, "people")).await.columns;
    assert_eq!(people.len(), 7);
    assert!(column(&people, "note").nullable);
    assert!(!column(&people, "name").nullable);

    // A view has columns too, and no keys.
    let view = details(&driver, &own, &folded(db, "active_people"))
        .await
        .columns;
    let names: Vec<String> = view.iter().map(|c| c.name.to_lowercase()).collect();
    assert_eq!(names, ["id", "name"]);
    assert!(
        view.iter()
            .all(|c| !c.primary_key && c.references.is_none())
    );
}

async fn indexes_list_their_columns_in_index_order(db: &TestDb) {
    let driver = db.connect().await;
    q(
        &driver,
        "CREATE INDEX orders_total_person ON orders (total, person_id)",
    )
    .await;
    let own = own_catalog(db);
    let indexes = details(&driver, &own, &folded(db, "orders")).await.indexes;
    let mine = indexes
        .iter()
        .find(|i| i.name.eq_ignore_ascii_case("orders_total_person"))
        .unwrap_or_else(|| panic!("no orders_total_person in {indexes:?}"));
    assert_eq!(lower_columns(mine), ["total", "person_id"]);
    assert!(!mine.unique);
    // The primary key's index, where the backend keeps one: SQLite's
    // INTEGER PRIMARY KEY is the rowid and has none.
    if db.backend != Backend::Sqlite {
        assert!(
            indexes
                .iter()
                .any(|i| i.unique && lower_columns(i) == ["id"]),
            "{indexes:?}"
        );
    }
}

async fn a_name_that_needs_quoting_loads_like_any_other(db: &TestDb) {
    let driver = db.connect().await;
    let dialect = db.backend.dialect();
    let name = "Odd \"name\" 'x'";
    let col = "Mixed Case";
    q(
        &driver,
        &format!(
            "CREATE TABLE {} ({} integer PRIMARY KEY)",
            sql::quote_ident(dialect, name),
            sql::quote_ident(dialect, col),
        ),
    )
    .await;
    let own = own_catalog(db);
    let objects = objects(&driver, &own).await;
    assert!(
        objects.iter().any(|o| o.name == name),
        "{name:?} in {objects:?}"
    );
    let details = details(&driver, &own, name).await;
    assert_eq!(details.columns.len(), 1, "{details:?}");
    assert_eq!(details.columns[0].name, col);
    assert!(details.columns[0].primary_key);
    // Selecting it as the tree does, through the qualified name.
    let catalog = (db.backend != Backend::Sqlite).then_some(own.as_str());
    let result = q(
        &driver,
        &format!(
            "SELECT * FROM {}",
            sql::qualified_name(dialect, catalog, name)
        ),
    )
    .await;
    assert_eq!(result.columns.len(), 1);
    assert!(result.rows.is_empty());
}

async fn tables_is_the_default_catalogs_objects(db: &TestDb) {
    let driver = db.connect().await;
    let tables: Vec<String> = driver
        .tables()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.to_lowercase())
        .collect();
    let objects: Vec<String> = objects(&driver, &own_catalog(db))
        .await
        .into_iter()
        .map(|o| o.name.to_lowercase())
        .collect();
    assert_eq!(tables, objects);
}

async fn defaults_are_none_unless_the_column_has_one(db: &TestDb) {
    let driver = db.connect().await;
    q(
        &driver,
        "CREATE TABLE defaults (plain integer, seven integer DEFAULT 7, \
         word varchar(10) DEFAULT 'NULL')",
    )
    .await;
    let own = own_catalog(db);
    let columns = details(&driver, &own, &folded(db, "defaults"))
        .await
        .columns;
    let defaults: Vec<Option<&str>> = columns.iter().map(|c| c.default.as_deref()).collect();
    match db.backend {
        // `DATA_DEFAULT` is a LONG, which the catalog query does not read.
        Backend::Oracle => assert_eq!(defaults, [None, None, None]),
        _ => {
            // Each server writes the expression its own way (`'NULL'`,
            // `NULL`, `'NULL'::character varying`), but a column without a
            // default has none; MariaDB's `NULL` for that is not one.
            assert_eq!(defaults[..2], [None, Some("7")], "{columns:?}");
            assert!(
                defaults[2].is_some_and(|d| d.contains("NULL")),
                "{columns:?}"
            );
        }
    }
    let people = details(&driver, &own, &folded(db, "people")).await.columns;
    assert!(
        people.iter().all(|c| c.default.is_none()),
        "the seed sets no defaults: {people:?}"
    );
}
