use std::path::PathBuf;
use tokio_rusqlite::{Connection, OpenFlags};
use tokio_rusqlite::types::ValueRef;
use serde::{Serialize, Deserialize};
use anyhow::Result;
use async_trait::async_trait;
use hex::encode;
use shared::{QueryResult};
use crate::{shared, DatabaseType};
use crate::shared::Database;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SqliteConfig {
    pub path: PathBuf,
    pub read_only: bool
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

impl Sqlite {
    pub async fn connect(config: SqliteConfig) -> Result<Self> {
        let flags = if config.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };

        let conn = Connection::open_with_flags(&config.path, flags).await?;
        Ok(Self {
            conn,
            config
        }
        )
    }
}

#[async_trait]
impl Database for Sqlite{
    fn kind(&self) -> DatabaseType {
        DatabaseType::Sqlite
    }

    fn label(&self) -> String {
        self.config.path.display().to_string()
    }

    async fn query(&self, sql: &str) -> Result<QueryResult> {
        let sql = sql.to_string();

        let result = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let columns: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();

                let mut rows = Vec::new();
                let mut cursor = stmt.query([])?;

                while let Some(row) = cursor.next()? {
                    let mut cells = Vec::with_capacity(columns.len());
                    for i in 0..columns.len() {
                        cells.push(render(row.get_ref(i)?));
                    }
                    rows.push(cells);
                }

                let affected = rows.len() as u64;
                Ok::<_, tokio_rusqlite::Error>(QueryResult {
                    columns,
                    rows,
                    affected: Some(affected),
                })
            })
            .await?;
        Ok(result)
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let sql = sql.to_string();

        let affected = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let affected = stmt.execute([])?;
                Ok::<_, tokio_rusqlite::Error>(QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    affected: Some(affected as u64),
                })
            })
            .await?;

        Ok(affected.affected.unwrap())
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