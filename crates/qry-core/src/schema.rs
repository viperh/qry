//! What a database holds, for the schema browser: catalogs, the tables and
//! views in each, and their columns and indexes.
//!
//! - The types ([`Catalog`], [`DbObject`], [`Column`], [`Index`]) are what
//!   the [`Database`] methods return.
//! - [`catalogs_query`] and friends build each backend's catalog query as a
//!   [`MetaQuery`]: constant SQL plus the names to bind. No name is ever
//!   written into the SQL text, so a hostile one cannot change what runs.
//! - The `parse_*` functions read the rows those queries return. Every
//!   backend's query returns the same shape, so one parser serves all.
//! - [`SchemaCache`] holds what a session has loaded so far, keyed by
//!   [`SchemaPath`], with the generations that tell a late answer from the
//!   live one.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::{DatabaseType, QueryResult, shared::Database, sql::Dialect};

/// A namespace of tables: a Postgres schema, a MySQL or MariaDB database,
/// an Oracle owner, or a SQLite database (`main`, attached ones).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub name: String,
    /// The connection's own: its `search_path` schema, its database, its
    /// current schema, SQLite's `main`.
    pub is_default: bool,
    /// The database's own bookkeeping (`pg_catalog`, `mysql`, `SYS`), hidden
    /// unless asked for.
    pub is_system: bool,
}

/// What kind of object a [`DbObject`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
}

impl ObjectKind {
    /// The kind a catalog calls `name`: `BASE TABLE`, `VIEW`, `table`,
    /// `MATERIALIZED VIEW`. Anything else that holds rows is a table.
    pub fn from_type_name(name: &str) -> Self {
        let upper = name.trim().to_ascii_uppercase();
        if upper.contains("MATERIALIZED") {
            Self::MaterializedView
        } else if upper.contains("VIEW") {
            Self::View
        } else {
            Self::Table
        }
    }

    /// Whether it goes in the Views folder.
    pub fn is_view(self) -> bool {
        matches!(self, Self::View | Self::MaterializedView)
    }
}

/// A table or view in a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbObject {
    pub catalog: String,
    pub name: String,
    pub kind: ObjectKind,
}

/// A column of a table or view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    /// The `(table, column)` a foreign key points at. The column is empty
    /// when the backend does not say (SQLite, for a key to the primary key).
    pub references: Option<(String, String)>,
    pub default: Option<String>,
    /// From 1, in table order.
    pub ordinal: u32,
}

/// An index of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    /// In index order. An expression is `(expression)`.
    pub columns: Vec<String>,
    pub unique: bool,
}

/// One object's columns and indexes, loaded together when it is expanded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
}

/// Something that can be loaded: a connection's catalogs, a catalog's
/// objects, or an object's columns and indexes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaPath {
    Catalogs,
    Objects { catalog: String },
    Details { catalog: String, object: String },
}

impl SchemaPath {
    /// The objects of `catalog`.
    pub fn objects(catalog: impl Into<String>) -> Self {
        Self::Objects {
            catalog: catalog.into(),
        }
    }

    /// The columns and indexes of `catalog.object`.
    pub fn details(catalog: impl Into<String>, object: impl Into<String>) -> Self {
        Self::Details {
            catalog: catalog.into(),
            object: object.into(),
        }
    }

    /// Whether `self` is `ancestor` or lies under it.
    pub fn is_within(&self, ancestor: &SchemaPath) -> bool {
        match (ancestor, self) {
            (SchemaPath::Catalogs, _) => true,
            (SchemaPath::Objects { catalog: a }, SchemaPath::Objects { catalog: b })
            | (SchemaPath::Objects { catalog: a }, SchemaPath::Details { catalog: b, .. }) => {
                a == b
            }
            (
                SchemaPath::Details {
                    catalog: a,
                    object: x,
                },
                SchemaPath::Details {
                    catalog: b,
                    object: y,
                },
            ) => a == b && x == y,
            _ => false,
        }
    }
}

/// What loading a [`SchemaPath`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Children {
    Catalogs(Vec<Catalog>),
    Objects(Vec<DbObject>),
    Details(Details),
}

/// What is known of one loadable node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<T> {
    /// What the last load that worked gave.
    pub value: Option<T>,
    /// The generation of the load in flight, if one is.
    pub loading: Option<u64>,
    /// Why the last load failed, if it did. What loaded before stays.
    pub error: Option<String>,
}

