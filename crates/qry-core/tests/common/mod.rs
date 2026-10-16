#![allow(dead_code, unreachable_pub)]
//! Shared helpers for the database integration suite (T004): a throw-away
//! database per test, the URL lookup, and the skip rules.
//!
//! Server URLs come from `QRY_TEST_POSTGRES_URL`, `QRY_TEST_MYSQL_URL`,
//! `QRY_TEST_MARIADB_URL` and `QRY_TEST_ORACLE_URL`. A test whose server
//! has no URL skips, except on CI (`CI` set), where a missing URL is a
//! broken setup and panics. `QRY_DB_TESTS=skip` opts out explicitly, on CI
//! too. SQLite needs no server and always runs.

pub mod mysql_family;

use std::{
    collections::hash_map::RandomState,
    env, fmt,
    hash::{BuildHasher, Hasher},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use qry_core::{
    ConnectionConfig, Driver, QueryResult, SslMode,
    mariadb::MariadbConfig,
    mysql::MySqlConfig,
    oracle::OracleConfig,
    postgres::PostgresConfig,
    sql::{self, Dialect},
    sqlite::SqliteConfig,
};

/// Every database and schema this suite creates starts with this.
pub const PREFIX: &str = "qry_test_";

/// The password of the Oracle users the suite creates.
const ORACLE_TEST_PASSWORD: &str = "Qry_test_pw1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Sqlite,
    Postgres,
    Mysql,
    MariaDb,
    Oracle,
}

impl Backend {
    /// The backends that need a server.
    pub const SERVERS: [Backend; 4] = [
        Backend::Postgres,
        Backend::Mysql,
        Backend::MariaDb,
        Backend::Oracle,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Backend::Sqlite => "sqlite",
            Backend::Postgres => "postgres",
            Backend::Mysql => "mysql",
            Backend::MariaDb => "mariadb",
            Backend::Oracle => "oracle",
        }
    }

    /// The variable holding the server's URL; SQLite has none.
    pub fn env_var(self) -> Option<&'static str> {
        match self {
            Backend::Sqlite => None,
            Backend::Postgres => Some("QRY_TEST_POSTGRES_URL"),
            Backend::Mysql => Some("QRY_TEST_MYSQL_URL"),
            Backend::MariaDb => Some("QRY_TEST_MARIADB_URL"),
            Backend::Oracle => Some("QRY_TEST_ORACLE_URL"),
        }
    }

    pub fn dialect(self) -> Dialect {
        match self {
            Backend::Sqlite => Dialect::Sqlite,
            Backend::Postgres => Dialect::Postgres,
            Backend::Mysql | Backend::MariaDb => Dialect::MySql,
            Backend::Oracle => Dialect::Oracle,
        }
    }

    /// `tests/fixtures/sql/seed_<this>.sql`; MariaDB loads MySQL's.
    pub fn seed_path(self) -> PathBuf {
        let file = match self {
            Backend::Sqlite => "seed_sqlite.sql",
            Backend::Postgres => "seed_postgres.sql",
            Backend::Mysql | Backend::MariaDb => "seed_mysql.sql",
            Backend::Oracle => "seed_oracle.sql",
        };
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/sql")
            .join(file)
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

fn env_set(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// `QRY_DB_TESTS=skip`: the explicit opt-out the CI `test` job uses.
pub fn opted_out() -> bool {
    env_set("QRY_DB_TESTS").is_some_and(|v| v.trim().eq_ignore_ascii_case("skip"))
}

/// The server's URL, unless the suite was told to skip.
pub fn url(backend: Backend) -> Option<String> {
    if opted_out() {
        return None;
    }
    env_set(backend.env_var()?)
}

/// Prints `message` once per backend and test binary, past libtest's
/// output capture, so a run without servers says what it left out.
fn notice_once(backend: Backend, message: &str) {
    static SHOWN: Mutex<Vec<Backend>> = Mutex::new(Vec::new());
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    if !shown.contains(&backend) {
        shown.push(backend);
        let _ = writeln!(std::io::stderr(), "{message}");
    }
}

/// What a test does when its server is not configured: skip with a notice,
/// or, on CI without `QRY_DB_TESTS=skip`, panic, because a CI run that
/// passes by skipping proves nothing.
pub fn skip_or_fail(backend: Backend) {
    let Some(var) = backend.env_var() else {
        return;
    };
    if opted_out() {
        notice_once(backend, &format!("skipped: {backend} (QRY_DB_TESTS=skip)"));
        return;
    }
    if env_set("CI").is_some() {
        panic!("{var} is not set on CI");
    }
    notice_once(backend, &format!("skipped: {backend} ({var} is not set)"));
}

/// Percent-decoding for the user and password parts of a URL.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The parts of `scheme://user:password@host:port/database`.
#[derive(Clone, PartialEq, Eq)]
pub struct ServerUrl {
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub database: String,
}

/// Leaves the password out.
impl fmt::Debug for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{}:{}/{}",
            self.user, self.host, self.port, self.database
        )
    }
}

