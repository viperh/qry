//! Unit tests for `schema.rs` and `sql::quote_ident`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    sql::{is_ddl, qualified_name, quote_ident},
    testing::{FakeDatabase, Reply, result},
};

fn catalog(name: &str, is_default: bool) -> Catalog {
    Catalog {
        name: name.into(),
        is_default,
        is_system: false,
    }
}

fn table(catalog: &str, name: &str) -> DbObject {
    DbObject {
        catalog: catalog.into(),
        name: name.into(),
        kind: ObjectKind::Table,
    }
}

fn column(name: &str) -> Column {
    Column {
        name: name.into(),
        data_type: "int".into(),
        nullable: false,
        primary_key: false,
        references: None,
        default: None,
        ordinal: 1,
    }
}

/// A cache with `main` and `audit` loaded, and `main.users`' details.
fn loaded() -> SchemaCache {
    let mut cache = SchemaCache::default();
    let g = cache.begin(&SchemaPath::Catalogs);
    let catalogs = Children::Catalogs(vec![catalog("main", true), catalog("audit", false)]);
    assert!(cache.finish(&SchemaPath::Catalogs, g, Ok(catalogs)));
    for (name, tables) in [("main", vec!["users", "orders"]), ("audit", vec!["log"])] {
        let path = SchemaPath::objects(name);
        let g = cache.begin(&path);
        let objects = tables.into_iter().map(|t| table(name, t)).collect();
        assert!(cache.finish(&path, g, Ok(Children::Objects(objects))));
    }
    let path = SchemaPath::details("main", "users");
    let g = cache.begin(&path);
    let details = Details {
        columns: vec![column("id"), column("email")],
        indexes: vec![],
    };
    assert!(cache.finish(&path, g, Ok(Children::Details(details))));
    cache
}

#[test]
fn cache_invalidate_drops_subtree_only() {
    let mut cache = loaded();
    cache.invalidate(&SchemaPath::objects("main"));

    assert!(cache.objects("main").is_none());
    assert!(cache.details("main", "users").is_none(), "its subtree goes");
    assert_eq!(cache.catalogs().unwrap().len(), 2, "its parent stays");
    assert_eq!(
        cache.objects("audit").unwrap().len(),
        1,
        "its sibling stays"
    );

    let mut cache = loaded();
    cache.invalidate(&SchemaPath::details("main", "users"));
    assert!(cache.details("main", "users").is_none());
    assert_eq!(cache.objects("main").unwrap().len(), 2);

    let mut cache = loaded();
    cache.invalidate(&SchemaPath::Catalogs);
    assert_eq!(
        cache,
        SchemaCache {
            generation: cache.generation,
            ..SchemaCache::default()
        }
    );
}

#[test]
fn cache_late_answer_with_old_generation_is_ignored() {
    let mut cache = SchemaCache::default();
    let path = SchemaPath::objects("main");
    let old = cache.begin(&path);
    let new = cache.begin(&path);
    assert_ne!(old, new);

    let stale = Children::Objects(vec![table("main", "stale")]);
    assert!(
        !cache.finish(&path, old, Ok(stale)),
        "a newer load is awaited"
    );
    assert!(cache.is_loading(&path));

    let fresh = Children::Objects(vec![table("main", "fresh")]);
    assert!(cache.finish(&path, new, Ok(fresh)));
    assert_eq!(cache.objects("main").unwrap()[0].name, "fresh");
    assert!(!cache.is_loading(&path));

    // Invalidated while in flight: the answer finds nobody waiting.
    let g = cache.begin(&path);
    cache.invalidate(&path);
    assert!(!cache.finish(&path, g, Ok(Children::Objects(vec![]))));
    assert!(!cache.is_known(&path));

    // Cleared (a reconnect) while in flight: the same.
    let g = cache.begin(&SchemaPath::Catalogs);
    cache.clear();
    assert!(!cache.finish(&SchemaPath::Catalogs, g, Ok(Children::Catalogs(vec![]))));
}

