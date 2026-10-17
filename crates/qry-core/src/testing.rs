//! Fakes and fixtures for tests that must not touch a database server.
//!
//! Compiled for this crate's own tests and, through the `test-util` feature,
//! for other crates' dev-dependencies. A release build never contains it.
//!
//! ```ignore
//! let db = FakeDatabase::new(DatabaseType::Postgres)
//!     .reply(Reply::Rows(result(&["id"], &[&[Some("1")]])))
//!     .reply(Reply::Error("relation \"nope\" does not exist".into()));
//! let driver = Driver::with_database(Box::new(db.clone()));
//! assert!(driver.query("SELECT id FROM t").await.is_ok());
//! assert!(driver.query("SELECT * FROM nope").await.is_err());
//! assert_eq!(db.calls(), ["SELECT id FROM t", "SELECT * FROM nope"]);
//! ```

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use crate::{
    ColumnMeta, DatabaseType, QueryResult,
    schema::{Catalog, Children, Column, DbObject, Details, Index, ObjectKind, SchemaPath},
    shared::{CancelHandle, Database},
};

/// One `query_bound` call: the SQL and the values bound to it.
pub type BoundCall = (String, Vec<String>);

/// One scripted answer from a [`FakeDatabase`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A result set, as a `SELECT` returns.
    Rows(QueryResult),
    /// A statement that touched this many rows and returned no columns.
    Affected(u64),
    /// The driver failed with this message.
    Error(String),
    /// The inner reply, after this long. It waits on `tokio::time`, so a test
    /// on a paused clock (`#[tokio::test(start_paused = true)]`) moves past
    /// it without waiting in real time.
    Delay(Duration, Box<Reply>),
    /// Never answers, like a server that went quiet mid-query.
    Hang,
}

/// A [`Database`] that answers from a script, for tests that must not touch a
/// server.
///
/// Clones share the script and the call log, so a test can hand one clone to
/// a [`Driver`](crate::Driver) and keep another to look at what it was sent.
#[derive(Debug, Clone)]
pub struct FakeDatabase {
    kind: DatabaseType,
    label: String,
    script: Arc<Mutex<VecDeque<Reply>>>,
    calls: Arc<Mutex<Vec<String>>>,
    /// Every `query_bound` call: its SQL and the values bound to it.
    bound: Arc<Mutex<Vec<BoundCall>>>,
    tables: Vec<String>,
    /// Wakes the statement in progress when its cancel handle is used.
    cancel: Arc<tokio::sync::Notify>,
    cancels: Arc<Mutex<usize>>,
    schema: Option<FakeSchema>,
    version: String,
    /// What [`Database::streams_rows`] says; on unless turned off.
    streams: bool,
    /// Every [`Database::set_autocommit`] call, in order.
    autocommits: Arc<Mutex<Vec<bool>>>,
}

/// What the fake says when a statement is cancelled, as Postgres does.
pub const CANCELLED: &str = "canceling statement due to user request";

/// A SQLite-flavoured fake with an empty script.
impl Default for FakeDatabase {
    fn default() -> Self {
        Self::new(DatabaseType::Sqlite)
    }
}