impl<T> Default for Entry<T> {
    fn default() -> Self {
        Self {
            value: None,
            loading: None,
            error: None,
        }
    }
}

/// The parts of the schema a session has loaded, for the tree and for
/// completion. Plain data: the reducer asks for loads with the generation
/// [`begin`](Self::begin) hands out and passes each answer to
/// [`finish`](Self::finish), which ignores one that is no longer awaited.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaCache {
    generation: u64,
    catalogs: Option<Entry<Vec<Catalog>>>,
    objects: BTreeMap<String, Entry<Vec<DbObject>>>,
    details: BTreeMap<String, BTreeMap<String, Entry<Details>>>,
}

impl SchemaCache {
    /// Starts loading `path` and returns the generation its answer must
    /// carry. What was loaded before stays until the answer arrives, and a
    /// previous error is cleared.
    pub fn begin(&mut self, path: &SchemaPath) -> u64 {
        self.generation += 1;
        let generation = self.generation;
        macro_rules! start {
            ($entry:expr) => {{
                let entry = $entry;
                entry.loading = Some(generation);
                entry.error = None;
            }};
        }
        match path {
            SchemaPath::Catalogs => start!(self.catalogs.get_or_insert_with(Entry::default)),
            SchemaPath::Objects { catalog } => {
                start!(self.objects.entry(catalog.clone()).or_default());
            }
            SchemaPath::Details { catalog, object } => start!(
                self.details
                    .entry(catalog.clone())
                    .or_default()
                    .entry(object.clone())
                    .or_default()
            ),
        }
        generation
    }

    /// Takes the answer to the load of `path` numbered `generation`. An
    /// answer nobody waits for any more (the node was invalidated, reloaded
    /// since, or the cache cleared) changes nothing and gives `false`. A
    /// failure keeps what was loaded before; a success replaces it and drops
    /// what was loaded under it, which may be stale.
    pub fn finish(
        &mut self,
        path: &SchemaPath,
        generation: u64,
        outcome: Result<Children, String>,
    ) -> bool {
        fn apply<T>(
            entry: Option<&mut Entry<T>>,
            generation: u64,
            outcome: Result<T, String>,
        ) -> bool {
            let Some(entry) = entry.filter(|e| e.loading == Some(generation)) else {
                return false;
            };
            entry.loading = None;
            match outcome {
                Ok(value) => {
                    entry.value = Some(value);
                    entry.error = None;
                }
                Err(error) => entry.error = Some(error),
            }
            true
        }
        let succeeded = outcome.is_ok();
        let applied = match (path, outcome) {
            (SchemaPath::Catalogs, Ok(Children::Catalogs(v))) => {
                apply(self.catalogs.as_mut(), generation, Ok(v))
            }
            (SchemaPath::Catalogs, Err(e)) => apply(self.catalogs.as_mut(), generation, Err(e)),
            (SchemaPath::Objects { catalog }, Ok(Children::Objects(v))) => {
                apply(self.objects.get_mut(catalog), generation, Ok(v))
            }
            (SchemaPath::Objects { catalog }, Err(e)) => {
                apply(self.objects.get_mut(catalog), generation, Err(e))
            }
            (SchemaPath::Details { catalog, object }, Ok(Children::Details(v))) => apply(
                self.details
                    .get_mut(catalog)
                    .and_then(|d| d.get_mut(object)),
                generation,
                Ok(v),
            ),
            (SchemaPath::Details { catalog, object }, Err(e)) => apply(
                self.details
                    .get_mut(catalog)
                    .and_then(|d| d.get_mut(object)),
                generation,
                Err(e),
            ),
            // An answer of the wrong shape for its path.
            _ => false,
        };
        if applied && succeeded {
            self.drop_below(path);
        }
        applied
    }

    /// Forgets `path` and everything under it, so the next look loads it
    /// again. Any load in flight there is forgotten too, and its answer
    /// will be ignored.
    pub fn invalidate(&mut self, path: &SchemaPath) {
        match path {
            SchemaPath::Catalogs => self.clear(),
            SchemaPath::Objects { catalog } => {
                self.objects.remove(catalog);
                self.details.remove(catalog);
            }
            SchemaPath::Details { catalog, object } => {
                if let Some(objects) = self.details.get_mut(catalog) {
                    objects.remove(object);
                }
            }
        }
    }

