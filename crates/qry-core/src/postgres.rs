
use crate::{DatabaseType, Database};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};
use async_trait::async_trait;
use crate::shared::{QueryResult, SslMode};
use serde::{Serialize, Deserialize};
use anyhow::Result;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    /// Set as the `search_path` right after connecting.
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub ssl_mode: SslMode,
}

/// Quotes an identifier for use in SQL, e.g. `my"schema` → `"my""schema"`.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub struct Postgres {
    client: Client,
    driver: JoinHandle<()>,
    config: PostgresConfig,
}

impl Postgres {
    pub async fn connect(config: PostgresConfig) -> Result<Self> {
        config.ssl_mode.ensure_supported()?;

        // Built field by field rather than from a DSN string, so values with
        // spaces or quotes (passwords especially) need no escaping.
        let (client, connection) = tokio_postgres::Config::new()
            .host(&config.host)
            .port(config.port)
            .user(&config.user)
            .password(&config.password)
            .dbname(&config.database)
            .connect(NoTls)
            .await?;

        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });

        if let Some(schema) = &config.schema {
            client
                .batch_execute(&format!("SET search_path TO {}", quote_ident(schema)))
                .await?;
        }

        Ok(Self { client, driver, config })
    }
}

#[async_trait]
impl Database for Postgres {
    fn kind(&self) -> DatabaseType {
        DatabaseType::Postgres
    }
    fn label(&self) -> String {
        format!("{}@{}:{}/{}", self.config.user, self.config.host, self.config.port, self.config.database)
    }
    async fn query(&self, sql: &str) -> Result<QueryResult> {
        let messages = self.client.simple_query(sql).await?;

        let mut columns = Vec::new();
        let mut rows = Vec::new();
        let mut affected = 0;

        for message in messages {
            match message {
                SimpleQueryMessage::Row(row) => {
                    if columns.is_empty() {
                        columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                    }
                    rows.push((0..row.len()).map(|i| row.get(i).map(str::to_string)).collect());
                },
                SimpleQueryMessage::CommandComplete(n) => affected = n,
                _ => {}
            }
        }

        Ok(QueryResult{ columns,
                        rows,
                        affected: Some(affected)
            }
        )
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let messages = self.client.simple_query(sql).await?;
        let mut affected = 0;
        for message in messages {
            if let SimpleQueryMessage::CommandComplete(n) = message {
                affected = n;
            }
        }
        Ok(affected)
    }
}

impl Drop for Postgres {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
