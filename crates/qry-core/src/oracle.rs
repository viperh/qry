//! Oracle Database, through oracledb: Oracle's own thin driver, written in
//! pure Rust, so no Oracle Client libraries are needed at runtime.
//!
//! The driver is blocking, so every call into it goes through
//! `spawn_blocking` and the connection lives behind an `Arc<Mutex<_>>`
//! shared with those closures.
//!
//! TLS is TCPS: the driver negotiates it when the connect string names that
//! protocol, and checks the server's certificate and host name against the
//! Mozilla roots plus any certificates in the wallet.

use std::{
    collections::HashMap,
    fmt::Write as _,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use hex::encode;
use oracledb::{
    Connection, JsonValue, OracleIntervalDS, OracleIntervalYM, OracleNumber, OracleTimestamp, Row,
    Vector,
};
use rustls_pki_types::ServerName;
use serde::{Deserialize, Serialize};

use crate::{
    DatabaseType,
    shared::{Database, QueryResult, SslMode},
};

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
    /// `require` and stricter connect over TCPS; the rest over plain TCP.
    #[serde(default)]
    pub ssl_mode: SslMode,
    /// Directory holding `ewallet.pem`: certificates to trust, and a client
    /// certificate with its key for mutual TLS. Only used over TCPS.
    #[serde(default)]
    pub wallet: Option<String>,
}

impl OracleConfig {
    /// `user@host:port/service`, the shape the other backends label with.
    pub fn label(&self) -> String {
        format!("{}@{}:{}/{}", self.user, self.host, self.port, self.service)
    }

    /// Whether this connection goes over TCPS. The driver always checks the
    /// certificate and the host name and cannot be told not to, so
    /// `require` and `verify-ca` get the checks of `verify-full`: stricter
    /// than asked, never weaker. `prefer` and `allow` stay on plain TCP, as
    /// they do for the other backends; an Oracle listener serves TCPS on a
    /// port of its own, so there is nothing to fall back from.
    pub fn uses_tls(&self) -> bool {
        matches!(
            self.ssl_mode,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull
        )
    }

    /// The Easy Connect string the driver parses.
    fn connect_string(&self) -> String {
        let protocol = if self.uses_tls() { "tcps://" } else { "//" };
        format!("{protocol}{}:{}/{}", self.host, self.port, self.service)
    }

    /// Everything that can be refused without the network or the disk.
    fn check(&self) -> Result<()> {
        if let Some(wallet) = &self.wallet
            && !self.uses_tls()
        {
            bail!(
                "the wallet `{wallet}` is only used over TLS; \
                 set the SSL mode to `require` or stricter"
            );
        }
        // The driver would panic on a name a certificate cannot carry.
        if self.uses_tls() && ServerName::try_from(self.host.as_str()).is_err() {
            bail!(
                "`{}` is not a host name a TLS certificate can be checked against",
                self.host
            );
        }
        Ok(())
    }

    /// What `oracledb::connect` takes. Parsing the connect string happens
    /// here, so a malformed one fails before any network traffic.
    fn driver_config(&self) -> Result<oracledb::Config> {
        let config = oracledb::Config::default()
            .set_credentials(&self.user, &self.password)
            .set_connect_string(&self.connect_string())
            .ora()?;
        Ok(match &self.wallet {
            Some(wallet) => config.set_wallet_location(wallet.clone()),
            None => config,
        })
    }
}

/// Reads the wallet before the driver does, for the case its own error does
/// not explain: a private key locked with a password, which qry has no way
/// to ask for yet. Blocking: call it from `spawn_blocking`.
fn check_wallet(dir: &str) -> Result<()> {
    let file = Path::new(dir).join("ewallet.pem");
    let contents = fs::read_to_string(&file)
        .with_context(|| format!("could not read the wallet file {}", file.display()))?;
    if contents.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----") {
        bail!(
            "the private key in {} is encrypted, and qry cannot take a wallet password yet",
            file.display()
        );
    }
    Ok(())
}

/// `oracledb::Error` does not implement `std::error::Error`, so it cannot
/// cross into `anyhow` with a bare `?`. Its `Display` carries the ORA- code.
trait OraResult<T> {
    fn ora(self) -> Result<T>;
}

impl<T> OraResult<T> for std::result::Result<T, oracledb::Error> {
    fn ora(self) -> Result<T> {
        self.map_err(|e| anyhow!("{e}"))
    }
}