#[test]
fn cache_failed_reload_keeps_previous_children() {
    let mut cache = loaded();
    let path = SchemaPath::objects("main");
    let g = cache.begin(&path);
    assert_eq!(
        cache.objects("main").unwrap().len(),
        2,
        "kept while loading"
    );
    assert!(cache.finish(&path, g, Err("connection reset".into())));

    let entry = cache.objects_entry("main").unwrap();
    assert_eq!(entry.error.as_deref(), Some("connection reset"));
    assert_eq!(
        cache.objects("main").unwrap().len(),
        2,
        "kept after failing"
    );
    assert!(
        cache.details("main", "users").is_some(),
        "and what is under it"
    );

    // The next try clears the error while it runs.
    let g = cache.begin(&path);
    assert!(cache.objects_entry("main").unwrap().error.is_none());
    let objects = Children::Objects(vec![table("main", "users")]);
    assert!(cache.finish(&path, g, Ok(objects)));
    assert!(
        cache.details("main", "users").is_none(),
        "a reload that works drops what was loaded under it"
    );
}

#[test]
fn cache_answer_of_the_wrong_shape_is_ignored() {
    let mut cache = SchemaCache::default();
    let g = cache.begin(&SchemaPath::Catalogs);
    assert!(!cache.finish(&SchemaPath::Catalogs, g, Ok(Children::Objects(vec![]))));
    assert!(cache.any_loading());
}

#[test]
fn completion_names_include_tables_columns_and_catalogs() {
    let names = loaded().names_for_completion();
    assert_eq!(
        names,
        ["audit", "email", "id", "log", "main", "orders", "users"]
    );
}

#[test]
fn quote_ident_postgres_plain_lowercase_unquoted() {
    assert_eq!(quote_ident(Dialect::Postgres, "users"), "users");
    assert_eq!(
        quote_ident(Dialect::Postgres, "order_items2"),
        "order_items2"
    );
    assert_eq!(quote_ident(Dialect::Postgres, "_tmp"), "_tmp");
}

#[test]
fn quote_ident_postgres_mixed_case_quoted() {
    assert_eq!(quote_ident(Dialect::Postgres, "Users"), "\"Users\"");
    assert_eq!(quote_ident(Dialect::Postgres, "my table"), "\"my table\"");
    assert_eq!(quote_ident(Dialect::Postgres, "2fa"), "\"2fa\"");
    assert_eq!(
        quote_ident(Dialect::Postgres, "order"),
        "\"order\"",
        "a keyword"
    );
    assert_eq!(quote_ident(Dialect::Postgres, "café"), "\"café\"");
    assert_eq!(quote_ident(Dialect::Postgres, ""), "\"\"");
}

#[test]
fn quote_ident_mysql_uses_backticks() {
    assert_eq!(
        quote_ident(Dialect::MySql, "Users"),
        "Users",
        "case is kept"
    );
    assert_eq!(quote_ident(Dialect::MySql, "my table"), "`my table`");
    assert_eq!(quote_ident(Dialect::MySql, "a`b"), "`a``b`");
}

#[test]
fn quote_ident_doubles_embedded_quotes() {
    assert_eq!(quote_ident(Dialect::Postgres, "a\"b"), "\"a\"\"b\"");
    assert_eq!(
        quote_ident(Dialect::Sqlite, "x'; DROP TABLE t; --"),
        "\"x'; DROP TABLE t; --\""
    );
    assert_eq!(
        quote_ident(Dialect::Oracle, "say \"hi\""),
        "\"say \"\"hi\"\"\""
    );
}

#[test]
fn quote_ident_oracle_uppercase_unquoted() {
    assert_eq!(quote_ident(Dialect::Oracle, "EMPLOYEES"), "EMPLOYEES");
    assert_eq!(quote_ident(Dialect::Oracle, "HR_2"), "HR_2");
    assert_eq!(quote_ident(Dialect::Oracle, "employees"), "\"employees\"");
    assert_eq!(quote_ident(Dialect::Oracle, "MixedCase"), "\"MixedCase\"");
}

#[test]
fn quote_ident_sqlite_ignores_case() {
    assert_eq!(quote_ident(Dialect::Sqlite, "Users"), "Users");
    assert_eq!(
        qualified_name(Dialect::Postgres, Some("Audit"), "log"),
        "\"Audit\".log"
    );
    assert_eq!(qualified_name(Dialect::Sqlite, None, "users"), "users");
}

#[test]
fn ddl_detection_create_alter_drop_only() {
    for dialect in Dialect::ALL {
        assert!(is_ddl(dialect, "CREATE TABLE t (x int)"));
        assert!(is_ddl(dialect, "  alter table t add y int"));
        assert!(is_ddl(dialect, "-- note\nDROP VIEW v"));
        assert!(!is_ddl(dialect, "SELECT 'drop table t'"));
        assert!(!is_ddl(dialect, "INSERT INTO t VALUES (1)"));
        assert!(!is_ddl(dialect, "/* create */ UPDATE t SET x = 1"));
    }
}

