use std::{fmt::Debug, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    DatabaseType, schema,
    schema::{Catalog, Column, DbObject, Index},
};

/// When to use TLS. Same names and meaning as libpq's `sslmode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Allow,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 6] = [
        SslMode::Disable,
        SslMode::Allow,
        SslMode::Prefer,
        SslMode::Require,
        SslMode::VerifyCa,
        SslMode::VerifyFull,
    ];

    pub fn ensure_supported(self) -> Result<()> {
        match self {
            SslMode::Disable | SslMode::Allow | SslMode::Prefer => Ok(()),
            _ => bail!("SSL mode `{self}` needs TLS, which qry does not support yet"),
        }
    }
}

impl std::fmt::Display for SslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SslMode::Disable => "disable",
            SslMode::Allow => "allow",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        })
    }
}

/// Stops the statement a connection is running, from outside it: taken
/// before the query starts, so it never waits for the busy connection.
#[async_trait]
pub trait CancelHandle: Send + Sync {
    async fn cancel(&self) -> Result<()>;
}

/// Keeps the first `max_rows` rows of `result` (all for 0), and says so
/// if there were more.
pub fn truncate(mut result: QueryResult, max_rows: usize) -> QueryResult {
    if max_rows > 0 && result.rows.len() > max_rows {
        result.rows.truncate(max_rows);
        result.truncated = true;
    }
    result
}

/// A piece of a streamed result: its columns once, first, then its rows in
/// batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    Columns(Vec<ColumnMeta>),
    Rows(Vec<Vec<Option<String>>>),
}

/// Where a streamed result goes. Bounded, so a reader that falls behind
/// holds the query back rather than letting rows pile up.
pub type RowSender = tokio::sync::mpsc::Sender<StreamItem>;

/// Sends `result` down `out` as [`Database::query_stream`] would: the
/// columns, then the rows `fetch_size` at a time. Returns the rows sent.
pub async fn send_result(result: QueryResult, fetch_size: usize, out: &RowSender) -> Result<u64> {
    let fetch_size = fetch_size.max(1);
    let total = result.rows.len() as u64;
    send(out, StreamItem::Columns(result.columns)).await?;
    let mut rows = result.rows.into_iter().peekable();
    while rows.peek().is_some() {
        let batch: Vec<_> = rows.by_ref().take(fetch_size).collect();
        send(out, StreamItem::Rows(batch)).await?;
    }
    Ok(total)
}

/// Sends one item, or says the reader has gone.
pub(crate) async fn send(out: &RowSender, item: StreamItem) -> Result<()> {
    if out.send(item).await.is_err() {
        bail!("the rows' reader stopped");
    }
    Ok(())
}

#[async_trait]
pub trait Database: Send + Sync {
    fn kind(&self) -> DatabaseType;
    fn label(&self) -> String;
    async fn query(&self, query: &str) -> Result<QueryResult> {
        self.query_limited(query, 0).await
    }
    /// Runs `query` and keeps at most `max_rows` rows (0 keeps all),
    /// setting [`QueryResult::truncated`] when there were more. A backend
    /// that streams stops reading there.
    async fn query_limited(&self, query: &str, max_rows: usize) -> Result<QueryResult>;
    async fn execute(&self, query: &str) -> Result<u64>;

    /// Runs `query` with no row limit and sends its columns, then its rows
    /// in batches of at most `fetch_size`, to `out`; returns how many rows
    /// it sent. A backend that streams (SQLite, Postgres) holds no more
    /// than a batch at a time. This default reads the whole result first,
    /// for the backends that cannot.
    async fn query_stream(&self, query: &str, fetch_size: usize, out: RowSender) -> Result<u64> {
        let result = self.query_limited(query, 0).await?;
        send_result(result, fetch_size, &out).await
    }

    /// What stops a running statement on this connection, if the backend
    /// can.
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        None
    }

    /// Runs a query with these values bound to its placeholders (`$1`,
    /// `?`, `?1` or `:1`, as the backend writes them). The catalog queries
    /// of [`schema`] go through here, so no name is ever
    /// written into SQL text. A backend that cannot bind refuses.
    async fn query_bound(&self, sql: &str, params: &[String]) -> Result<QueryResult> {
        let _ = (sql, params);
        bail!("this database cannot run a query with bound parameters")
    }

    /// The catalogs (schemas, databases, owners), the connection's own
    /// first.
    async fn catalogs(&self) -> Result<Vec<Catalog>> {
        schema::load_catalogs(self).await
    }

    /// The tables and views of `catalog`, by name.
    async fn objects(&self, catalog: &str) -> Result<Vec<DbObject>> {
        schema::load_objects(self, catalog).await
    }

    /// The columns of `catalog.object`, in table order.
    async fn columns(&self, catalog: &str, object: &str) -> Result<Vec<Column>> {
        schema::load_columns(self, catalog, object).await
    }

    /// The indexes of `catalog.object`.
    async fn indexes(&self, catalog: &str, object: &str) -> Result<Vec<Index>> {
        schema::load_indexes(self, catalog, object).await
    }

    /// What the server says it is, such as `PostgreSQL 16.4` or `3.45.1`,
    /// for the connection dialog's test message. Each backend asks in its
    /// own words; one with a cheaper way of its own can override this.
    async fn server_version(&self) -> Result<String> {
        let sql = match self.kind() {
            DatabaseType::Sqlite => "SELECT 'SQLite ' || sqlite_version()",
            DatabaseType::Postgres => "SELECT 'PostgreSQL ' || current_setting('server_version')",
            DatabaseType::Mysql => "SELECT CONCAT('MySQL ', VERSION())",
            DatabaseType::MariaDb => "SELECT CONCAT('MariaDB ', VERSION())",
            DatabaseType::Oracle => "SELECT banner FROM v$version WHERE ROWNUM = 1",
        };
        let result = self.query(sql).await?;
        result
            .rows
            .first()
            .and_then(|row| row.first().cloned().flatten())
            .ok_or_else(|| anyhow::anyhow!("the server did not say what version it is"))
    }

    /// The names of the tables and views in the connection's own catalog: a
    /// thin wrapper over [`objects`](Self::objects). Each backend overrides
    /// it with one query of its own (Oracle, with no `information_schema`,
    /// its catalog views).
    async fn tables(&self) -> Result<Vec<String>> {
        let catalogs = self.catalogs().await?;
        let Some(own) = catalogs
            .iter()
            .find(|c| c.is_default)
            .or_else(|| catalogs.first())
        else {
            return Ok(Vec::new());
        };
        Ok(self
            .objects(&own.name)
            .await?
            .into_iter()
            .map(|o| o.name)
            .collect())
    }
}

