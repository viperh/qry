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

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use hex::encode;
use oracledb::{
    Connection, JsonValue, OracleIntervalDS, OracleIntervalYM, OracleNumber, OracleTimestamp, Row,
    Vector,
};
use rustls_pki_types::ServerName;
use serde::{Deserialize, Serialize};

use crate::DatabaseType;
use crate::shared::{Database, QueryResult, SslMode};

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
            ssl_mode: SslMode::Prefer,
            wallet: None,
        }
    }

    fn tls() -> OracleConfig {
        OracleConfig {
            ssl_mode: SslMode::VerifyFull,
            ..config()
        }
    }

    /// The wallet tests need real files; each gets its own directory.
    fn wallet_dir(name: &str, pem: Option<&str>) -> String {
        let dir = std::env::temp_dir().join(format!("qry-wallet-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        if let Some(pem) = pem {
            fs::write(dir.join("ewallet.pem"), pem).unwrap();
        }
        dir.display().to_string()
    }

    /// Accepts one connection on loopback, connects to it with `config`,
    /// and answers with the first bytes the driver sent.
    async fn first_bytes(config: OracleConfig) -> [u8; 5] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::Read;
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 5];
            stream.read_exact(&mut bytes).unwrap();
            bytes
        });
        let config = OracleConfig {
            host: "127.0.0.1".into(),
            port,
            ..config
        };
        // The server hangs up after reading, so this is always an error.
        assert!(Oracle::connect(config).await.is_err());
        server.join().unwrap()
    }

    #[test]
    fn label_is_user_at_host_port_service() {
        assert_eq!(config().label(), "alice@db.invalid:1521/ORCLPDB1");
        assert_eq!(config().connect_string(), "//db.invalid:1521/ORCLPDB1");
    }

    #[test]
    fn require_and_stricter_go_over_tcps() {
        for mode in SslMode::ALL {
            let config = OracleConfig {
                ssl_mode: mode,
                ..config()
            };
            let tls = matches!(
                mode,
                SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull
            );
            assert_eq!(config.uses_tls(), tls, "{mode}");
            let expected = if tls { "tcps://" } else { "//" };
            assert!(config.connect_string().starts_with(expected), "{mode}");
        }
        let descriptor = tls().driver_config().unwrap().get_connect_descriptor();
        assert!(
            descriptor.to_lowercase().contains("protocol=tcps"),
            "{descriptor}"
        );
    }

    #[test]
    fn the_driver_accepts_the_connect_string() {
        let parsed = config().driver_config().unwrap();
        assert_eq!(parsed.user(), Some("alice"));
        let descriptor = parsed.get_connect_descriptor();
        assert!(descriptor.contains("db.invalid"), "{descriptor}");
        assert!(descriptor.contains("1521"), "{descriptor}");
        assert!(descriptor.contains("ORCLPDB1"), "{descriptor}");
        assert!(!descriptor.contains("hunter2"), "{descriptor}");
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

    #[test]
    fn json_columns_render_as_compact_json() {
        let object = JsonValue::JsonObject(HashMap::from([
            (
                "name".to_string(),
                JsonValue::String("a \"b\"\n\u{1}".into()),
            ),
            (
                "tags".to_string(),
                JsonValue::JsonArray(vec![
                    JsonValue::Boolean(true),
                    JsonValue::Null,
                    JsonValue::BinaryDouble(1.5),
                ]),
            ),
            ("id".to_string(), JsonValue::Raw(vec![0xde, 0xad])),
        ]));
        assert_eq!(
            json(&object),
            r#"{"id":"dead","name":"a \"b\"\n\u0001","tags":[true,null,1.5]}"#
        );
    }

    #[tokio::test]
    async fn a_wallet_without_tls_is_refused_before_connecting() {
        let config = OracleConfig {
            wallet: Some("/etc/oracle/wallet".into()),
            ..config()
        };
        // `db.invalid` never resolves, so an error here could only come
        // from the wallet check, which runs first.
        let message = match Oracle::connect(config).await {
            Ok(_) => panic!("a wallet over plain TCP must not connect"),
            Err(err) => err.to_string(),
        };
        assert!(message.contains("only used over TLS"), "{message}");
        assert!(message.contains("/etc/oracle/wallet"), "{message}");
    }

    #[tokio::test]
    async fn a_host_no_certificate_can_name_is_refused_before_connecting() {
        let config = OracleConfig {
            host: "not a host".into(),
            ..tls()
        };
        let message = Oracle::connect(config).await.err().unwrap().to_string();
        assert!(message.contains("not a host name"), "{message}");
    }

    #[tokio::test]
    async fn tls_is_negotiated_before_anything_else_is_sent() {
        // A TLS record starts 0x16 0x03: a handshake, then the version.
        let bytes = first_bytes(tls()).await;
        assert_eq!(bytes[..2], [0x16, 0x03], "{bytes:02x?}");

        // Without TLS the first thing on the wire is a TNS CONNECT packet,
        // type 1 in the fifth byte, and never a TLS record.
        let bytes = first_bytes(config()).await;
        assert_eq!(bytes[4], 1, "{bytes:02x?}");
        assert_ne!(bytes[0], 0x16, "{bytes:02x?}");
    }

    /// A TLS server on loopback with a certificate for 127.0.0.1, signed by
    /// a test CA no system trusts. `testdata/ca.pem` is that CA; its key was
    /// thrown away after signing. Answers with the first bytes read inside
    /// the TLS session, or the handshake error.
    fn tls_server() -> (u16, std::thread::JoinHandle<Result<[u8; 5], String>>) {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use std::io::Read;

        let certs = CertificateDer::pem_slice_iter(include_bytes!("../testdata/server.pem"))
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let key = PrivateKeyDer::from_pem_slice(include_bytes!("../testdata/server.key")).unwrap();
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(Arc::new(tls)).unwrap();
            let mut session = rustls::Stream::new(&mut conn, &mut stream);
            let mut bytes = [0; 5];
            session.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(bytes)
        });
        (port, server)
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_refused() {
        let (port, server) = tls_server();
        let config = OracleConfig {
            host: "127.0.0.1".into(),
            port,
            ..tls()
        };
        let message = Oracle::connect(config).await.err().unwrap().to_string();
        assert!(message.to_lowercase().contains("certificate"), "{message}");
        assert!(
            server.join().unwrap().is_err(),
            "nothing may cross an untrusted session"
        );
    }

    #[tokio::test]
    async fn a_wallet_can_add_the_ca_to_trust() {
        let dir = wallet_dir("ca", Some(include_str!("../testdata/ca.pem")));
        let (port, server) = tls_server();
        let config = OracleConfig {
            host: "127.0.0.1".into(),
            port,
            wallet: Some(dir),
            ..tls()
        };
        // The server hangs up once it has read, so connecting still fails,
        // but only after the handshake: the TNS CONNECT came through TLS.
        assert!(Oracle::connect(config).await.is_err());
        let bytes = server.join().unwrap().unwrap();
        assert_eq!(bytes[4], 1, "{bytes:02x?}");
    }

    #[tokio::test]
    async fn a_missing_wallet_file_says_where_it_looked() {
        let dir = wallet_dir("missing", None);
        let config = OracleConfig {
            wallet: Some(dir.clone()),
            host: "127.0.0.1".into(),
            port: 1,
            ..tls()
        };
        let message = Oracle::connect(config).await.err().unwrap().to_string();
        assert!(message.contains("ewallet.pem"), "{message}");
        assert!(message.contains(&dir), "{message}");
    }

    #[tokio::test]
    async fn an_encrypted_wallet_key_is_explained() {
        let pem =
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
        let dir = wallet_dir("encrypted", Some(pem));
        let config = OracleConfig {
            wallet: Some(dir),
            host: "127.0.0.1".into(),
            port: 1,
            ..tls()
        };
        let message = Oracle::connect(config).await.err().unwrap().to_string();
        assert!(message.contains("encrypted"), "{message}");
        assert!(message.contains("wallet password"), "{message}");
    }

    /// Nothing listens on port 1 on loopback, so the connection is refused
    /// at once rather than waiting for a timeout. It must be an `Err`.
    #[tokio::test]
    async fn an_unreachable_server_is_an_error_not_a_panic() {
        let config = OracleConfig {
            host: "127.0.0.1".into(),
            port: 1,
            ..config()
        };
        assert!(Oracle::connect(config).await.is_err());
    }
}