pub fn parse_url(url: &str, default_port: u16) -> Result<ServerUrl> {
    let (_, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow!("a database URL needs a scheme, as in postgres://…"))?;
    let (credentials, location) = rest
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("a database URL needs user:password@"))?;
    let (user, password) = credentials.split_once(':').unwrap_or((credentials, ""));
    let (address, database) = location.split_once('/').unwrap_or((location, ""));
    let database = database.split(['?', '#']).next().unwrap_or("");
    let (host, port) = match address.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().context("the port is not a number")?),
        None => (address, default_port),
    };
    if host.is_empty() || database.is_empty() {
        bail!("a database URL needs a host and a database");
    }
    Ok(ServerUrl {
        user: decode(user),
        password: decode(password),
        host: host.to_string(),
        port,
        database: database.to_string(),
    })
}

/// The connection a server URL describes, as `backend`.
pub fn config_from_url(backend: Backend, url: &str) -> ConnectionConfig {
    let default_port = match backend {
        Backend::Postgres => 5432,
        Backend::Mysql | Backend::MariaDb => 3306,
        Backend::Oracle => 1521,
        Backend::Sqlite => 0,
    };
    let var = backend.env_var().unwrap_or("the URL");
    let u = parse_url(url, default_port).unwrap_or_else(|e| panic!("{var}: {e:#}"));
    with_target(backend, &u, &u.database)
}

/// `u`'s server, connected to `database` (for Oracle, the service).
fn with_target(backend: Backend, u: &ServerUrl, database: &str) -> ConnectionConfig {
    match backend {
        Backend::Postgres => ConnectionConfig::Postgres(PostgresConfig {
            host: u.host.clone(),
            port: u.port,
            user: u.user.clone(),
            password: u.password.clone(),
            database: database.to_string(),
            schema: None,
            ssl_mode: SslMode::Prefer,
        }),
        Backend::Mysql => ConnectionConfig::Mysql(MySqlConfig {
            host: u.host.clone(),
            port: u.port,
            user: u.user.clone(),
            password: u.password.clone(),
            database: database.to_string(),
            ssl_mode: SslMode::Prefer,
        }),
        Backend::MariaDb => ConnectionConfig::MariaDb(MariadbConfig {
            host: u.host.clone(),
            port: u.port,
            user: u.user.clone(),
            password: u.password.clone(),
            database: database.to_string(),
            ssl_mode: SslMode::Prefer,
        }),
        Backend::Oracle => ConnectionConfig::Oracle(OracleConfig {
            host: u.host.clone(),
            port: u.port,
            user: u.user.clone(),
            password: u.password.clone(),
            service: database.to_string(),
            schema: None,
            ssl_mode: SslMode::Prefer,
            wallet: None,
        }),
        Backend::Sqlite => unreachable!("SQLite has no server URL"),
    }
}

/// A new name, `qry_test_` and 32 hex digits shaped like a UUID v7: the
/// first 12 are the creation time in Unix milliseconds, which
/// [`created_at`] reads back to find stragglers.
pub fn test_db_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let millis = (now.as_millis() as u64) & 0xffff_ffff_ffff;
    let random = |salt: u64| {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_u64(salt);
        h.write_u32(std::process::id());
        h.write_u128(now.as_nanos());
        h.finish()
    };
    format!(
        "{PREFIX}{millis:012x}{:016x}{:04x}",
        random(1),
        random(2) & 0xffff
    )
}

/// When the suite created the database (or Oracle user) called `name`, or
/// `None` if the name is not one of [`test_db_name`]'s.
pub fn created_at(name: &str) -> Option<SystemTime> {
    let name = name.to_ascii_lowercase();
    let hex = name.strip_prefix(PREFIX)?.get(..12)?;
    let millis = u64::from_str_radix(hex, 16).ok()?;
    Some(UNIX_EPOCH + Duration::from_millis(millis))
}

/// Runs `sql`, panicking with the statement and the whole error chain.
pub async fn q(driver: &Driver, sql: &str) -> QueryResult {
    driver
        .query(sql)
        .await
        .unwrap_or_else(|e| panic!("{sql}\n  failed: {e:#}"))
}

/// Runs every statement of `script`, split the way the editor splits it.
pub async fn run_script(driver: &Driver, dialect: Dialect, script: &str) -> Result<()> {
    for statement in sql::split(dialect, script) {
        driver
            .query(&statement.text)
            .await
            .with_context(|| statement.text.clone())?;
    }
    Ok(())
}

