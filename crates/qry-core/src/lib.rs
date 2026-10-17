use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub mod connspec;
pub mod copy;
pub mod exporter;
#[doc(hidden)]
pub mod fuzz;
pub mod fuzzy;
pub mod grid;
pub mod history;
pub mod inspect;
pub mod jobs;
pub mod mariadb;
pub mod mysql;
pub mod oracle;
pub mod postgres;
pub mod schema;
pub mod shared;
pub mod snippets;
pub mod sql;
pub mod sqlite;
/// Fakes and fixtures for tests; never in a release build.
#[cfg(any(test, feature = "test-util"))]
pub mod testing;

pub use exporter::{ExportOptions, RowSink};
use mariadb::{MariaDb, MariadbConfig};
use mysql::{MySql, MySqlConfig};
use oracle::{Oracle, OracleConfig};
use postgres::{Postgres, PostgresConfig};
pub use shared::{ColumnMeta, Database, QueryResult, SslMode, ValueKind};
use sqlite::{Sqlite, SqliteConfig};

#[derive(Debug, Clone)]
pub enum DatabaseType {
    Sqlite,
    Postgres,
    Mysql,
    Oracle,
    MariaDb,
}
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExportType {
    #[default]
    Csv,
    Tsv,
    Json,
    JsonLines,
    Excel,
    Markdown,
    SqlInsert,
    Text,
}

impl ExportType {
    /// In the order the export dialog offers them.
    pub const ALL: [ExportType; 8] = [
        ExportType::Csv,
        ExportType::Tsv,
        ExportType::Json,
        ExportType::JsonLines,
        ExportType::Excel,
        ExportType::Markdown,
        ExportType::SqlInsert,
        ExportType::Text,
    ];

    /// Whether the rows are separated by a character the user chooses.
    /// TSV is always tabs, and the others write cells, objects or
    /// statements.
    pub fn uses_separator(self) -> bool {
        matches!(self, ExportType::Csv | ExportType::Text)
    }

    /// Whether a header line can be left out: JSON names every value
    /// anyway.
    pub fn has_header(self) -> bool {
        !matches!(self, ExportType::Json | ExportType::JsonLines)
    }

    /// Whether a NULL is written as some text the user chooses. JSON has
    /// `null` and SQL `NULL`.
    pub fn uses_null_as(self) -> bool {
        !matches!(
            self,
            ExportType::Json | ExportType::JsonLines | ExportType::SqlInsert
        )
    }

    /// Whether the file is text, with an encoding to choose.
    pub fn is_text(self) -> bool {
        self != ExportType::Excel
    }

    /// The most rows the format holds, if it has a limit: a worksheet has
    /// 1 048 576 rows, one of them the header.
    pub fn row_limit(self) -> Option<usize> {
        (self == ExportType::Excel).then_some(exporter::EXCEL_MAX_ROWS)
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportType::Csv => "csv",
            ExportType::Tsv => "tsv",
            ExportType::Json => "json",
            ExportType::JsonLines => "jsonl",
            ExportType::Excel => "xlsx",
            ExportType::Markdown => "md",
            ExportType::SqlInsert => "sql",
            ExportType::Text => "txt",
        }
    }

    /// The path with this type's extension, added unless it is already there.
    /// The check ignores case, so `ROWS.CSV` is left alone.
    pub fn with_extension(self, path: &str) -> String {
        let extension = self.extension();
        let present = path
            .rsplit_once('.')
            .is_some_and(|(_, last)| last.eq_ignore_ascii_case(extension));
        match path {
            _ if present => path.to_string(),
            // A trailing dot already separates the extension.
            _ if path.ends_with('.') => format!("{path}{extension}"),
            _ => format!("{path}.{extension}"),
        }
    }
}

impl std::fmt::Display for ExportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ExportType::Csv => "CSV",
            ExportType::Tsv => "TSV",
            ExportType::Json => "JSON",
            ExportType::JsonLines => "JSON lines",
            ExportType::Excel => "Excel",
            ExportType::Markdown => "Markdown",
            ExportType::SqlInsert => "SQL INSERT",
            ExportType::Text => "Text",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExportConfig {
    pub path: String,
    pub separator: String,
    pub etype: ExportType,
}

