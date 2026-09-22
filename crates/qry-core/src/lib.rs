use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub mod shared;
pub mod postgres;
pub mod sqlite;
pub mod mysql;
pub mod mariadb;

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

/// Everything needed to open a connection, one variant per backend.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionConfig {
    Sqlite(SqliteConfig),
    Postgres(PostgresConfig),
    Mysql(MySqlConfig),
    MariaDb(MariadbConfig),
}

/// Written by hand so a password can never end up in a log.
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

/// Holds the active connection. It is meant to be owned by a single task,
/// so it needs no locking of its own.
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


    pub async fn export(&self, sql: &str) -> Result<()> {
        if let Some(db) = &self.db {
            
        }
        Ok(())
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
    fn debug_output_never_contains_the_password() {
        let shown = format!("{:?}", postgres(SslMode::Prefer));
        assert_eq!(shown, "Postgres(alice@db.invalid:5432/app)");
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