/// A connection to the server's URL as given: the account that creates and
/// drops the test databases. `None` when the backend is not configured.
pub async fn admin(backend: Backend) -> Option<Driver> {
    let url = url(backend)?;
    Some(admin_at(backend, &url).await)
}

async fn admin_at(backend: Backend, url: &str) -> Driver {
    let mut driver = Driver::new();
    driver
        .connect(config_from_url(backend, url))
        .await
        .unwrap_or_else(|e| {
            panic!(
                "{} is set, but connecting to it failed: {e:#}",
                backend.env_var().unwrap_or_default()
            )
        });
    driver
}

/// A throw-away database (or schema/user for Oracle, or file for SQLite)
/// that exists for one test. Dropping it drops the database, also when the
/// test panicked; [`TestDb::cleanup`] does the same without blocking.
pub struct TestDb {
    pub backend: Backend,
    pub name: String,
    pub config: ConnectionConfig,
    /// `None` for SQLite, whose file goes with `dir`.
    admin_url: Option<String>,
    dir: Option<tempfile::TempDir>,
    cleaned: bool,
}

impl fmt::Debug for TestDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestDb")
            .field("backend", &self.backend)
            .field("name", &self.name)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl TestDb {
    /// Creates `qry_test_<…>`, connects to it, and loads
    /// `tests/fixtures/sql/seed_<backend>.sql`.
    pub async fn seeded(backend: Backend) -> Option<TestDb> {
        let db = Self::fresh(backend).await?;
        let seed = std::fs::read_to_string(backend.seed_path())
            .unwrap_or_else(|e| panic!("{}: {e}", backend.seed_path().display()));
        let driver = db.connect().await;
        if let Err(e) = run_script(&driver, backend.dialect(), &seed).await {
            panic!(
                "{} did not load on {backend}: {e:#}",
                backend.seed_path().display()
            );
        }
        drop(driver);
        Some(db)
    }

    /// An empty database. `None` when the backend is not configured (see
    /// [`skip_or_fail`]); a configured server that refuses is a panic.
    pub async fn fresh(backend: Backend) -> Option<TestDb> {
        let name = test_db_name();
        if backend == Backend::Sqlite {
            let dir = tempfile::tempdir().expect("a scratch directory");
            let path = dir.path().join(format!("{name}.sqlite"));
            return Some(TestDb {
                backend,
                name,
                config: ConnectionConfig::Sqlite(SqliteConfig::new(path, false)),
                admin_url: None,
                dir: Some(dir),
                cleaned: false,
            });
        }

        let url = url(backend)?;
        let admin = admin_at(backend, &url).await;
        let server = parse_url(&url, 0).expect("the URL parsed a moment ago");
        let statements: Vec<String> = match backend {
            Backend::Postgres => vec![format!("CREATE DATABASE {name}")],
            Backend::Mysql | Backend::MariaDb => vec![format!(
                "CREATE DATABASE {name} CHARACTER SET utf8mb4 COLLATE utf8mb4_bin"
            )],
            Backend::Oracle => vec![
                format!(
                    "CREATE USER {name} IDENTIFIED BY \"{ORACLE_TEST_PASSWORD}\" \
                     QUOTA UNLIMITED ON USERS"
                ),
                format!("GRANT CREATE SESSION, CREATE TABLE, CREATE VIEW TO {name}"),
            ],
            Backend::Sqlite => unreachable!(),
        };
        for statement in &statements {
            q(&admin, statement).await;
        }
        let config = match backend {
            Backend::Oracle => {
                let user = ServerUrl {
                    user: name.clone(),
                    password: ORACLE_TEST_PASSWORD.into(),
                    ..server.clone()
                };
                with_target(backend, &user, &server.database)
            }
            _ => with_target(backend, &server, &name),
        };
        Some(TestDb {
            backend,
            name,
            config,
            admin_url: Some(url),
            dir: None,
            cleaned: false,
        })
    }

    /// A new connection to this database.
    pub async fn connect(&self) -> Driver {
        let mut driver = Driver::new();
        driver
            .connect(self.config.clone())
            .await
            .unwrap_or_else(|e| panic!("connecting to {:?} failed: {e:#}", self.config));
        driver
    }

    /// The SQLite file, for the SQLite-only tests.
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        match &self.config {
            ConnectionConfig::Sqlite(c) => Some(c.path.clone()),
            _ => None,
        }
    }

    /// Drops the database: `DROP DATABASE … WITH (FORCE)`, or
    /// `DROP USER … CASCADE` for Oracle.
    pub async fn cleanup(mut self) {
        if let Some(url) = &self.admin_url
            && let Err(e) = drop_database(self.backend, &self.name, url).await
        {
            panic!("dropping {} on {} failed: {e:#}", self.name, self.backend);
        }
        self.cleaned = true;
    }
}

