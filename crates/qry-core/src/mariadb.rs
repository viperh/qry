use anyhow::Result;
use async_trait::async_trait;
use mysql_async::{Conn, OptsBuilder, Row, prelude::*};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    Database, DatabaseType,
    mysql::{TABLES_SQL, cell, column_meta},
    shared::{ColumnMeta, QueryResult, SslMode, first_column},
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MariadbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
}

/// Written by hand so the password never reaches a log or panic message.
impl std::fmt::Debug for MariadbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MariadbConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"[redacted]")
            .field("database", &self.database)
            .field("ssl_mode", &self.ssl_mode)
            .finish()
    }
}

impl MariadbConfig {
    fn opts(&self) -> OptsBuilder {
        OptsBuilder::default()
            .ip_or_hostname(self.host.clone())
            .tcp_port(self.port)
            .user(Some(self.user.clone()))
            .pass(Some(self.password.clone()))
            .db_name(Some(self.database.clone()))
    }
}

pub struct MariaDb {
    conn: Mutex<Conn>,
    config: MariadbConfig,
}

/// Shows only the (redacted) config: the driver's connection handle has no
/// `Debug` of its own.
impl std::fmt::Debug for MariaDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MariaDb")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MariaDb {
    pub async fn connect(config: MariadbConfig) -> Result<Self> {
        config.ssl_mode.ensure_supported()?;
        let conn = Conn::new(config.opts()).await?;
        Ok(Self {
            conn: Mutex::new(conn),
            config,
        })
    }

    pub async fn disconnect(self) -> Result<()> {
        self.conn.into_inner().disconnect().await?;
        Ok(())
    }
}

#[async_trait]
impl Database for MariaDb {
    fn kind(&self) -> DatabaseType {
        DatabaseType::MariaDb
    }

    fn label(&self) -> String {
        format!(
            "{}@{}:{}/{}",
            self.config.user, self.config.host, self.config.port, self.config.database
        )
    }

    async fn query(&self, sql: &str) -> Result<QueryResult> {
        let mut conn = self.conn.lock().await;
        let mut result = conn.query_iter(sql).await?;

        // `MYSQL_TYPE_LONG` and the like, which `ValueKind` reads.
        let (columns, shown): (Vec<ColumnMeta>, Vec<_>) =
            result.columns_ref().iter().map(column_meta).unzip();

        let rows = result
            .collect::<Row>()
            .await?
            .into_iter()
            .map(|row| {
                row.unwrap()
                    .into_iter()
                    .zip(&shown)
                    .map(|(value, &shown)| cell(value, shown))
                    .collect()
            })
            .collect();

        let affected = result.affected_rows();
        result.drop_result().await?;

        Ok(QueryResult {
            columns,
            rows,
            affected: Some(affected),
            elapsed: std::time::Duration::ZERO,
            truncated: false,
        })
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let mut conn = self.conn.lock().await;
        conn.query_drop(sql).await?;
        Ok(conn.affected_rows())
    }

    async fn tables(&self) -> Result<Vec<String>> {
        Ok(first_column(self.query(TABLES_SQL).await?))
    }
}