#[test]
fn object_kinds_read_every_backends_names() {
    assert_eq!(ObjectKind::from_type_name("BASE TABLE"), ObjectKind::Table);
    assert_eq!(ObjectKind::from_type_name("table"), ObjectKind::Table);
    assert_eq!(ObjectKind::from_type_name("virtual"), ObjectKind::Table);
    assert_eq!(ObjectKind::from_type_name("VIEW"), ObjectKind::View);
    assert_eq!(ObjectKind::from_type_name("SYSTEM VIEW"), ObjectKind::View);
    assert_eq!(
        ObjectKind::from_type_name("MATERIALIZED VIEW"),
        ObjectKind::MaterializedView
    );
    assert!(ObjectKind::MaterializedView.is_view());
    assert!(!ObjectKind::Table.is_view());
}

#[test]
fn every_query_binds_names_and_never_writes_them_into_the_sql() {
    let hostile = "x'; DROP TABLE t; --";
    for dialect in Dialect::ALL {
        let queries = [
            objects_query(dialect, hostile),
            columns_query(dialect, hostile, hostile),
            indexes_query(dialect, hostile, hostile),
        ];
        for q in queries {
            assert!(!q.sql.contains("DROP TABLE t"), "{dialect:?}: {}", q.sql);
            assert!(!q.params.is_empty(), "{dialect:?}: {}", q.sql);
            assert!(q.params.iter().all(|p| p == hostile), "{dialect:?}");
            let placeholders = match dialect {
                Dialect::Postgres => (1..=q.params.len())
                    .filter(|n| q.sql.contains(&format!("${n}::text")))
                    .count(),
                Dialect::MySql => q.sql.matches('?').count(),
                Dialect::Sqlite => (1..=q.params.len())
                    .filter(|n| q.sql.contains(&format!("?{n}")))
                    .count(),
                Dialect::Oracle => (1..=q.params.len())
                    .filter(|n| q.sql.contains(&format!(":{n}")))
                    .count(),
            };
            assert_eq!(placeholders, q.params.len(), "{dialect:?}: {}", q.sql);
        }
        assert!(catalogs_query(dialect).params.is_empty());
    }
}

#[test]
fn parsers_read_the_shared_row_shape() {
    let catalogs = result(
        &["name", "is_default", "is_system"],
        &[
            &[Some("pg_catalog"), Some("false"), Some("true")],
            &[Some("public"), Some("true"), Some("false")],
            &[None, Some("false"), Some("false")],
        ],
    );
    let parsed = parse_catalogs(&catalogs);
    assert_eq!(parsed.len(), 2, "a row without a name is skipped");
    assert_eq!(
        parsed[0],
        catalog("public", true),
        "the default comes first"
    );
    assert!(parsed[1].is_system);

    let columns = result(
        &["n", "t", "null", "def", "ord", "pk", "rt", "rc"],
        &[
            &[
                Some("id"),
                Some("int8"),
                Some("false"),
                None,
                Some("1"),
                Some("true"),
                None,
                None,
            ],
            &[
                Some("user_id"),
                Some("int8"),
                Some("YES"),
                Some("0"),
                Some("2"),
                Some("f"),
                Some("users"),
                Some("id"),
            ],
            // The same column under a second foreign key.
            &[
                Some("user_id"),
                Some("int8"),
                Some("YES"),
                None,
                Some("2"),
                Some("f"),
                Some("people"),
                Some("id"),
            ],
        ],
    );
    let parsed = parse_columns(&columns);
    assert_eq!(parsed.len(), 2);
    assert!(parsed[0].primary_key && !parsed[0].nullable);
    assert_eq!(parsed[1].references, Some(("users".into(), "id".into())));
    assert!(parsed[1].nullable);
    assert_eq!(parsed[1].default.as_deref(), Some("0"));
    assert_eq!(parsed[1].ordinal, 2);

    let indexes = result(
        &["i", "u", "c"],
        &[
            &[Some("orders_pkey"), Some("true"), Some("id")],
            &[Some("orders_user_total"), Some("false"), Some("user_id")],
            &[Some("orders_user_total"), Some("false"), Some("total")],
            &[Some("orders_lower"), Some("0"), None],
        ],
    );
    let parsed = parse_indexes(&indexes);
    assert_eq!(parsed.len(), 3);
    assert!(parsed[0].unique);
    assert_eq!(parsed[1].columns, ["user_id", "total"]);
    assert_eq!(parsed[2].columns, ["(expression)"]);
}

