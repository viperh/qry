use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use hex::encode;
use serde::{Deserialize, Serialize};
use shared::QueryResult;
use tokio_rusqlite::{Connection, OpenFlags, rusqlite, types::ValueRef};

use crate::{DatabaseType, shared, shared::Database};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SqliteConfig {
    pub path: PathBuf,
    pub read_only: bool,
}

impl SqliteConfig {
    pub fn new(path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            path: path.into(),
            read_only,
        }
    }
}

pub struct Sqlite {
    conn: Connection,
    config: SqliteConfig,
}

/// Shows only the (redacted) config: the driver's connection handle has no
/// `Debug` of its own.
impl std::fmt::Debug for Sqlite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sqlite")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Sqlite {
    pub async fn connect(config: SqliteConfig) -> Result<Self> {
        let flags = if config.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };

        let conn = Connection::open_with_flags(&config.path, flags).await?;
        Ok(Self { conn, config })
    }
}

impl Sqlite {
    /// Runs `sql` with `params` bound to its placeholders, in order.
    async fn run(&self, sql: &str, params: Vec<String>) -> Result<QueryResult> {
        let sql = sql.to_string();

        let result = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                // SQLite types values, not columns: a column's type is that
                // of its first value that is not NULL.
                let mut types: Vec<&'static str> = vec![""; names.len()];

                let mut rows = Vec::new();
                let mut cursor = stmt.query(rusqlite::params_from_iter(params.iter()))?;

                while let Some(row) = cursor.next()? {
                    let mut cells = Vec::with_capacity(names.len());
                    for (i, type_name) in types.iter_mut().enumerate() {
                        let value = row.get_ref(i)?;
                        if type_name.is_empty() {
                            *type_name = match value {
                                ValueRef::Null => "",
                                ValueRef::Integer(_) => "integer",
                                ValueRef::Real(_) => "real",
                                ValueRef::Text(_) => "text",
                                ValueRef::Blob(_) => "blob",
                            };
                        }
                        cells.push(render(value));
                    }
                    rows.push(cells);
                }

                let affected = rows.len() as u64;
                let columns = names
                    .into_iter()
                    .zip(types)
                    .map(|(name, type_name)| shared::ColumnMeta::new(name, type_name))
                    .collect();
                Ok::<_, tokio_rusqlite::Error>(QueryResult {
                    columns,
                    rows,
                    affected: Some(affected),
                    elapsed: std::time::Duration::ZERO,
                    truncated: false,
                })
            })
            .await
            .map_err(positioned)?;
        Ok(result)
    }
}

#[async_trait]
impl Database for Sqlite {
    fn kind(&self) -> DatabaseType {
        DatabaseType::Sqlite
    }

    fn label(&self) -> String {
        self.config.path.display().to_string()
    }

    async fn query(&self, sql: &str) -> Result<QueryResult> {
        self.run(sql, Vec::new()).await
    }

    async fn query_bound(&self, sql: &str, params: &[String]) -> Result<QueryResult> {
        self.run(sql, params.to_vec()).await
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let sql = sql.to_string();

        let affected = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let affected = stmt.execute([])?;
                Ok::<_, tokio_rusqlite::Error>(affected as u64)
            })
            .await?;

        Ok(affected)
    }
}

fn render(value: ValueRef<'_>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Some(format!("x'{}'", encode(b))),
    }
}

/// SQLite says which token of the statement it could not take; that offset
/// rides along in a [`QueryError`](shared::QueryError).
fn positioned(e: tokio_rusqlite::Error<tokio_rusqlite::Error>) -> anyhow::Error {
    match e {
        tokio_rusqlite::Error::Error(tokio_rusqlite::Error::Error(
            rusqlite::Error::SqlInputError { msg, offset, .. },
        )) => anyhow::Error::new(shared::QueryError {
            message: msg,
            position: usize::try_from(offset).ok(),
        }),
        e => anyhow::Error::new(e),
    }
}