    /// Forgets everything, as after reconnecting or a DDL statement.
    pub fn clear(&mut self) {
        self.catalogs = None;
        self.objects.clear();
        self.details.clear();
    }

    /// Drops what lies strictly under `path`.
    fn drop_below(&mut self, path: &SchemaPath) {
        match path {
            SchemaPath::Catalogs => {
                self.objects.clear();
                self.details.clear();
            }
            SchemaPath::Objects { catalog } => {
                self.details.remove(catalog);
            }
            SchemaPath::Details { .. } => {}
        }
    }

    /// The catalogs' entry, if they were ever asked for.
    pub fn catalogs_entry(&self) -> Option<&Entry<Vec<Catalog>>> {
        self.catalogs.as_ref()
    }

    /// The objects' entry of `catalog`, if they were ever asked for.
    pub fn objects_entry(&self, catalog: &str) -> Option<&Entry<Vec<DbObject>>> {
        self.objects.get(catalog)
    }

    /// The details' entry of `catalog.object`, if they were ever asked for.
    pub fn details_entry(&self, catalog: &str, object: &str) -> Option<&Entry<Details>> {
        self.details.get(catalog)?.get(object)
    }

    /// The loaded catalogs.
    pub fn catalogs(&self) -> Option<&[Catalog]> {
        self.catalogs.as_ref()?.value.as_deref()
    }

    /// The loaded objects of `catalog`.
    pub fn objects(&self, catalog: &str) -> Option<&[DbObject]> {
        self.objects.get(catalog)?.value.as_deref()
    }

    /// The loaded columns and indexes of `catalog.object`.
    pub fn details(&self, catalog: &str, object: &str) -> Option<&Details> {
        self.details_entry(catalog, object)?.value.as_ref()
    }

    /// Whether `path` was asked for: loaded, loading or failed.
    pub fn is_known(&self, path: &SchemaPath) -> bool {
        match path {
            SchemaPath::Catalogs => self.catalogs.is_some(),
            SchemaPath::Objects { catalog } => self.objects.contains_key(catalog),
            SchemaPath::Details { catalog, object } => {
                self.details_entry(catalog, object).is_some()
            }
        }
    }

    /// Whether a load of `path` is in flight.
    pub fn is_loading(&self, path: &SchemaPath) -> bool {
        match path {
            SchemaPath::Catalogs => self.catalogs.as_ref().is_some_and(|e| e.loading.is_some()),
            SchemaPath::Objects { catalog } => self
                .objects
                .get(catalog)
                .is_some_and(|e| e.loading.is_some()),
            SchemaPath::Details { catalog, object } => self
                .details_entry(catalog, object)
                .is_some_and(|e| e.loading.is_some()),
        }
    }

    /// Whether any load is in flight.
    pub fn any_loading(&self) -> bool {
        self.catalogs.as_ref().is_some_and(|e| e.loading.is_some())
            || self.objects.values().any(|e| e.loading.is_some())
            || self
                .details
                .values()
                .flat_map(BTreeMap::values)
                .any(|e| e.loading.is_some())
    }

    /// Every loaded object, catalog by catalog.
    pub fn loaded_objects(&self) -> impl Iterator<Item = &DbObject> + '_ {
        self.objects
            .values()
            .filter_map(|e| e.value.as_ref())
            .flatten()
    }

    /// The names an editor can complete: catalogs, objects and columns,
    /// sorted and without repeats.
    pub fn names_for_completion(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .catalogs()
            .unwrap_or_default()
            .iter()
            .map(|c| c.name.clone())
            .chain(self.loaded_objects().map(|o| o.name.clone()))
            .chain(
                self.details
                    .values()
                    .flat_map(BTreeMap::values)
                    .filter_map(|e| e.value.as_ref())
                    .flat_map(|d| d.columns.iter().map(|c| c.name.clone())),
            )
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

/// A catalog query: constant SQL with placeholders, and the names to bind
/// to them, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaQuery {
    pub sql: &'static str,
    pub params: Vec<String>,
}

impl MetaQuery {
    fn new(sql: &'static str, params: &[&str]) -> Self {
        Self {
            sql,
            params: params.iter().map(|p| p.to_string()).collect(),
        }
    }
}