/// Each backend without a server: the fake answers the catalog query the
/// backend's dialect sends, and the parser reads it.
#[tokio::test]
async fn every_backend_loads_through_its_catalog_queries() {
    for kind in [
        DatabaseType::Postgres,
        DatabaseType::Mysql,
        DatabaseType::MariaDb,
        DatabaseType::Sqlite,
        DatabaseType::Oracle,
    ] {
        let db = FakeDatabase::new(kind.clone())
            .reply(Reply::Rows(result(
                &["n", "d", "s"],
                &[&[Some("app"), Some("true"), Some("false")]],
            )))
            .reply(Reply::Rows(result(
                &["n", "t"],
                &[
                    &[Some("users"), Some("BASE TABLE")],
                    &[Some("v"), Some("VIEW")],
                ],
            )))
            .reply(Reply::Rows(result(
                &["n", "t", "null", "def", "ord", "pk", "rt", "rc"],
                &[&[
                    Some("id"),
                    Some("integer"),
                    Some("false"),
                    None,
                    Some("1"),
                    Some("true"),
                    None,
                    None,
                ]],
            )))
            .reply(Reply::Rows(result(
                &["i", "u", "c"],
                &[&[Some("users_pkey"), Some("true"), Some("id")]],
            )));

        let catalogs = db.catalogs().await.unwrap();
        assert_eq!(catalogs, [catalog("app", true)], "{kind:?}");
        let objects = db.objects("app").await.unwrap();
        assert_eq!(objects[1].kind, ObjectKind::View, "{kind:?}");
        let columns = db.columns("app", "users").await.unwrap();
        assert!(columns[0].primary_key, "{kind:?}");
        let indexes = db.indexes("app", "users").await.unwrap();
        assert_eq!(indexes[0].columns, ["id"], "{kind:?}");

        let dialect = kind.dialect();
        let sent: Vec<String> = db.bound_calls().into_iter().map(|(sql, _)| sql).collect();
        assert_eq!(
            sent,
            [
                catalogs_query(dialect).sql,
                objects_query(dialect, "app").sql,
                columns_query(dialect, "app", "users").sql,
                indexes_query(dialect, "app", "users").sql,
            ],
            "{kind:?}"
        );
    }
}

#[tokio::test]
async fn hostile_names_reach_every_backend_only_as_bound_values() {
    let hostile = "x'; DROP TABLE t; --";
    for kind in [
        DatabaseType::Postgres,
        DatabaseType::Mysql,
        DatabaseType::MariaDb,
        DatabaseType::Oracle,
    ] {
        let empty = || Reply::Rows(result(&["a"], &[]));
        let db = FakeDatabase::new(kind.clone())
            .reply(empty())
            .reply(empty())
            .reply(empty());
        db.objects(hostile).await.unwrap();
        db.columns(hostile, hostile).await.unwrap();
        db.indexes(hostile, hostile).await.unwrap();
        for (sql, params) in db.bound_calls() {
            assert!(!sql.contains(hostile), "{kind:?}");
            assert!(params.iter().all(|p| p == hostile), "{kind:?}");
        }
    }
}

#[tokio::test]
async fn load_goes_through_the_schema_methods() {
    let db = FakeDatabase::default().schema(
        crate::testing::FakeSchema::new()
            .catalog("main", true, false)
            .table("main", "users")
            .column(
                "main",
                "users",
                crate::testing::FakeSchema::pk("id", "integer"),
            )
            .fail(SchemaPath::objects("broken"), "no such schema"),
    );
    let Children::Catalogs(catalogs) = load(&db, &SchemaPath::Catalogs).await.unwrap() else {
        panic!("catalogs");
    };
    assert_eq!(catalogs[0].name, "main");
    let Children::Details(details) = load(&db, &SchemaPath::details("main", "users"))
        .await
        .unwrap()
    else {
        panic!("details");
    };
    assert_eq!(details.columns[0].ordinal, 1);
    let error = load(&db, &SchemaPath::objects("broken")).await.unwrap_err();
    assert_eq!(error.to_string(), "no such schema");
    assert!(db.calls().is_empty(), "a fake schema runs no SQL");
}
