use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub mod shared;
pub mod postgres;
pub mod sqlite;
pub mod mysql;
pub mod mariadb;
pub mod exporter;

pub use exporter::Exporter;
pub use shared::{Database, QueryResult, SslMode};

use mariadb::{MariaDb, MariadbConfig};
use mysql::{MySql, MySqlConfig};
use postgres::{Postgres, PostgresConfig};
use sqlite::{Sqlite, SqliteConfig};

#[derive(Debug, Clone)]
pub enum DatabaseType {
    Sqlite,
    Postgres,
    Mysql,
    Oracle,
    MariaDb
}
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExportType {
    #[default]
    Csv,
    Excel,
    Text,
    Json
}

impl ExportType {
    /// In the order the export form offers them.
    pub const ALL: [ExportType; 4] =
        [ExportType::Csv, ExportType::Excel, ExportType::Text, ExportType::Json];

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
    pub etype: ExportType
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
}


impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(c) => write!(f, "Sqlite({})", c.path.display()),
            Self::Postgres(c) => write!(f, "Postgres({}@{}:{}/{})", c.user, c.host, c.port, c.database),
            Self::Mysql(c) => write!(f, "Mysql({}@{}:{}/{})", c.user, c.host, c.port, c.database),
            Self::MariaDb(c) => write!(f, "MariaDb({}@{}:{}/{})", c.user, c.host, c.port, c.database),
        }
    }
}


#[derive(Default)]
pub struct Driver {
    db: Option<Box<dyn Database>>,

}

impl Driver {

    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a connection, replacing any previous one, and returns its label.
    pub async fn connect(&mut self, config: ConnectionConfig) -> Result<String> {
        let db: Box<dyn Database> = match config {
            ConnectionConfig::Sqlite(config) => Box::new(Sqlite::connect(config).await?),
            ConnectionConfig::Postgres(config) => Box::new(Postgres::connect(config).await?),
            ConnectionConfig::Mysql(config) => Box::new(MySql::connect(config).await?),
            ConnectionConfig::MariaDb(config) => Box::new(MariaDb::connect(config).await?),
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
mod tests {
    use super::*;

    fn in_memory() -> ConnectionConfig {
        ConnectionConfig::Sqlite(SqliteConfig::new(":memory:", false))
    }

    #[tokio::test]
    async fn query_without_connection_is_err() {
        assert!(Driver::new().query("SELECT 1").await.is_err());
    }

    #[tokio::test]
    async fn sqlite_round_trip() {
        let mut driver = Driver::new();
        assert_eq!(driver.connect(in_memory()).await.unwrap(), ":memory:");

        let result = driver
            .query("WITH t(id, name) AS (VALUES (1, 'alice'), (2, NULL)) SELECT * FROM t")
            .await
            .unwrap();
        assert_eq!(result.columns, ["id", "name"]);
        assert_eq!(
            result.rows,
            [
                vec![Some("1".to_string()), Some("alice".to_string())],
                vec![Some("2".to_string()), None],
            ]
        );
    }

    fn postgres(ssl_mode: SslMode) -> ConnectionConfig {
        ConnectionConfig::Postgres(PostgresConfig {
            host: "db.invalid".into(),
            port: 5432,
            user: "alice".into(),
            password: "hunter2".into(),
            database: "app".into(),
            schema: None,
            ssl_mode,
        })
    }

    #[tokio::test]
    async fn tls_only_ssl_modes_are_refused_before_connecting() {
        for mode in [SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull] {
            let err = Driver::new().connect(postgres(mode)).await.unwrap_err();
            assert!(err.to_string().contains("does not support"), "{mode}: {err}");
        }
    }

    #[test]
    fn each_type_has_an_extension_and_adds_it_once() {
        let extensions: Vec<&str> = ExportType::ALL.iter().map(|t| t.extension()).collect();
        assert_eq!(extensions, ["csv", "xlsx", "txt", "json"]);

        assert_eq!(ExportType::Csv.with_extension("rows"), "rows.csv");
        assert_eq!(ExportType::Csv.with_extension("rows.csv"), "rows.csv");
        // Already there, whatever the case.
        assert_eq!(ExportType::Csv.with_extension("ROWS.CSV"), "ROWS.CSV");
        assert_eq!(ExportType::Csv.with_extension("rows."), "rows.csv");
        // A different extension is kept, and the right one added after it.
        assert_eq!(ExportType::Csv.with_extension("rows.txt"), "rows.txt.csv");
        assert_eq!(ExportType::Excel.with_extension(r"C:\data\rows"), r"C:\data\rows.xlsx");
        assert_eq!(ExportType::Json.with_extension("rows.json"), "rows.json");
    }

    #[test]
    fn a_config_gets_the_extension_of_its_type() {
        let config = ExportConfig::new("rows", ",", ExportType::Json);
        assert_eq!(config.path, "rows.json");
        assert_eq!(config.separator, ",");
    }

    #[test]
    fn debug_output_never_contains_the_password() {
        let shown = format!("{:?}", postgres(SslMode::Prefer));
        assert_eq!(shown, "Postgres(alice@db.invalid:5432/app)");
    }

    #[tokio::test]
    async fn tables_lists_what_the_connection_can_see() {
        let mut driver = Driver::new();
        assert!(driver.tables().await.is_err(), "not connected");

        driver.connect(in_memory()).await.unwrap();
        assert!(driver.tables().await.unwrap().is_empty());

        driver.query("CREATE TABLE people (name TEXT)").await.unwrap();
        driver.query("CREATE TABLE addresses (line TEXT)").await.unwrap();
        driver.query("CREATE VIEW recent AS SELECT * FROM people").await.unwrap();
        assert_eq!(driver.tables().await.unwrap(), ["addresses", "people", "recent"]);
    }

    #[tokio::test]
    async fn state_persists_between_queries() {
        let mut driver = Driver::new();
        driver.connect(in_memory()).await.unwrap();
        driver.query("CREATE TABLE t (x INTEGER)").await.unwrap();
        driver.query("INSERT INTO t VALUES (7)").await.unwrap();

        let result = driver.query("SELECT x FROM t").await.unwrap();
        assert_eq!(result.rows, [vec![Some("7".to_string())]]);
    }
}