impl ExportConfig {
    /// Builds a config whose path ends with the type's extension.
    pub fn new(path: impl Into<String>, separator: impl Into<String>, etype: ExportType) -> Self {
        Self {
            path: etype.with_extension(&path.into()),
            separator: separator.into(),
            etype,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionConfig {
    Sqlite(SqliteConfig),
    Postgres(PostgresConfig),
    Mysql(MySqlConfig),
    MariaDb(MariadbConfig),
    Oracle(OracleConfig),
}

impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(c) => write!(f, "Sqlite({})", c.path.display()),
            Self::Postgres(c) => write!(
                f,
                "Postgres({}@{}:{}/{})",
                c.user, c.host, c.port, c.database
            ),
            Self::Mysql(c) => write!(f, "Mysql({}@{}:{}/{})", c.user, c.host, c.port, c.database),
            Self::MariaDb(c) => write!(
                f,
                "MariaDb({}@{}:{}/{})",
                c.user, c.host, c.port, c.database
            ),
            Self::Oracle(c) => write!(f, "Oracle({}@{}:{}/{})", c.user, c.host, c.port, c.service),
        }
    }
}

#[derive(Default)]
pub struct Driver {
    db: Option<Box<dyn Database>>,
}

/// Shows the label of the open connection, if any; the connection itself has
/// no `Debug`.
impl std::fmt::Debug for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver")
            .field("connected_to", &self.db.as_ref().map(|db| db.label()))
            .finish()
    }
}

impl Driver {
    pub fn new() -> Self {
        Self::default()
    }

    /// A driver already connected to `db`, so the worker can be tested with a
    /// [`FakeDatabase`](crate::testing::FakeDatabase) instead of a server.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_database(db: Box<dyn Database>) -> Self {
        Self { db: Some(db) }
    }

    /// Opens a connection, replacing any previous one, and returns its label.
    pub async fn connect(&mut self, config: ConnectionConfig) -> Result<String> {
        let db: Box<dyn Database> = match config {
            ConnectionConfig::Sqlite(config) => Box::new(Sqlite::connect(config).await?),
            ConnectionConfig::Postgres(config) => Box::new(Postgres::connect(config).await?),
            ConnectionConfig::Mysql(config) => Box::new(MySql::connect(config).await?),
            ConnectionConfig::MariaDb(config) => Box::new(MariaDb::connect(config).await?),
            ConnectionConfig::Oracle(config) => Box::new(Oracle::connect(config).await?),
        };
        let label = db.label();
        self.db = Some(db);
        Ok(label)
    }

    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        db.query(sql).await
    }

    /// Runs `sql` and streams every row to `out`, `fetch_size` at a time
    /// (see [`Database::query_stream`]).
    pub async fn query_stream(
        &self,
        sql: &str,
        fetch_size: usize,
        out: shared::RowSender,
    ) -> Result<u64> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        db.query_stream(sql, fetch_size, out).await
    }

    /// Runs `sql`, keeping at most `max_rows` rows (0 keeps all).
    pub async fn query_limited(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        db.query_limited(sql, max_rows).await
    }

    /// What stops the open connection's running statement, if it can be.
    pub fn cancel_handle(&self) -> Option<std::sync::Arc<dyn shared::CancelHandle>> {
        self.db.as_ref()?.cancel_handle()
    }

    /// What the open connection's server says it is; see
    /// [`Database::server_version`].
    pub async fn server_version(&self) -> Result<String> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        db.server_version().await
    }

    /// The tables of the open connection's own catalog.
    pub async fn tables(&self) -> Result<Vec<String>> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        db.tables().await
    }

    /// Loads one node of the schema browser: the catalogs, a catalog's
    /// objects, or an object's columns and indexes.
    pub async fn load_schema(&self, path: &schema::SchemaPath) -> Result<schema::Children> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        schema::load(db.as_ref(), path).await
    }
}

#[cfg(test)]
mod lib_tests;