/// Quotes an identifier for use in SQL, e.g. `my"schema` → `"my""schema"`.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// One cell as text, or `None` for NULL. The driver only converts character
/// columns to `String`, so each type it can hand back is tried in turn; a
/// conversion that does not apply is an error, not a panic, and costs no
/// round trip.
fn cell(row: &Row, i: usize) -> Result<Option<String>> {
    macro_rules! try_as {
        ($t:ty, $show:expr) => {
            if let Ok(value) = row.get::<Option<$t>>(i) {
                return Ok(value.map($show));
            }
        };
    }
    try_as!(String, |v| v);
    try_as!(OracleNumber, |v| v.to_string());
    try_as!(f64, |v| v.to_string());
    try_as!(f32, |v| v.to_string());
    try_as!(OracleTimestamp, |v| v.to_string());
    try_as!(OracleIntervalDS, |v| v.to_string());
    try_as!(OracleIntervalYM, |v| v.to_string());
    try_as!(bool, |v| v.to_string());
    // Written the way the SQLite backend writes a BLOB.
    try_as!(Vec<u8>, |v| format!("x'{}'", encode(v)));
    try_as!(JsonValue, |v| json(&v));
    try_as!(Vector, |v| format!("{v:?}"));

    let column = &row.columns()[i];
    bail!(
        "column {} has type {}, which qry cannot show",
        column.name(),
        column.data_type()
    )
}

/// Compact JSON text for a JSON column. Object keys are sorted, since the
/// driver hands them over in a `HashMap` and the grid should not reshuffle
/// between runs.
fn json(value: &JsonValue) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &JsonValue) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Boolean(v) => out.push_str(if *v { "true" } else { "false" }),
        JsonValue::Number(v) => out.push_str(&v.to_string()),
        JsonValue::BinaryDouble(v) => out.push_str(&v.to_string()),
        JsonValue::BinaryFloat(v) => out.push_str(&v.to_string()),
        JsonValue::String(v) => write_json_string(out, v),
        JsonValue::Timestamp(v) => write_json_string(out, &v.to_string()),
        JsonValue::IntervalDS(v) => write_json_string(out, &v.to_string()),
        JsonValue::IntervalYM(v) => write_json_string(out, &v.to_string()),
        JsonValue::Raw(v) | JsonValue::JsonId(v) => write_json_string(out, &encode(v)),
        JsonValue::Vector(v) => write_json_string(out, &format!("{v:?}")),
        JsonValue::JsonArray(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        JsonValue::JsonObject(fields) => write_json_object(out, fields),
    }
}

fn write_json_object(out: &mut String, fields: &HashMap<String, JsonValue>) {
    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort();
    out.push('{');
    for (n, key) in keys.into_iter().enumerate() {
        if n > 0 {
            out.push(',');
        }
        write_json_string(out, key);
        out.push(':');
        write_json(out, &fields[key]);
    }
    out.push('}');
}

fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub struct Oracle {
    /// Shared with the blocking closures, which is why it is not just a
    /// `Connection`. The driver has no async API of its own.
    conn: Arc<Mutex<Connection>>,
    config: OracleConfig,
}

impl Oracle {
    pub async fn connect(config: OracleConfig) -> Result<Self> {
        config.check()?;

        let driver_config = config.driver_config()?;
        let wallet = config.wallet.clone();
        let schema = config.schema.clone();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            if let Some(wallet) = &wallet {
                check_wallet(wallet)?;
            }
            let conn = oracledb::connect(driver_config).ora()?;
            if let Some(schema) = &schema {
                conn.execute(
                    &format!("ALTER SESSION SET CURRENT_SCHEMA = {}", quote_ident(schema)),
                    &[],
                )
                .ora()?;
            }
            Ok(conn)
        })
        .await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            config,
        })
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

/// Runs a statement that is not a query, then commits. The driver has no
/// autocommit switch, and the other backends commit as they go, so this one
/// does too. A statement that fails is not committed.
fn execute_and_commit(conn: &Connection, sql: &str) -> Result<u64> {
    let affected = conn.execute(sql, &[]).ora()?.rows_affected();
    conn.commit().ora()?;
    Ok(affected)
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
            let mut stmt = conn.statement(&sql).ora()?.build().ora()?;
            // The driver refuses to run anything but a query through
            // `query`. The editor sends every statement through here, so a
            // CREATE or an INSERT is run instead of rejected.
            if !stmt.is_query() {
                let affected = stmt.execute(&[]).ora()?.rows_affected();
                conn.commit().ora()?;
                return Ok(QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    affected: Some(affected),
                });
            }
            let cursor = stmt.query(&[]).ora()?;

            let columns: Vec<String> = cursor
                .columns()
                .iter()
                .map(|c| c.name().to_string())
                .collect();

            let mut rows: Vec<Vec<Option<String>>> = Vec::new();
            for row in cursor {
                let row = row.ora()?;
                let mut cells = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    cells.push(cell(&row, i)?);
                }
                rows.push(cells);
            }

            let affected = rows.len() as u64;
            Ok(QueryResult {
                columns,
                rows,
                affected: Some(affected),
            })
        })
        .await
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let sql = sql.to_string();
        self.with_conn(move |conn| execute_and_commit(conn, &sql))
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
        Ok(result
            .rows
            .iter()
            .filter_map(|row| row.first().cloned().flatten())
            .collect())
    }
}

#[cfg(test)]
#[path = "oracle_tests.rs"]
mod oracle_tests;
