use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};

use crate::{
    Database, DatabaseType,
    shared::{ColumnMeta, QueryError, QueryResult, SslMode, ValueKind, first_column},
};

/// A boolean as the text protocol writes it (`t`, `f`), as `true` or
/// `false`. Anything else is left alone.
fn bool_text(value: String) -> String {
    match value.as_str() {
        "t" => "true".into(),
        "f" => "false".into(),
        _ => value,
    }
}

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

/// Written by hand so the password never reaches a log or panic message.
impl std::fmt::Debug for PostgresConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"[redacted]")
            .field("database", &self.database)
            .field("schema", &self.schema)
            .field("ssl_mode", &self.ssl_mode)
            .finish()
    }
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

/// Shows only the (redacted) config: the driver's connection handle has no
/// `Debug` of its own.
impl std::fmt::Debug for Postgres {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Postgres")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
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

        Ok(Self {
            client,
            driver,
            config,
        })
    }
}

#[async_trait]
impl Database for Postgres {
    fn kind(&self) -> DatabaseType {
        DatabaseType::Postgres
    }
    fn label(&self) -> String {
        format!(
            "{}@{}:{}/{}",
            self.config.user, self.config.host, self.config.port, self.config.database
        )
    }
    async fn query(&self, sql: &str) -> Result<QueryResult> {
        // Preparing first gives the columns' types, and their names even
        // when no row comes back. The simple protocol still runs it, as it
        // takes what preparing refuses (COPY, several commands).
        let described: Option<Vec<ColumnMeta>> = self.client.prepare(sql).await.ok().map(|st| {
            st.columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), c.type_().name()))
                .collect()
        });
        let messages = self.client.simple_query(sql).await.map_err(|e| {
            let Some(db) = e.as_db_error() else {
                return anyhow::Error::new(e);
            };
            let position = match db.position() {
                Some(tokio_postgres::error::ErrorPosition::Original(p)) => {
                    QueryError::char_position(sql, *p as usize)
                }
                _ => None,
            };
            anyhow::Error::new(QueryError {
                message: db.message().to_string(),
                position,
            })
        })?;

        let mut columns = Vec::new();
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();
        let mut affected = 0;

        for message in messages {
            match message {
                SimpleQueryMessage::Row(row) => {
                    if columns.is_empty() {
                        columns = row
                            .columns()
                            .iter()
                            .map(|c| ColumnMeta::untyped(c.name()))
                            .collect();
                    }
                    rows.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_string))
                            .collect(),
                    );
                }
                SimpleQueryMessage::CommandComplete(n) => affected = n,
                _ => {}
            }
        }

        if let Some(described) = described
            && (columns.is_empty() || described.len() == columns.len())
        {
            columns = described;
        }
        // The text protocol writes booleans as `t` and `f`; every backend
        // shows them as `true` and `false`.
        for (i, column) in columns.iter().enumerate() {
            if column.kind == ValueKind::Bool {
                for row in &mut rows {
                    if let Some(cell) = row.get_mut(i) {
                        *cell = cell.take().map(bool_text);
                    }
                }
            }
        }
        Ok(QueryResult {
            columns,
            rows,
            affected: Some(affected),
            elapsed: std::time::Duration::ZERO,
            truncated: false,
        })
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let messages = self.client.simple_query(sql).await.map_err(|e| {
            let Some(db) = e.as_db_error() else {
                return anyhow::Error::new(e);
            };
            let position = match db.position() {
                Some(tokio_postgres::error::ErrorPosition::Original(p)) => {
                    QueryError::char_position(sql, *p as usize)
                }
                _ => None,
            };
            anyhow::Error::new(QueryError {
                message: db.message().to_string(),
                position,
            })
        })?;
        let mut affected = 0;
        for message in messages {
            if let SimpleQueryMessage::CommandComplete(n) = message {
                affected = n;
            }
        }
        Ok(affected)
    }

    async fn tables(&self) -> Result<Vec<String>> {
        let result = self
            .query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = current_schema() ORDER BY table_name",
            )
            .await?;
        Ok(first_column(result))
    }
}

impl Drop for Postgres {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