impl DatabaseType {
    /// The SQL dialect this backend speaks.
    pub fn dialect(&self) -> Dialect {
        match self {
            DatabaseType::Postgres => Dialect::Postgres,
            DatabaseType::Mysql | DatabaseType::MariaDb => Dialect::MySql,
            DatabaseType::Sqlite => Dialect::Sqlite,
            DatabaseType::Oracle => Dialect::Oracle,
        }
    }
}

// Every query below returns the shape its parser reads, column by column;
// flags are the text `true` or `false`.

const PG_CATALOGS: &str = "\
SELECT n.nspname::text,
       (n.nspname = current_schema())::text,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\\_%')::text
FROM pg_namespace n
ORDER BY (n.nspname = current_schema()) DESC, n.nspname";

const PG_OBJECTS: &str = "\
SELECT t.table_name::text, t.table_type::text
FROM information_schema.tables t
WHERE t.table_schema::text = $1::text
UNION ALL
SELECT m.matviewname::text, 'MATERIALIZED VIEW'
FROM pg_matviews m
WHERE m.schemaname::text = $1::text
ORDER BY 1";

const PG_COLUMNS: &str = "\
SELECT c.column_name::text,
       c.udt_name::text,
       (c.is_nullable = 'YES')::text,
       c.column_default::text,
       c.ordinal_position::text,
       EXISTS (
         SELECT 1
         FROM pg_constraint k
         JOIN pg_class t ON t.oid = k.conrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY (k.conkey)
         WHERE k.contype = 'p'
           AND n.nspname::text = c.table_schema::text
           AND t.relname::text = c.table_name::text
           AND a.attname::text = c.column_name::text
       )::text,
       fk.ref_table,
       fk.ref_column
FROM information_schema.columns c
LEFT JOIN LATERAL (
  SELECT ft.relname::text AS ref_table, fa.attname::text AS ref_column
  FROM pg_constraint k
  JOIN pg_class t ON t.oid = k.conrelid
  JOIN pg_namespace n ON n.oid = t.relnamespace
  JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.conkey[1]
  JOIN pg_class ft ON ft.oid = k.confrelid
  JOIN pg_attribute fa ON fa.attrelid = k.confrelid AND fa.attnum = k.confkey[1]
  WHERE k.contype = 'f'
    AND cardinality(k.conkey) = 1
    AND n.nspname::text = c.table_schema::text
    AND t.relname::text = c.table_name::text
    AND a.attname::text = c.column_name::text
  LIMIT 1
) fk ON true
WHERE c.table_schema::text = $1::text AND c.table_name::text = $2::text
ORDER BY c.ordinal_position";

const PG_INDEXES: &str = "\
SELECT i.relname::text, ix.indisunique::text, a.attname::text
FROM pg_index ix
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN LATERAL unnest(ix.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) ON true
LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
WHERE n.nspname::text = $1::text AND t.relname::text = $2::text
ORDER BY i.relname, k.ord";

const MY_CATALOGS: &str = "\
SELECT SCHEMA_NAME,
       IF(SCHEMA_NAME = DATABASE(), 'true', 'false'),
       IF(SCHEMA_NAME IN ('mysql', 'information_schema', 'performance_schema', 'sys'), 'true', 'false')
FROM information_schema.SCHEMATA
ORDER BY (SCHEMA_NAME = DATABASE()) DESC, SCHEMA_NAME";

const MY_OBJECTS: &str = "\
SELECT TABLE_NAME, TABLE_TYPE
FROM information_schema.TABLES
WHERE TABLE_SCHEMA = ?
ORDER BY TABLE_NAME";

const MY_COLUMNS: &str = "\
SELECT c.COLUMN_NAME,
       c.COLUMN_TYPE,
       IF(c.IS_NULLABLE = 'YES', 'true', 'false'),
       c.COLUMN_DEFAULT,
       CAST(c.ORDINAL_POSITION AS CHAR),
       IF(c.COLUMN_KEY = 'PRI', 'true', 'false'),
       k.REFERENCED_TABLE_NAME,
       k.REFERENCED_COLUMN_NAME
FROM information_schema.COLUMNS c
LEFT JOIN information_schema.KEY_COLUMN_USAGE k
  ON k.TABLE_SCHEMA = c.TABLE_SCHEMA
 AND k.TABLE_NAME = c.TABLE_NAME
 AND k.COLUMN_NAME = c.COLUMN_NAME
 AND k.REFERENCED_TABLE_NAME IS NOT NULL