/// The first column of each row, NULLs left out: what a backend's
/// [`Database::tables`] query gives back as names.
pub(crate) fn first_column(result: QueryResult) -> Vec<String> {
    result
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next().flatten())
        .collect()
}

/// What kind of value a column holds, as far as showing it goes: numbers
/// are right-aligned and sort as numbers, dates sort as text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValueKind {
    Integer,
    Decimal,
    Float,
    Text,
    Bool,
    Date,
    Time,
    Timestamp,
    Json,
    Binary,
    #[default]
    Other,
}

impl ValueKind {
    /// The kind of a column whose database type is called `name` (`int8`,
    /// `VARCHAR(20)`, `MYSQL_TYPE_LONG`, `NUMBER(10,2)`), across the five
    /// backends. A name it does not know is `Other`.
    pub fn from_type_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        let name = lower
            .trim_start_matches("mysql_type_")
            .split(['(', ' '])
            .next()
            .unwrap_or("");
        match name {
            "int" | "integer" | "int2" | "int4" | "int8" | "bigint" | "smallint" | "tinyint"
            | "mediumint" | "serial" | "bigserial" | "smallserial" | "long" | "longlong"
            | "short" | "tiny" | "int24" | "year" | "oid" => Self::Integer,
            "numeric" | "decimal" | "newdecimal" | "number" | "money" => Self::Decimal,
            "float" | "float4" | "float8" | "double" | "real" | "binary_float"
            | "binary_double" => Self::Float,
            "bool" | "boolean" | "bit" => Self::Bool,
            "date" | "newdate" => Self::Date,
            "time" | "timetz" | "time2" => Self::Time,
            "timestamp" | "timestamptz" | "datetime" | "datetime2" | "timestamp2" => {
                Self::Timestamp
            }
            "json" | "jsonb" => Self::Json,
            "bytea" | "blob" | "tiny_blob" | "medium_blob" | "long_blob" | "binary"
            | "varbinary" | "raw" | "long raw" | "bfile" => Self::Binary,
            "text" | "varchar" | "char" | "bpchar" | "character" | "string" | "var_string"
            | "varchar2" | "nvarchar2" | "nchar" | "clob" | "nclob" | "name" | "uuid" | "enum"
            | "set" | "citext" => Self::Text,
            _ => Self::Other,
        }
    }

    /// Whether values of this kind are numbers: right-aligned, sorted as
    /// numbers, written unquoted.
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Decimal | Self::Float)
    }
}

/// A result column: its name, its database type and what kind that is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    /// The type as the database calls it; empty when it did not say.
    pub type_name: String,
    pub kind: ValueKind,
}

impl ColumnMeta {
    /// A column of this database type.
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        let type_name = type_name.into();
        Self {
            name: name.into(),
            kind: ValueKind::from_type_name(&type_name),
            type_name,
        }
    }

    /// A column whose type is unknown.
    pub fn untyped(name: impl Into<String>) -> Self {
        Self::new(name, "")
    }
}

/// An untyped column of this name.
impl From<&str> for ColumnMeta {
    fn from(name: &str) -> Self {
        Self::untyped(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: Option<u64>,
    /// How long the statement took, filled in by the worker.
    #[serde(default)]
    pub elapsed: std::time::Duration,
    /// More rows exist than were fetched (T305).
    #[serde(default)]
    pub truncated: bool,
}

impl QueryResult {
    /// A result with no rows and no columns, as a statement that changed
    /// `affected` rows gives.
    pub fn affected(affected: u64) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            affected: Some(affected),
            elapsed: std::time::Duration::ZERO,
            truncated: false,
        }
    }

    /// The columns' names.
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

/// A statement the server refused, and where in it, when the server says:
/// a byte offset into the statement's text. Backends return it inside the
/// `anyhow::Error`, so callers that only print the error see the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueryError {}

impl QueryError {
    /// The position a backend reported as a 1-based character index into
    /// `sql`, as a byte offset; `None` if it is past the end.
    pub fn char_position(sql: &str, one_based: usize) -> Option<usize> {
        let index = one_based.checked_sub(1)?;
        sql.char_indices().nth(index).map(|(byte, _)| byte)
    }
}
