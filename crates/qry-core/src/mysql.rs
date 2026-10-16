use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mysql_async::{Conn, OptsBuilder, Value, prelude::*};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    Database, DatabaseType,
    shared::{CancelHandle, ColumnMeta, QueryResult, SslMode},
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MySqlConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
}

/// Written by hand so the password never reaches a log or panic message.
impl std::fmt::Debug for MySqlConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MySqlConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"[redacted]")
            .field("database", &self.database)
            .field("ssl_mode", &self.ssl_mode)
            .finish()
    }
}

impl MySqlConfig {
    fn opts(&self) -> OptsBuilder {
        OptsBuilder::default()
            .ip_or_hostname(self.host.clone())
            .tcp_port(self.port)
            .user(Some(self.user.clone()))
            .pass(Some(self.password.clone()))
            .db_name(Some(self.database.clone()))
    }
}

pub struct MySql {
    conn: Mutex<Conn>,
    /// The server's id for the connection, which `KILL QUERY` names.
    id: u32,
    config: MySqlConfig,
}

/// Shows only the (redacted) config: the driver's connection handle has no
/// `Debug` of its own.
impl std::fmt::Debug for MySql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MySql")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MySql {
    pub async fn connect(config: MySqlConfig) -> Result<Self> {
        config.ssl_mode.ensure_supported()?;
        let conn = Conn::new(config.opts()).await?;
        Ok(Self {
            id: conn.id(),
            conn: Mutex::new(conn),
            config,
        })
    }

    pub async fn disconnect(self) -> Result<()> {
        self.conn.into_inner().disconnect().await?;
        Ok(())
    }
}

fn value_to_string(value: Value) -> Option<String> {
    match value {
        Value::NULL => None,
        Value::Bytes(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Value::Int(n) => Some(n.to_string()),
        Value::UInt(n) => Some(n.to_string()),
        Value::Float(n) => Some(n.to_string()),
        Value::Double(n) => Some(n.to_string()),
        other => Some(other.as_sql(true).trim_matches('\'').to_string()),
    }
}

/// Stops a statement from a short-lived second connection, as the busy
/// one cannot be asked.
struct KillQuery {
    opts: OptsBuilder,
    id: u32,
}

#[async_trait]
impl CancelHandle for KillQuery {
    async fn cancel(&self) -> Result<()> {
        let mut conn = Conn::new(self.opts.clone()).await?;
        conn.query_drop(format!("KILL QUERY {}", self.id)).await?;
        conn.disconnect().await?;
        Ok(())
    }
}

#[async_trait]
impl Database for MySql {
    fn kind(&self) -> DatabaseType {
        DatabaseType::Mysql
    }

    fn label(&self) -> String {
        format!(
            "{}@{}:{}/{}",
            self.config.user, self.config.host, self.config.port, self.config.database
        )
    }

    async fn query_limited(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        let mut conn = self.conn.lock().await;
        let mut result = conn.query_iter(sql).await?;

        // `MYSQL_TYPE_LONG` and the like, which `ValueKind` reads.
        let columns = result
            .columns_ref()
            .iter()
            .map(|c| ColumnMeta::new(c.name_str().into_owned(), format!("{:?}", c.column_type())))
            .collect();

        let mut rows = Vec::new();
        let mut truncated = false;
        while let Some(row) = result.next().await? {
            // The rest is read off the wire and dropped, never kept.
            if max_rows > 0 && rows.len() == max_rows {
                truncated = true;
                break;
            }
            rows.push(row.unwrap().into_iter().map(value_to_string).collect());
        }

        let affected = result.affected_rows();
        result.drop_result().await?;

        Ok(QueryResult {
            columns,
            rows,
            affected: Some(affected),
            elapsed: std::time::Duration::ZERO,
            truncated,
        })
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(KillQuery {
            opts: self.config.opts(),
            id: self.id,
        }))
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let mut conn = self.conn.lock().await;
        conn.query_drop(sql).await?;
        Ok(conn.affected_rows())
    }
}
