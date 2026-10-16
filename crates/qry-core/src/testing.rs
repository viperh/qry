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
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use crate::{ColumnMeta, DatabaseType, QueryResult, shared::Database};

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
    tables: Vec<String>,
    version: String,
}

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
            tables: Vec::new(),
            version: "fake 1.0".to_string(),
        }
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

    async fn query(&self, sql: &str) -> Result<QueryResult> {
        resolve(self.next_reply(sql)).await
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let result = resolve(self.next_reply(sql)).await?;
        Ok(result.affected.unwrap_or(result.rows.len() as u64))
    }

    async fn tables(&self) -> Result<Vec<String>> {
        Ok(self.table_names())
    }

    async fn server_version(&self) -> Result<String> {
        Ok(self.server_version_text())
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