/// Locks a mutex even if a panicking test poisoned it: the data inside is a
/// plain queue or log and cannot be left half-written.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl FakeDatabase {
    /// A fake of this kind with nothing scripted. Its label is `fake`.
    pub fn new(kind: DatabaseType) -> Self {
        Self {
            kind,
            label: "fake".to_string(),
            script: Arc::default(),
            calls: Arc::default(),
            bound: Arc::default(),
            tables: Vec::new(),
            cancel: Arc::default(),
            cancels: Arc::default(),
            schema: None,
            version: "fake 1.0".to_string(),
            streams: true,
            autocommits: Arc::default(),
        }
    }

    /// Says it cannot stream rows ([`Database::streams_rows`]), like the
    /// backends whose stream reads the whole result first.
    #[must_use]
    pub fn without_streaming(mut self) -> Self {
        self.streams = false;
        self
    }

    /// Whether it says it streams rows.
    pub fn streams(&self) -> bool {
        self.streams
    }

    /// Every [`Database::set_autocommit`] call so far. They use up no
    /// scripted reply and are not in [`calls`](Self::calls).
    pub fn autocommit_calls(&self) -> Vec<bool> {
        lock(&self.autocommits).clone()
    }

    /// What [`Database::server_version`] answers; `fake 1.0` unless set.
    /// Asking does not use up a scripted reply and is not recorded.
    #[must_use]
    pub fn version(mut self, version: &str) -> Self {
        self.version = version.to_string();
        self
    }

    /// The version it reports, as [`Database::server_version`] returns it.
    pub fn server_version_text(&self) -> String {
        self.version.clone()
    }

    /// Queues the answer to the next statement. Replies are used once each,
    /// in the order they were queued.
    #[must_use]
    pub fn reply(self, reply: Reply) -> Self {
        lock(&self.script).push_back(reply);
        self
    }

    /// The tables [`Database::tables`] lists. Listing them does not use up a
    /// scripted reply and is not recorded in [`calls`](Self::calls).
    #[must_use]
    pub fn tables(mut self, names: &[&str]) -> Self {
        self.tables = names.iter().map(|name| name.to_string()).collect();
        self
    }

    /// How many times its cancel handle was used.
    pub fn cancels(&self) -> usize {
        *lock(&self.cancels)
    }

    /// Answers the schema methods ([`Database::catalogs`] and the rest)
    /// from `schema` instead of running catalog queries. Without one they
    /// run the backend's catalog query through
    /// [`query_bound`](Database::query_bound), answered from the script.
    #[must_use]
    pub fn schema(mut self, schema: FakeSchema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// The schema this fake describes: the one given to
    /// [`schema`](Self::schema), else a `main` catalog holding the
    /// [`tables`](Self::tables).
    pub fn fake_schema(&self) -> FakeSchema {
        match &self.schema {
            Some(schema) => schema.clone(),
            None => {
                let mut schema = FakeSchema::new().catalog("main", true, false);
                for table in &self.tables {
                    schema = schema.table("main", table);
                }
                schema
            }
        }
    }

    /// What loading `path` gives, from [`fake_schema`](Self::fake_schema),
    /// and how many milliseconds it takes (`u64::MAX` never answers). The
    /// app harness answers schema loads with this on its virtual clock.
    pub fn schema_answer(&self, path: &SchemaPath) -> (Result<Children, String>, u64) {
        let schema = self.fake_schema();
        let delay = schema.delays.get(path).copied().unwrap_or(0);
        (schema.load(path), delay)
    }

    /// Every `query_bound` call so far: the SQL and the values bound.
    pub fn bound_calls(&self) -> Vec<BoundCall> {
        lock(&self.bound).clone()
    }

    /// Every SQL text it received, in order.
    pub fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }

    /// Records `sql` and takes the next scripted reply. With nothing left in
    /// the script it answers with an error naming the statement, so a test
    /// that sends more than it scripted fails loudly.
    ///
    /// This is what the [`Database`] methods use; the app harness in the
    /// `qry` crate calls it directly to answer on its own virtual clock.
    pub fn next_reply(&self, sql: &str) -> Reply {
        lock(&self.calls).push(sql.to_string());
        lock(&self.script)
            .pop_front()
            .unwrap_or_else(|| Reply::Error(format!("FakeDatabase: no reply scripted for `{sql}`")))
    }

    /// The listed tables, as [`Database::tables`] returns them.
    pub fn table_names(&self) -> Vec<String> {
        self.tables.clone()
    }

    /// The label a real connection would show, such as `fake`.
    pub fn name(&self) -> String {
        self.label.clone()
    }
}

/// Plays a reply out: waits for any delay, then answers.
async fn resolve(reply: Reply) -> Result<QueryResult> {
    let mut reply = reply;
    loop {
        match reply {
            Reply::Rows(result) => return Ok(result),
            Reply::Affected(count) => {
                return Ok(QueryResult::affected(count));
            }
            Reply::Error(message) => return Err(anyhow!(message)),
            Reply::Delay(after, inner) => {
                tokio::time::sleep(after).await;
                reply = *inner;
            }
            Reply::Hang => std::future::pending::<()>().await,
        }
    }
}

#[async_trait]
impl Database for FakeDatabase {
    fn kind(&self) -> DatabaseType {
        self.kind.clone()
    }

    fn label(&self) -> String {
        self.name()
    }

    async fn query_limited(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        let reply = self.next_reply(sql);
        tokio::select! {
            result = resolve(reply) => Ok(crate::shared::truncate(result?, max_rows)),
            () = self.cancel.notified() => Err(anyhow!(CANCELLED)),
        }
    }

    fn streams_rows(&self) -> bool {
        self.streams
    }

