use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures::{StreamExt, pin_mut};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};

use crate::{
    Database, DatabaseType,
    shared::{
        self, CancelHandle, ColumnMeta, QueryError, QueryResult, RowSender, SslMode, StreamItem,
        ValueKind, first_column,
    },
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

/// Sends a batch of rows, booleans written as every backend shows them.
async fn send_rows(
    out: &RowSender,
    columns: &[ColumnMeta],
    mut rows: Vec<Vec<Option<String>>>,
) -> Result<()> {
    for (i, column) in columns.iter().enumerate() {
        if column.kind == ValueKind::Bool {
            for row in &mut rows {
                if let Some(cell) = row.get_mut(i) {
                    *cell = cell.take().map(bool_text);
                }
            }
        }
    }
    shared::send(out, StreamItem::Rows(rows)).await
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
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(Cancel(self.client.cancel_token())))
    }

    fn streams_rows(&self) -> bool {
        true
    }

    async fn query_limited(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        // Preparing first gives the columns' types, and their names even
        // when no row comes back. The simple protocol still runs it, as it
        // takes what preparing refuses (COPY, several commands).
        let described: Option<Vec<ColumnMeta>> = self.client.prepare(sql).await.ok().map(|st| {
            st.columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), c.type_().name()))
                .collect()
        });
        let stream = self
            .client
            .simple_query_raw(sql)
            .await
            .map_err(|e| positioned(sql, e))?;
        pin_mut!(stream);

        let mut columns = Vec::new();
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();
        let mut affected = 0;
        let mut truncated = false;

        while let Some(message) = stream.next().await {
            match message.map_err(|e| positioned(sql, e))? {
                SimpleQueryMessage::Row(row) => {
                    // The rest stays unread; the connection drains it.
                    if max_rows > 0 && rows.len() == max_rows {
                        truncated = true;
                        break;
                    }
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
            truncated,
        })
    }

    /// Reads the simple-protocol stream as it arrives, handing on
    /// `fetch_size` rows at a time, so no more than a batch is held.
    async fn query_stream(&self, sql: &str, fetch_size: usize, out: RowSender) -> Result<u64> {
        let fetch_size = fetch_size.max(1);
        let described: Option<Vec<ColumnMeta>> = self.client.prepare(sql).await.ok().map(|st| {
            st.columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), c.type_().name()))
                .collect()
        });
        let stream = self
            .client
            .simple_query_raw(sql)
            .await
            .map_err(|e| positioned(sql, e))?;
        pin_mut!(stream);

        let mut columns: Option<Vec<ColumnMeta>> = None;
        let mut batch: Vec<Vec<Option<String>>> = Vec::with_capacity(fetch_size);
        let mut sent = 0u64;
        while let Some(message) = stream.next().await {
            let SimpleQueryMessage::Row(row) = message.map_err(|e| positioned(sql, e))? else {
                continue;
            };
            if columns.is_none() {
                let named: Vec<ColumnMeta> = row
                    .columns()
                    .iter()
                    .map(|c| ColumnMeta::untyped(c.name()))
                    .collect();
                let chosen = match &described {
                    Some(d) if d.len() == named.len() => d.clone(),
                    _ => named,
                };
                shared::send(&out, StreamItem::Columns(chosen.clone())).await?;
                columns = Some(chosen);
            }
            batch.push(
                (0..row.len())
                    .map(|i| row.get(i).map(str::to_string))
                    .collect(),
            );
            if batch.len() == fetch_size {
                sent += batch.len() as u64;
                let full = std::mem::replace(&mut batch, Vec::with_capacity(fetch_size));
                send_rows(&out, columns.as_deref().unwrap_or_default(), full).await?;
            }
        }
        match &columns {
            None => {
                let columns = described.unwrap_or_default();
                shared::send(&out, StreamItem::Columns(columns)).await?;
            }
            Some(columns) if !batch.is_empty() => {
                sent += batch.len() as u64;
                send_rows(&out, columns, batch).await?;
            }
            Some(_) => {}
        }
        Ok(sent)
    }

    /// The extended protocol, which `simple_query` cannot do: the catalog
    /// queries cast every column to `text`, and their placeholders to
    /// `$n::text`, so every value crosses as a string.
    async fn query_bound(&self, sql: &str, params: &[String]) -> Result<QueryResult> {
        let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let rows = self.client.query(sql, &refs).await?;
        let columns = rows
            .first()
            .map(|row| {
                row.columns()
                    .iter()
                    .map(|c| ColumnMeta::new(c.name(), c.type_().name()))
                    .collect()
            })
            .unwrap_or_default();
        let rows = rows
            .iter()
            .map(|row| {
                (0..row.len())
                    .map(|i| row.try_get::<_, Option<String>>(i).ok().flatten())
                    .collect()
            })
            .collect();
        Ok(QueryResult {
            columns,
            rows,
            affected: None,
            elapsed: std::time::Duration::ZERO,
            truncated: false,
        })
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let messages = self
            .client
            .simple_query(sql)
            .await
            .map_err(|e| positioned(sql, e))?;
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

/// A server error, with where in `sql` it stopped when the server says.
fn positioned(sql: &str, e: tokio_postgres::Error) -> anyhow::Error {
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
}

/// Asks the server, on a connection of its own, to stop the statement.
struct Cancel(tokio_postgres::CancelToken);

#[async_trait]
impl CancelHandle for Cancel {
    async fn cancel(&self) -> Result<()> {
        self.0.cancel_query(NoTls).await?;
        Ok(())
    }
}

impl Drop for Postgres {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
