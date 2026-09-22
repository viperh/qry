use crate::shared::{QueryResult, SslMode};
use crate::{Database, DatabaseType};
use anyhow::Result;
use async_trait::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Conn, OptsBuilder, Row, Value};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

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

        let columns = result
            .columns_ref()
            .iter()
            .map(|c| c.name_str().into_owned())
            .collect();

        let rows = result
            .collect::<Row>()
            .await?
            .into_iter()
            .map(|row| row.unwrap().into_iter().map(value_to_string).collect())
            .collect();

        let affected = result.affected_rows();
        result.drop_result().await?;

        Ok(QueryResult {
            columns,
            rows,
            affected: Some(affected),
        })
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let mut conn = self.conn.lock().await;
        conn.query_drop(sql).await?;
        Ok(conn.affected_rows())
    }
}