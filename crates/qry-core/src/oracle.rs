//! Oracle Database, through rust-oracle (which bundles ODPI-C).
//!
//! The driver is blocking and needs the Oracle Instant Client present at
//! runtime, so every call into it goes through `spawn_blocking` and the
//! connection lives behind an `Arc<Mutex<_>>` shared with those closures.

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use oracle::Connection;
use serde::{Deserialize, Serialize};

use crate::shared::{Database, QueryResult};
use crate::DatabaseType;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// A service name or SID: what Postgres calls the database.
    pub service: String,
    /// Set as the `CURRENT_SCHEMA` right after connecting.
    #[serde(default)]
    pub schema: Option<String>,
    /// Directory of a TLS wallet. qry has no TLS yet, so a wallet is
    /// refused rather than quietly ignored.
    #[serde(default)]
    pub wallet: Option<String>,
}

impl OracleConfig {
    /// `user@host:port/service`, the shape the other backends label with.
    pub fn label(&self) -> String {
        format!("{}@{}:{}/{}", self.user, self.host, self.port, self.service)
    }

    /// The Easy Connect string ODPI-C expects.
    fn connect_string(&self) -> String {
        format!("//{}:{}/{}", self.host, self.port, self.service)
    }
}

/// Quotes an identifier for use in SQL, e.g. `my"schema` → `"my""schema"`.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub struct Oracle {
    /// Shared with the blocking closures, which is why it is not just a
    /// `Connection`. The driver has no async API of its own.
    conn: Arc<Mutex<Connection>>,
    config: OracleConfig,
}

impl Oracle {
    pub async fn connect(config: OracleConfig) -> Result<Self> {
        if let Some(wallet) = &config.wallet {
            bail!("the wallet `{wallet}` needs TLS, which qry does not support yet");
        }

        let opened = config.clone();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let mut conn = Connection::connect(
                &opened.user,
                &opened.password,
                opened.connect_string(),
            )?;
            // The other backends commit as they go, so this one does too.
            conn.set_autocommit(true);

            if let Some(schema) = &opened.schema {
                conn.execute(
                    &format!("ALTER SESSION SET CURRENT_SCHEMA = {}", quote_ident(schema)),
                    &[],
                )?;
            }
            Ok(conn)
        })
        .await??;

        Ok(Self { conn: Arc::new(Mutex::new(conn)), config })
    }

    /// Runs `f` against the connection on the blocking pool.
    async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| anyhow!("the Oracle connection is unusable: a query panicked"))?;
            f(&guard)
        })
        .await?
    }
}

#[async_trait]
impl Database for Oracle {
    fn kind(&self) -> DatabaseType {
        DatabaseType::Oracle
    }

    fn label(&self) -> String {
        self.config.label()
    }

    async fn query(&self, sql: &str) -> Result<QueryResult> {
        let sql = sql.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.statement(&sql).build()?;
            // Unlike the other drivers, this one refuses anything that is not
            // a query. The editor sends every statement through here, so a
            // CREATE or an INSERT is run instead of rejected.
            if !stmt.is_query() {
                stmt.execute(&[])?;
                return Ok(QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    affected: Some(stmt.row_count()?),
                });
            }
            let result_set = stmt.query(&[])?;

            let columns: Vec<String> =
                result_set.column_info().iter().map(|c| c.name().to_string()).collect();

            let mut rows: Vec<Vec<Option<String>>> = Vec::new();
            for row in result_set {
                let row = row?;
                let mut cells = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    // `Option<String>` renders every type as text and maps
                    // a NULL to `None`.
                    cells.push(row.get::<usize, Option<String>>(i)?);
                }
                rows.push(cells);
            }

            let affected = rows.len() as u64;
            Ok(QueryResult { columns, rows, affected: Some(affected) })
        })
        .await
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let sql = sql.to_string();
        self.with_conn(move |conn| {
            let stmt = conn.execute(&sql, &[])?;
            Ok(stmt.row_count()?)
        })
        .await
    }

    /// Oracle keeps no `information_schema`. This follows `CURRENT_SCHEMA`,
    /// so it answers for the schema the config asked for rather than only
    /// for what the connected user owns, and it includes views as the other
    /// backends do.
    async fn tables(&self) -> Result<Vec<String>> {
        let result = self
            .query(
                "SELECT table_name AS name FROM all_tables \
                 WHERE owner = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') \
                 UNION ALL \
                 SELECT view_name FROM all_views \
                 WHERE owner = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') \
                 ORDER BY 1",
            )
            .await?;
        Ok(result.rows.iter().filter_map(|row| row.first().cloned().flatten()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnectionConfig;

    fn config() -> OracleConfig {
        OracleConfig {
            host: "db.invalid".into(),
            port: 1521,
            user: "alice".into(),
            password: "hunter2".into(),
            service: "ORCLPDB1".into(),
            schema: None,
            wallet: None,
        }
    }

    #[test]
    fn label_is_user_at_host_port_service() {
        assert_eq!(config().label(), "alice@db.invalid:1521/ORCLPDB1");
        assert_eq!(config().connect_string(), "//db.invalid:1521/ORCLPDB1");
    }

    #[test]
    fn debug_output_never_contains_the_password() {
        let shown = format!("{:?}", ConnectionConfig::Oracle(config()));
        assert_eq!(shown, "Oracle(alice@db.invalid:1521/ORCLPDB1)");
        assert!(!shown.contains("hunter2"));
    }

    #[test]
    fn identifiers_are_quoted_for_the_session_schema() {
        assert_eq!(quote_ident("hr"), "\"hr\"");
        assert_eq!(quote_ident("my\"schema"), "\"my\"\"schema\"");
    }

    #[tokio::test]
    async fn a_wallet_is_refused_before_connecting() {
        let config = OracleConfig { wallet: Some("/etc/oracle/wallet".into()), ..config() };
        // `db.invalid` never resolves, so an error here could only come
        // from the wallet check, which runs first.
        let message = match Oracle::connect(config).await {
            Ok(_) => panic!("a config with a wallet must not connect"),
            Err(err) => err.to_string(),
        };
        assert!(message.contains("does not support"), "{message}");
        assert!(message.contains("/etc/oracle/wallet"), "{message}");
    }

    /// Without an Oracle Instant Client this fails at load time; with one it
    /// fails on a refused connection. Either way it must be an `Err`, and
    /// port 1 on loopback keeps it fast rather than waiting for a timeout.
    #[tokio::test]
    async fn an_unreachable_server_is_an_error_not_a_panic() {
        let config = OracleConfig { host: "127.0.0.1".into(), port: 1, ..config() };
        assert!(Oracle::connect(config).await.is_err());
    }
}
