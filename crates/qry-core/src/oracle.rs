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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
    shared::{
        self, Batches, ColumnMeta, Database, QueryError, QueryResult, RowSender, SslMode,
        StreamItem, first_column,
    },
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

/// Written by hand so the password never reaches a log or panic message.
impl std::fmt::Debug for OracleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OracleConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"[redacted]")
            .field("service", &self.service)
            .field("schema", &self.schema)
            .field("ssl_mode", &self.ssl_mode)
            .field("wallet", &self.wallet)
            .finish()
    }
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

/// A failed statement as a [`QueryError`], with the position the server
/// reported. Oracle counts it from 0, in characters, and sends 0 when it
/// has none, so a statement that fails at its very first character gets no
/// position.
fn query_error(e: oracledb::Error, sql: &str) -> anyhow::Error {
    match e.kind() {
        oracledb::ErrorKind::DbError(db) => anyhow::Error::new(QueryError {
            message: e.to_string(),
            position: match db.offset() {
                0 => None,
                offset => QueryError::char_position(sql, offset + 1),
            },
        }),
        _ => anyhow!("{e}"),
    }
}

/// How a date or timestamp column's values are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Temporal {
    /// `DATE`: always has a time of day, written only when it is not
    /// midnight.
    Date,
    Timestamp,
    /// `TIMESTAMP WITH TIME ZONE`: the offset follows, as `+02:00`.
    TimestampTz,
}

impl Temporal {
    /// From the column's type as `Metadata::data_type` writes it.
    fn of(data_type: &str) -> Self {
        if data_type.starts_with("DATE") {
            Self::Date
        } else if data_type.ends_with("WITH TIME ZONE") && !data_type.contains("LOCAL") {
            Self::TimestampTz
        } else {
            Self::Timestamp
        }
    }
}

/// `YYYY-MM-DD`, or `YYYY-MM-DD HH:MM:SS[.fff]`: the form every backend
/// shows dates and timestamps in. The driver's own `Display` writes
/// `1990-04-01T00:00:00.000000000Z`, even for a `DATE`.
fn temporal_text(ts: &OracleTimestamp, kind: Temporal) -> String {
    let date = format!("{:04}-{:02}-{:02}", ts.year(), ts.month(), ts.day());
    let (hour, minute, second, nanos) = (ts.hour(), ts.minute(), ts.second(), ts.nanoseconds());
    if kind == Temporal::Date && (hour, minute, second, nanos) == (0, 0, 0, 0) {
        return date;
    }
    let mut out = format!("{date} {hour:02}:{minute:02}:{second:02}");
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        out.push('.');
        out.push_str(fraction.trim_end_matches('0'));
    }
    if kind == Temporal::TimestampTz {
        let _ = write!(
            out,
            "{:+03}:{:02}",
            ts.tz_hour_offset(),
            ts.tz_minute_offset().unsigned_abs()
        );
    }
    out
}

/// A `NUMBER(p,s)` with its `s` decimals, as the other backends write a
/// `numeric(p,s)`: `-12.50`, not the driver's `-12.5`.
fn number_text(text: String, scale: i8) -> String {
    let Ok(scale) = usize::try_from(scale) else {
        return text;
    };
    if scale == 0 || text.contains(['e', 'E']) {
        return text;
    }
    let decimals = text
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    if decimals >= scale {
        return text;
    }
    let mut out = text;
    if decimals == 0 {
        out.push('.');
    }
    out.extend(std::iter::repeat_n('0', scale - decimals));
    out
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
    let column = &row.columns()[i];
    try_as!(String, |v| v);
    // `precision` 0 is a `NUMBER` without one, whose scale means nothing.
    let scale = if column.precision() == 0 {
        0
    } else {
        column.scale()
    };
    try_as!(OracleNumber, |v| number_text(v.to_string(), scale));
    try_as!(f64, |v| v.to_string());
    try_as!(f32, |v| v.to_string());
    let temporal = Temporal::of(&column.data_type());
    try_as!(OracleTimestamp, |v| temporal_text(&v, temporal));
    try_as!(OracleIntervalDS, |v| v.to_string());
    try_as!(OracleIntervalYM, |v| v.to_string());
    try_as!(bool, |v| v.to_string());
    // `\x` and lowercase hex, as Postgres writes a `bytea`.
    try_as!(Vec<u8>, |v| format!("\\x{}", encode(v)));
    try_as!(JsonValue, |v| json(&v));
    try_as!(Vector, |v| format!("{v:?}"));

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
    /// Commit after each statement that is not a query. The driver has no
    /// autocommit switch of its own, so this is it; on by default, as the
    /// other backends commit as they go.
    autocommit: Arc<AtomicBool>,
}

/// Shows only the (redacted) config: the driver's connection handle has no
/// `Debug` of its own.
impl std::fmt::Debug for Oracle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Oracle")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
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
            autocommit: Arc::new(AtomicBool::new(true)),
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