WHERE c.TABLE_SCHEMA = ? AND c.TABLE_NAME = ?
ORDER BY c.ORDINAL_POSITION";

const MY_INDEXES: &str = "\
SELECT INDEX_NAME, IF(NON_UNIQUE = 0, 'true', 'false'), COLUMN_NAME
FROM information_schema.STATISTICS
WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
ORDER BY INDEX_NAME, SEQ_IN_INDEX";

const SQLITE_CATALOGS: &str = "\
SELECT name,
       CASE WHEN name = 'main' THEN 'true' ELSE 'false' END,
       CASE WHEN name = 'temp' THEN 'true' ELSE 'false' END
FROM pragma_database_list
ORDER BY seq";

const SQLITE_OBJECTS: &str = "\
SELECT name, type
FROM pragma_table_list
WHERE schema = ?1 AND type IN ('table', 'view', 'virtual') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
ORDER BY name";

const SQLITE_COLUMNS: &str = "\
SELECT c.name,
       c.type,
       CASE WHEN c.\"notnull\" = 0 AND c.pk = 0 THEN 'true' ELSE 'false' END,
       c.dflt_value,
       c.cid + 1,
       CASE WHEN c.pk > 0 THEN 'true' ELSE 'false' END,
       f.\"table\",
       f.\"to\"
FROM pragma_table_info(?1, ?2) c
LEFT JOIN pragma_foreign_key_list(?1, ?2) f ON f.\"from\" = c.name
ORDER BY c.cid";

const SQLITE_INDEXES: &str = "\
SELECT l.name,
       CASE WHEN l.\"unique\" THEN 'true' ELSE 'false' END,
       i.name
FROM pragma_index_list(?1, ?2) l
JOIN pragma_index_info(l.name, ?2) i
ORDER BY l.name, i.seqno";

const ORA_CATALOGS: &str = "\
SELECT u.username,
       CASE WHEN u.username = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') THEN 'true' ELSE 'false' END,
       CASE WHEN u.oracle_maintained = 'Y'
              OR u.username IN ('SYS', 'SYSTEM', 'XDB', 'MDSYS', 'CTXSYS', 'OUTLN', 'DBSNMP', 'ORDSYS')
            THEN 'true' ELSE 'false' END
FROM all_users u
WHERE u.username = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')
   OR EXISTS (
     SELECT 1 FROM all_objects o
     WHERE o.owner = u.username AND o.object_type IN ('TABLE', 'VIEW', 'MATERIALIZED VIEW')
   )
ORDER BY CASE WHEN u.username = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') THEN 0 ELSE 1 END,
         u.username";

const ORA_OBJECTS: &str = "\
SELECT table_name, 'TABLE' FROM all_tables
WHERE owner = :1
  AND table_name NOT IN (SELECT mview_name FROM all_mviews WHERE owner = :2)
UNION ALL
SELECT view_name, 'VIEW' FROM all_views WHERE owner = :3
UNION ALL
SELECT mview_name, 'MATERIALIZED VIEW' FROM all_mviews WHERE owner = :4
ORDER BY 1";

const ORA_COLUMNS: &str = "\
SELECT c.column_name,
       c.data_type,
       CASE WHEN c.nullable = 'Y' THEN 'true' ELSE 'false' END,
       NULL,
       TO_CHAR(c.column_id),
       CASE WHEN EXISTS (
         SELECT 1 FROM all_constraints k
         JOIN all_cons_columns kc ON kc.owner = k.owner AND kc.constraint_name = k.constraint_name
         WHERE k.constraint_type = 'P' AND k.owner = c.owner
           AND k.table_name = c.table_name AND kc.column_name = c.column_name
       ) THEN 'true' ELSE 'false' END,
       (SELECT MIN(rc.table_name) FROM all_constraints k
          JOIN all_cons_columns kc ON kc.owner = k.owner AND kc.constraint_name = k.constraint_name
          JOIN all_cons_columns rc ON rc.owner = k.r_owner
                                  AND rc.constraint_name = k.r_constraint_name
                                  AND rc.position = kc.position
         WHERE k.constraint_type = 'R' AND k.owner = c.owner
           AND k.table_name = c.table_name AND kc.column_name = c.column_name),
       (SELECT MIN(rc.column_name) FROM all_constraints k
          JOIN all_cons_columns kc ON kc.owner = k.owner AND kc.constraint_name = k.constraint_name
          JOIN all_cons_columns rc ON rc.owner = k.r_owner
                                  AND rc.constraint_name = k.r_constraint_name
                                  AND rc.position = kc.position
         WHERE k.constraint_type = 'R' AND k.owner = c.owner
           AND k.table_name = c.table_name AND kc.column_name = c.column_name)