    async fn set_autocommit(&self, on: bool) -> Result<()> {
        lock(&self.autocommits).push(on);
        Ok(())
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(FakeCancel {
            cancel: self.cancel.clone(),
            cancels: self.cancels.clone(),
        }))
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let result = resolve(self.next_reply(sql)).await?;
        Ok(result.affected.unwrap_or(result.rows.len() as u64))
    }

    async fn tables(&self) -> Result<Vec<String>> {
        Ok(self.table_names())
    }

    /// Records the SQL and the values, and answers from the script.
    async fn query_bound(&self, sql: &str, params: &[String]) -> Result<QueryResult> {
        lock(&self.bound).push((sql.to_string(), params.to_vec()));
        resolve(self.next_reply(sql)).await
    }

    async fn catalogs(&self) -> Result<Vec<Catalog>> {
        match &self.schema {
            Some(schema) => match schema.load(&SchemaPath::Catalogs) {
                Ok(Children::Catalogs(c)) => Ok(c),
                Ok(_) => Ok(Vec::new()),
                Err(e) => Err(anyhow!(e)),
            },
            None => crate::schema::load_catalogs(self).await,
        }
    }

    async fn objects(&self, catalog: &str) -> Result<Vec<DbObject>> {
        match &self.schema {
            Some(schema) => match schema.load(&SchemaPath::objects(catalog)) {
                Ok(Children::Objects(o)) => Ok(o),
                Ok(_) => Ok(Vec::new()),
                Err(e) => Err(anyhow!(e)),
            },
            None => crate::schema::load_objects(self, catalog).await,
        }
    }

    async fn columns(&self, catalog: &str, object: &str) -> Result<Vec<Column>> {
        match &self.schema {
            Some(schema) => match schema.load(&SchemaPath::details(catalog, object)) {
                Ok(Children::Details(d)) => Ok(d.columns),
                Ok(_) => Ok(Vec::new()),
                Err(e) => Err(anyhow!(e)),
            },
            None => crate::schema::load_columns(self, catalog, object).await,
        }
    }

    async fn indexes(&self, catalog: &str, object: &str) -> Result<Vec<Index>> {
        match &self.schema {
            Some(schema) => match schema.load(&SchemaPath::details(catalog, object)) {
                Ok(Children::Details(d)) => Ok(d.indexes),
                Ok(_) => Ok(Vec::new()),
                Err(e) => Err(anyhow!(e)),
            },
            None => crate::schema::load_indexes(self, catalog, object).await,
        }
    }

    async fn server_version(&self) -> Result<String> {
        Ok(self.server_version_text())
    }
}

/// A database's structure for a [`FakeDatabase`] to describe, built up a
/// piece at a time:
///
/// ```ignore
/// FakeSchema::new()
///     .catalog("public", true, false)
///     .catalog("pg_catalog", false, true)
///     .table("public", "users")
///     .column("public", "users", FakeSchema::pk("id", "int8"))
///     .fail(SchemaPath::objects("audit"), "permission denied")
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeSchema {
    pub catalogs: Vec<Catalog>,
    pub objects: BTreeMap<String, Vec<DbObject>>,
    pub details: BTreeMap<(String, String), Details>,
    /// Loads that fail, with the message.
    pub errors: BTreeMap<SchemaPath, String>,
    /// Loads that take this many milliseconds; `u64::MAX` never answers.
    pub delays: BTreeMap<SchemaPath, u64>,
}

impl FakeSchema {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a catalog.
    #[must_use]
    pub fn catalog(mut self, name: &str, is_default: bool, is_system: bool) -> Self {
        self.catalogs.push(Catalog {
            name: name.to_string(),
            is_default,
            is_system,
        });
        self.objects.entry(name.to_string()).or_default();
        self
    }

    fn object(mut self, catalog: &str, name: &str, kind: ObjectKind) -> Self {
        self.objects
            .entry(catalog.to_string())
            .or_default()
            .push(DbObject {
                catalog: catalog.to_string(),
                name: name.to_string(),
                kind,
            });
        self
    }

    /// Adds a table to `catalog`.
    #[must_use]
    pub fn table(self, catalog: &str, name: &str) -> Self {
        self.object(catalog, name, ObjectKind::Table)
    }

    /// Adds a view to `catalog`.
    #[must_use]
    pub fn view(self, catalog: &str, name: &str) -> Self {
        self.object(catalog, name, ObjectKind::View)
    }

    /// Adds a column to `catalog.object`; its ordinal follows the others.
    #[must_use]
    pub fn column(mut self, catalog: &str, object: &str, mut column: Column) -> Self {
        let details = self
            .details
            .entry((catalog.to_string(), object.to_string()))
            .or_default();
        column.ordinal = u32::try_from(details.columns.len() + 1).unwrap_or(u32::MAX);
        details.columns.push(column);
        self
    }