/// Runs a statement that is not a query, then commits if `autocommit`
/// says to. A statement that fails is not committed.
fn execute_and_commit(conn: &Connection, sql: &str, autocommit: bool) -> Result<u64> {
    let affected = conn.execute(sql, &[]).ora()?.rows_affected();
    if autocommit {
        conn.commit().ora()?;
    }
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

    async fn query_limited(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        let sql = sql.to_string();
        let autocommit = self.autocommit.load(Ordering::SeqCst);
        self.with_conn(move |conn| {
            let failed = |e| query_error(e, &sql);
            let mut stmt = conn
                .statement(&sql)
                .map_err(failed)?
                .build()
                .map_err(failed)?;
            // The driver refuses to run anything but a query through
            // `query`. The editor sends every statement through here, so a
            // CREATE or an INSERT is run instead of rejected.
            if !stmt.is_query() {
                let affected = stmt.execute(&[]).map_err(failed)?.rows_affected();
                if autocommit {
                    conn.commit().ora()?;
                }
                return Ok(QueryResult::affected(affected));
            }
            let cursor = stmt.query(&[]).map_err(failed)?;

            let columns: Vec<ColumnMeta> = cursor
                .columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), c.data_type().to_string()))
                .collect();

            let mut rows: Vec<Vec<Option<String>>> = Vec::new();
            let mut truncated = false;
            for row in cursor {
                // The cursor fetches as it goes; the rest is never read.
                if max_rows > 0 && rows.len() == max_rows {
                    truncated = true;
                    break;
                }
                let row = row.map_err(failed)?;
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
                elapsed: std::time::Duration::ZERO,
                truncated,
            })
        })
        .await
    }

    /// Walks the cursor on the blocking pool, `fetch_size` rows to a round
    /// trip, handing each batch on as it fills. No more than a batch is
    /// held: a reader that falls behind blocks the walk, and once it has
    /// gone the cursor is closed with the rest unread.
    async fn query_stream(&self, sql: &str, fetch_size: usize, out: RowSender) -> Result<u64> {
        let sql = sql.to_string();
        self.with_conn(move |conn| {
            let failed = |e| query_error(e, &sql);
            let mut batches = Batches::new(fetch_size);
            let array_size = u32::try_from(fetch_size.max(1)).unwrap_or(u32::MAX);
            let mut stmt = conn
                .statement(&sql)
                .map_err(failed)?
                .fetch_array_size(array_size)
                .build()
                .map_err(failed)?;
            // As in `query_limited`: anything but a query is run, and sends
            // no rows.
            if !stmt.is_query() {
                stmt.execute(&[]).map_err(failed)?;
                conn.commit().ora()?;
                shared::blocking_send(&out, StreamItem::Columns(Vec::new()))?;
                return Ok(0);
            }
            let cursor = stmt.query(&[]).map_err(failed)?;
            let columns: Vec<ColumnMeta> = cursor
                .columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), c.data_type().to_string()))
                .collect();
            let width = columns.len();
            shared::blocking_send(&out, StreamItem::Columns(columns))?;
            for row in cursor {
                let row = row.map_err(failed)?;
                let mut cells = Vec::with_capacity(width);
                for i in 0..width {
                    cells.push(cell(&row, i)?);
                }
                if let Some(full) = batches.push(cells) {
                    shared::blocking_send(&out, StreamItem::Rows(full))?;
                }
            }
            if let Some(rest) = batches.rest() {
                shared::blocking_send(&out, StreamItem::Rows(rest))?;
            }
            Ok(batches.sent())
        })
        .await
    }

    async fn execute(&self, sql: &str) -> Result<u64> {
        let sql = sql.to_string();
        let autocommit = self.autocommit.load(Ordering::SeqCst);
        self.with_conn(move |conn| execute_and_commit(conn, &sql, autocommit))
            .await
    }

    /// Sets the flag statements follow. Turning it on commits what is
    /// pending, as MySQL's `SET autocommit=1` does.
    async fn set_autocommit(&self, on: bool) -> Result<()> {
        if on {
            self.with_conn(|conn| conn.commit().ora()).await?;
        }
        self.autocommit.store(on, Ordering::SeqCst);
        Ok(())
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
        Ok(first_column(result))
    }

    /// Binds each value to `:1`, `:2`, … by position. The catalog queries
    /// return only character columns.
    async fn query_bound(&self, sql: &str, params: &[String]) -> Result<QueryResult> {
        let sql = sql.to_string();
        let params = params.to_vec();
        self.with_conn(move |conn| {
            let binds: Vec<&dyn oracledb::ToDbValue> = params
                .iter()
                .map(|p| p as &dyn oracledb::ToDbValue)
                .collect();
            let cursor = conn.query(&sql, &binds).ora()?;
            let columns: Vec<ColumnMeta> = cursor
                .columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), c.data_type().to_string()))
                .collect();
            let mut rows = Vec::new();
            for row in cursor {
                let row = row.ora()?;
                let mut cells = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    cells.push(cell(&row, i)?);
                }
                rows.push(cells);
            }
            Ok(QueryResult {
                columns,
                rows,
                affected: None,
                elapsed: std::time::Duration::ZERO,
                truncated: false,
            })
        })
        .await
    }
}

#[cfg(test)]
#[path = "oracle_tests.rs"]
mod oracle_tests;