FROM all_tab_columns c
WHERE c.owner = :1 AND c.table_name = :2
ORDER BY c.column_id";

const ORA_INDEXES: &str = "\
SELECT i.index_name,
       CASE WHEN i.uniqueness = 'UNIQUE' THEN 'true' ELSE 'false' END,
       ic.column_name
FROM all_indexes i
JOIN all_ind_columns ic ON ic.index_owner = i.owner AND ic.index_name = i.index_name
WHERE i.table_owner = :1 AND i.table_name = :2
ORDER BY i.index_name, ic.column_position";

/// The query listing the catalogs: `(name, is_default, is_system)`, the
/// default first.
pub fn catalogs_query(dialect: Dialect) -> MetaQuery {
    let sql = match dialect {
        Dialect::Postgres => PG_CATALOGS,
        Dialect::MySql => MY_CATALOGS,
        Dialect::Sqlite => SQLITE_CATALOGS,
        Dialect::Oracle => ORA_CATALOGS,
    };
    MetaQuery::new(sql, &[])
}

/// The query listing the tables and views of `catalog`: `(name, type)`.
pub fn objects_query(dialect: Dialect, catalog: &str) -> MetaQuery {
    match dialect {
        Dialect::Postgres => MetaQuery::new(PG_OBJECTS, &[catalog]),
        Dialect::MySql => MetaQuery::new(MY_OBJECTS, &[catalog]),
        Dialect::Sqlite => MetaQuery::new(SQLITE_OBJECTS, &[catalog]),
        // Oracle binds each placeholder by position.
        Dialect::Oracle => MetaQuery::new(ORA_OBJECTS, &[catalog, catalog, catalog, catalog]),
    }
}

/// The query listing the columns of `catalog.object`: `(name, type,
/// nullable, default, ordinal, primary_key, ref_table, ref_column)`.
pub fn columns_query(dialect: Dialect, catalog: &str, object: &str) -> MetaQuery {
    match dialect {
        Dialect::Postgres => MetaQuery::new(PG_COLUMNS, &[catalog, object]),
        Dialect::MySql => MetaQuery::new(MY_COLUMNS, &[catalog, object]),
        // The pragmas take the table first and the schema second.
        Dialect::Sqlite => MetaQuery::new(SQLITE_COLUMNS, &[object, catalog]),
        Dialect::Oracle => MetaQuery::new(ORA_COLUMNS, &[catalog, object]),
    }
}

/// The query listing the indexes of `catalog.object`, one row per indexed
/// column: `(index, unique, column)`, in index order.
pub fn indexes_query(dialect: Dialect, catalog: &str, object: &str) -> MetaQuery {
    match dialect {
        Dialect::Postgres => MetaQuery::new(PG_INDEXES, &[catalog, object]),
        Dialect::MySql => MetaQuery::new(MY_INDEXES, &[catalog, object]),
        Dialect::Sqlite => MetaQuery::new(SQLITE_INDEXES, &[object, catalog]),
        Dialect::Oracle => MetaQuery::new(ORA_INDEXES, &[catalog, object]),
    }
}

/// Cell `i` of `row` as text, `None` for NULL or a missing cell.
fn cell(row: &[Option<String>], i: usize) -> Option<&str> {
    row.get(i).and_then(|c| c.as_deref())
}

/// A flag as the catalog queries write it, or as a backend might:
/// `true`, `t`, `yes`, `y`, `1`.
fn flag(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "true" | "t" | "yes" | "y" | "1"
        )
    })
}

/// Reads `(name, is_default, is_system)` rows. The default catalog comes
/// first whatever order they arrived in.
pub fn parse_catalogs(result: &QueryResult) -> Vec<Catalog> {
    let mut catalogs: Vec<Catalog> = result
        .rows
        .iter()
        .filter_map(|row| {
            Some(Catalog {
                name: cell(row, 0)?.to_string(),
                is_default: flag(cell(row, 1)),
                is_system: flag(cell(row, 2)),
            })
        })
        .collect();
    // Stable, so the rest keep the query's order.
    catalogs.sort_by_key(|c| !c.is_default);
    catalogs
}

