use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mysql_async::{Column, Conn, OptsBuilder, Value, consts::ColumnType, prelude::*};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    Database, DatabaseType,
    shared::{CancelHandle, ColumnMeta, QueryResult, SslMode, ValueKind, first_column},
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

pub(crate) fn value_to_string(value: Value) -> Option<String> {
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
/// one cannot be asked. MariaDB uses it too.
pub(crate) struct KillQuery {
    pub(crate) opts: OptsBuilder,
    pub(crate) id: u32,
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

/// How a column's values are written out. The text protocol sends every
/// value as bytes, so only the column says what they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    Text,
    /// `\x` and lowercase hex, as Postgres writes a `bytea`.
    Binary,
    /// `true` or `false`: a `tinyint(1)`, which is what `BOOL` declares.
    Bool,
}

/// The collation id MySQL and MariaDB give binary strings (and numbers).
const BINARY_CHARSET: u16 = 63;

/// A result column's name, type and kind, and how to write its values.
/// `TEXT` and `BLOB` share a type on the wire (`MYSQL_TYPE_BLOB`), as do
/// `VARCHAR` and `VARBINARY`; the binary collation tells them apart.
pub(crate) fn column_meta(column: &Column) -> (ColumnMeta, Shown) {
    let column_type = column.column_type();
    let mut meta = ColumnMeta::new(column.name_str().into_owned(), format!("{column_type:?}"));
    let stringish = matches!(
        column_type,
        ColumnType::MYSQL_TYPE_TINY_BLOB
            | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
            | ColumnType::MYSQL_TYPE_LONG_BLOB
            | ColumnType::MYSQL_TYPE_BLOB
            | ColumnType::MYSQL_TYPE_VAR_STRING
            | ColumnType::MYSQL_TYPE_STRING
            | ColumnType::MYSQL_TYPE_VARCHAR
    );
    let shown = if stringish && column.character_set() == BINARY_CHARSET {
        meta.kind = ValueKind::Binary;
        Shown::Binary
    } else if stringish {
        meta.kind = ValueKind::Text;
        Shown::Text
    } else if column_type == ColumnType::MYSQL_TYPE_TINY && column.column_length() == 1 {
        meta.kind = ValueKind::Bool;
        Shown::Bool
    } else {
        Shown::Text
    };
    (meta, shown)
}

/// One cell as text, or `None` for NULL.
pub(crate) fn cell(value: Value, shown: Shown) -> Option<String> {
    match (shown, value) {
        (_, Value::NULL) => None,
        (Shown::Binary, Value::Bytes(bytes)) => Some(format!("\\x{}", hex::encode(bytes))),
        (Shown::Bool, value) => {
            let text = value_to_string(value)?;
            Some(match text.as_str() {
                "0" => "false".into(),
                "1" => "true".into(),
                _ => text,
            })
        }
        (_, value) => value_to_string(value),
    }
}

/// The tables and views of the current database, for both backends.
pub(crate) const TABLES_SQL: &str = "SELECT table_name FROM information_schema.tables \
     WHERE table_schema = DATABASE() ORDER BY table_name";

/// Runs `sql` as a prepared statement with `params` bound to its `?`s, in
/// order. MariaDB speaks the same protocol, so it shares this.
pub(crate) async fn exec_bound(
    conn: &mut Conn,
    sql: &str,
    params: &[String],
) -> Result<QueryResult> {
    let params = if params.is_empty() {
        mysql_async::Params::Empty
    } else {
        mysql_async::Params::Positional(params.iter().map(|p| Value::from(p.as_str())).collect())
    };
    let mut result = conn.exec_iter(sql, params).await?;
    let (columns, shown): (Vec<ColumnMeta>, Vec<_>) =
        result.columns_ref().iter().map(column_meta).unzip();
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        rows.push(
            row.unwrap()
                .into_iter()
                .zip(&shown)
                .map(|(value, &shown)| cell(value, shown))
                .collect(),
        );
    }
    result.drop_result().await?;
    Ok(QueryResult {
        columns,
        rows,
        affected: None,
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    })
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
        let (columns, shown): (Vec<ColumnMeta>, Vec<_>) =
            result.columns_ref().iter().map(column_meta).unzip();

        let mut rows = Vec::new();
        let mut truncated = false;
        while let Some(row) = result.next().await? {
            // The rest is read off the wire and dropped, never kept.
            if max_rows > 0 && rows.len() == max_rows {
                truncated = true;
                break;
            }
            rows.push(
                row.unwrap()
                    .into_iter()
                    .zip(&shown)
                    .map(|(value, &shown)| cell(value, shown))
                    .collect(),
            );
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

    async fn tables(&self) -> Result<Vec<String>> {
        Ok(first_column(self.query(TABLES_SQL).await?))
    }

    async fn query_bound(&self, sql: &str, params: &[String]) -> Result<QueryResult> {
        let mut conn = self.conn.lock().await;
        exec_bound(&mut conn, sql, params).await
    }
}

#[cfg(test)]
#[path = "mysql_tests.rs"]
mod mysql_tests;
