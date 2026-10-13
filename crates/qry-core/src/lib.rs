use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub mod exporter;
pub mod mariadb;
pub mod mysql;
pub mod oracle;
pub mod postgres;
pub mod shared;
pub mod sql;
pub mod sqlite;
/// Fakes and fixtures for tests; never in a release build.
#[cfg(any(test, feature = "test-util"))]
pub mod testing;

pub use exporter::Exporter;
use mariadb::{MariaDb, MariadbConfig};
use mysql::{MySql, MySqlConfig};
use oracle::{Oracle, OracleConfig};
use postgres::{Postgres, PostgresConfig};
pub use shared::{Database, QueryResult, SslMode};
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
    Excel,
    Text,
    Json,
}

impl ExportType {
    /// In the order the export form offers them.
    pub const ALL: [ExportType; 4] = [
        ExportType::Csv,
        ExportType::Excel,
        ExportType::Text,
        ExportType::Json,
    ];

    /// Whether the rows are separated by a character the user chooses.
    /// Excel writes cells and JSON writes objects, so neither uses one.
    pub fn uses_separator(self) -> bool {
        matches!(self, ExportType::Csv | ExportType::Text)
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportType::Csv => "csv",
            ExportType::Excel => "xlsx",
            ExportType::Text => "txt",
            ExportType::Json => "json",
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
            ExportType::Excel => "Excel",
            ExportType::Text => "Text",
            ExportType::Json => "JSON",
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

    /// The tables of the open connection, for the connection tree.
    pub async fn tables(&self) -> Result<Vec<String>> {
        let Some(db) = &self.db else {
            bail!("not connected to a database");
        };
        db.tables().await
    }
}

#[cfg(test)]
mod lib_tests;