/// Reads `(name, type)` rows of `catalog`.
pub fn parse_objects(catalog: &str, result: &QueryResult) -> Vec<DbObject> {
    result
        .rows
        .iter()
        .filter_map(|row| {
            Some(DbObject {
                catalog: catalog.to_string(),
                name: cell(row, 0)?.to_string(),
                kind: ObjectKind::from_type_name(cell(row, 1).unwrap_or("")),
            })
        })
        .collect()
}

/// Reads `(name, type, nullable, default, ordinal, primary_key, ref_table,
/// ref_column)` rows. A column that two foreign keys name comes once, with
/// the first.
pub fn parse_columns(result: &QueryResult) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    for row in &result.rows {
        let Some(name) = cell(row, 0) else {
            continue;
        };
        if columns.iter().any(|c| c.name == name) {
            continue;
        }
        let ordinal = cell(row, 4)
            .and_then(|o| o.trim().parse().ok())
            .unwrap_or(u32::try_from(columns.len() + 1).unwrap_or(u32::MAX));
        columns.push(Column {
            name: name.to_string(),
            data_type: cell(row, 1).unwrap_or("").to_string(),
            nullable: flag(cell(row, 2)),
            primary_key: flag(cell(row, 5)),
            references: cell(row, 6)
                .map(|table| (table.to_string(), cell(row, 7).unwrap_or("").to_string())),
            default: cell(row, 3).map(str::to_string),
            ordinal,
        });
    }
    columns
}

/// Reads `(index, unique, column)` rows, one per indexed column, grouped by
/// index in the order they came.
pub fn parse_indexes(result: &QueryResult) -> Vec<Index> {
    let mut indexes: Vec<Index> = Vec::new();
    for row in &result.rows {
        let Some(name) = cell(row, 0) else {
            continue;
        };
        let column = cell(row, 2).unwrap_or("(expression)").to_string();
        match indexes.iter_mut().find(|i| i.name == name) {
            Some(index) => index.columns.push(column),
            None => indexes.push(Index {
                name: name.to_string(),
                columns: vec![column],
                unique: flag(cell(row, 1)),
            }),
        }
    }
    indexes
}

/// The catalogs, through the backend's catalog query.
pub async fn load_catalogs<D: Database + ?Sized>(db: &D) -> Result<Vec<Catalog>> {
    let q = catalogs_query(db.kind().dialect());
    let result = db.query_bound(q.sql, &q.params).await?;
    Ok(parse_catalogs(&result))
}

/// The objects of `catalog`, through the backend's catalog query.
pub async fn load_objects<D: Database + ?Sized>(db: &D, catalog: &str) -> Result<Vec<DbObject>> {
    let q = objects_query(db.kind().dialect(), catalog);
    let result = db.query_bound(q.sql, &q.params).await?;
    Ok(parse_objects(catalog, &result))
}

/// The columns of `catalog.object`, through the backend's catalog query.
pub async fn load_columns<D: Database + ?Sized>(
    db: &D,
    catalog: &str,
    object: &str,
) -> Result<Vec<Column>> {
    let q = columns_query(db.kind().dialect(), catalog, object);
    let result = db.query_bound(q.sql, &q.params).await?;
    Ok(parse_columns(&result))
}

/// The indexes of `catalog.object`, through the backend's catalog query.
pub async fn load_indexes<D: Database + ?Sized>(
    db: &D,
    catalog: &str,
    object: &str,
) -> Result<Vec<Index>> {
    let q = indexes_query(db.kind().dialect(), catalog, object);
    let result = db.query_bound(q.sql, &q.params).await?;
    Ok(parse_indexes(&result))
}

/// Loads what `path` names through `db`'s schema methods.
pub async fn load(db: &dyn Database, path: &SchemaPath) -> Result<Children> {
    Ok(match path {
        SchemaPath::Catalogs => Children::Catalogs(db.catalogs().await?),
        SchemaPath::Objects { catalog } => Children::Objects(db.objects(catalog).await?),
        SchemaPath::Details { catalog, object } => Children::Details(Details {
            columns: db.columns(catalog, object).await?,
            indexes: db.indexes(catalog, object).await?,
        }),
    })
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod schema_tests;