    /// Adds an index to `catalog.object`.
    #[must_use]
    pub fn index(mut self, catalog: &str, object: &str, index: Index) -> Self {
        self.details
            .entry((catalog.to_string(), object.to_string()))
            .or_default()
            .indexes
            .push(index);
        self
    }

    /// Makes loading `path` fail with `message`.
    #[must_use]
    pub fn fail(mut self, path: SchemaPath, message: &str) -> Self {
        self.errors.insert(path, message.to_string());
        self
    }

    /// Makes loading `path` take `ms` milliseconds (`u64::MAX`: forever).
    #[must_use]
    pub fn delay(mut self, path: SchemaPath, ms: u64) -> Self {
        self.delays.insert(path, ms);
        self
    }

    /// A plain column that may be NULL.
    pub fn col(name: &str, data_type: &str) -> Column {
        Column {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable: true,
            primary_key: false,
            references: None,
            default: None,
            ordinal: 0,
        }
    }

    /// A primary-key column.
    pub fn pk(name: &str, data_type: &str) -> Column {
        Column {
            nullable: false,
            primary_key: true,
            ..Self::col(name, data_type)
        }
    }

    /// A NOT NULL column that points at `table.column`.
    pub fn fk(name: &str, data_type: &str, table: &str, column: &str) -> Column {
        Column {
            nullable: false,
            references: Some((table.to_string(), column.to_string())),
            ..Self::col(name, data_type)
        }
    }

    /// What loading `path` gives.
    pub fn load(&self, path: &SchemaPath) -> Result<Children, String> {
        if let Some(error) = self.errors.get(path) {
            return Err(error.clone());
        }
        Ok(match path {
            SchemaPath::Catalogs => Children::Catalogs(self.catalogs.clone()),
            SchemaPath::Objects { catalog } => {
                let mut objects = self.objects.get(catalog).cloned().unwrap_or_default();
                objects.sort_by(|a, b| a.name.cmp(&b.name));
                Children::Objects(objects)
            }
            SchemaPath::Details { catalog, object } => Children::Details(
                self.details
                    .get(&(catalog.clone(), object.clone()))
                    .cloned()
                    .unwrap_or_default(),
            ),
        })
    }
}

/// Stops the fake's statement in progress, if any.
struct FakeCancel {
    cancel: Arc<tokio::sync::Notify>,
    cancels: Arc<Mutex<usize>>,
}

#[async_trait]
impl CancelHandle for FakeCancel {
    async fn cancel(&self) -> Result<()> {
        *lock(&self.cancels) += 1;
        // Only a statement already waiting is woken; none is stored up
        // for the next one.
        self.cancel.notify_waiters();
        Ok(())
    }
}

/// A result whose columns have these `(name, database type)`s, and these
/// rows.
pub fn typed_result(columns: &[(&str, &str)], rows: &[&[Option<&str>]]) -> QueryResult {
    let mut result = result(&[], rows);
    result.columns = columns
        .iter()
        .map(|(name, type_name)| ColumnMeta::new(*name, *type_name))
        .collect();
    result
}

/// A result with these columns and rows, for readable fixtures. `None` is a
/// SQL `NULL`.
///
/// ```ignore
/// result(&["id", "name"], &[&[Some("1"), Some("alice")], &[Some("2"), None]])
/// ```
pub fn result(columns: &[&str], rows: &[&[Option<&str>]]) -> QueryResult {
    QueryResult {
        columns: columns.iter().map(|c| ColumnMeta::untyped(*c)).collect(),
        rows: rows
            .iter()
            .map(|row| row.iter().map(|cell| cell.map(str::to_string)).collect())
            .collect(),
        affected: None,
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    }
}

/// `rows` rows of `cols` columns named `c0`, `c1`, … holding `r{i}c{j}`, for
/// tests that need a result of a given size more than particular values.
pub fn grid(rows: usize, cols: usize) -> QueryResult {
    QueryResult {
        columns: (0..cols)
            .map(|c| ColumnMeta::untyped(format!("c{c}")))
            .collect(),
        rows: (0..rows)
            .map(|r| (0..cols).map(|c| Some(format!("r{r}c{c}"))).collect())
            .collect(),
        affected: None,
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    }
}

#[cfg(test)]
#[path = "testing_tests.rs"]
mod testing_tests;
