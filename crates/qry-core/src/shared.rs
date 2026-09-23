use std::fmt::{Debug};
use async_trait::async_trait;
use crate::DatabaseType;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

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

#[async_trait]
pub trait Database: Send + Sync{

    fn kind(&self) -> DatabaseType;
    fn label(&self) -> String;
    async fn query(&self, query: &str) -> Result<QueryResult>;
    async fn execute(&self, query: &str) -> Result<u64>;

    /// The tables and views this connection can see, for the tree. Each
    /// backend keeps its list somewhere else, so the query differs; a
    /// backend with a cheaper way of its own can override this.
    async fn tables(&self) -> Result<Vec<String>> {
        let sql = match self.kind() {
            DatabaseType::Sqlite => {
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name"
            }
            DatabaseType::Postgres => {
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = current_schema() ORDER BY table_name"
            }
            DatabaseType::Mysql | DatabaseType::MariaDb => {
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = DATABASE() ORDER BY table_name"
            }
            DatabaseType::Oracle => bail!("listing tables is not supported for this database"),
        };
        let result = self.query(sql).await?;
        Ok(result.rows.iter().filter_map(|row| row.first().cloned().flatten()).collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: Option<u64>,
}