impl Drop for TestDb {
    /// The test ended without [`TestDb::cleanup`], most likely by
    /// panicking. Async code cannot run here, so a thread of its own with
    /// its own runtime drops the database.
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        let Some(url) = self.admin_url.clone() else {
            return;
        };
        let (backend, name) = (self.backend, self.name.clone());
        let outcome = std::thread::spawn(move || -> Result<()> {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(drop_database(backend, &name, &url))
        })
        .join();
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("could not drop {} on {backend}: {e:#}", self.name),
            Err(_) => eprintln!("dropping {} on {backend} panicked", self.name),
        }
    }
}

async fn drop_database(backend: Backend, name: &str, url: &str) -> Result<()> {
    let mut admin = Driver::new();
    admin.connect(config_from_url(backend, url)).await?;
    match backend {
        Backend::Postgres => {
            admin
                .query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
                .await?;
        }
        Backend::Mysql | Backend::MariaDb => {
            admin
                .query(&format!("DROP DATABASE IF EXISTS {name}"))
                .await?;
        }
        Backend::Oracle => {
            // A user with a session cannot be dropped (ORA-01940), and a
            // session that just closed can linger for a moment.
            let mut last = None;
            for _ in 0..20 {
                let sessions = admin
                    .query(&format!(
                        "SELECT sid, serial# FROM v$session WHERE username = UPPER('{name}')"
                    ))
                    .await?;
                for row in &sessions.rows {
                    if let [Some(sid), Some(serial)] = row.as_slice() {
                        let _ = admin
                            .query(&format!(
                                "ALTER SYSTEM KILL SESSION '{sid},{serial}' IMMEDIATE"
                            ))
                            .await;
                    }
                }
                match admin.query(&format!("DROP USER {name} CASCADE")).await {
                    Ok(_) => return Ok(()),
                    // ORA-01918: the user does not exist, so nothing to do.
                    Err(e) if format!("{e:#}").contains("ORA-01918") => return Ok(()),
                    Err(e) => last = Some(e),
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            return Err(last.unwrap_or_else(|| anyhow!("DROP USER {name} kept failing")));
        }
        Backend::Sqlite => {}
    }
    Ok(())
}

/// The suite's databases (or Oracle users) on `backend`'s server that are
/// more than an hour old: ones a crashed run left behind. `None` when the
/// backend is not configured.
pub async fn leftover_test_databases(backend: Backend) -> Option<Vec<String>> {
    let admin = admin(backend).await?;
    let sql = match backend {
        Backend::Postgres => "SELECT datname FROM pg_database ORDER BY datname",
        Backend::Mysql | Backend::MariaDb => {
            "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name"
        }
        Backend::Oracle => "SELECT username FROM all_users ORDER BY username",
        Backend::Sqlite => return Some(Vec::new()),
    };
    let hour_ago = SystemTime::now() - Duration::from_secs(60 * 60);
    let names = q(&admin, sql)
        .await
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next().flatten());
    Some(
        names
            .filter(|name| name.to_ascii_lowercase().starts_with(PREFIX))
            // A name that does not decode is not one of ours, but it looks
            // like one; report it too.
            .filter(|name| created_at(name).is_none_or(|at| at < hour_ago))
            .collect(),
    )
}

/// `db_or_skip!(backend)`: a seeded [`TestDb`], or skip (or fail on CI)
/// when the backend is not configured.
#[macro_export]
macro_rules! db_or_skip {
    ($backend:expr) => {
        match $crate::common::TestDb::seeded($backend).await {
            Some(db) => db,
            None => {
                $crate::common::skip_or_fail($backend);
                return;
            }
        }
    };
}

/// `fresh_or_skip!(backend)`: like [`db_or_skip!`], with an empty database.
#[macro_export]
macro_rules! fresh_or_skip {
    ($backend:expr) => {
        match $crate::common::TestDb::fresh($backend).await {
            Some(db) => db,
            None => {
                $crate::common::skip_or_fail($backend);
                return;
            }
        }
    };
}

/// The cells of `result`, column `col`, as `Option<&str>`s.
pub fn column(result: &QueryResult, col: usize) -> Vec<Option<&str>> {
    result
        .rows
        .iter()
        .map(|row| row.get(col).and_then(|c| c.as_deref()))
        .collect()
}

/// The column names, lower-cased: Oracle upper-cases unquoted names.
pub fn lower_names(result: &QueryResult) -> Vec<String> {
    result
        .columns
        .iter()
        .map(|c| c.name.to_lowercase())
        .collect()
}

/// `names`, lower-cased.
pub fn lower(names: &[String]) -> Vec<String> {
    names.iter().map(|n| n.to_lowercase()).collect()
}

/// Asserts `result` is what a statement without rows gives.
pub fn assert_no_rows(result: &QueryResult, sql: &str) {
    assert!(
        result.columns.is_empty() && result.rows.is_empty(),
        "{sql} gave {result:?}"
    );
}
