//! The connection file: metadata only, never a secret.
//!
//! Secrets live in the OS keychain (see [`crate::secrets`]), in an
//! environment variable, or nowhere at all. Nothing in this module writes a
//! password, and [`StoredConnection`] has no field to hold one.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Context, bail};
use qry_core::{
    ConnectionConfig, SslMode, mariadb::MariadbConfig, mysql::MySqlConfig, oracle::OracleConfig,
    postgres::PostgresConfig, sqlite::SqliteConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::get_data_dir;

/// The format this build writes. A file numbered higher is refused rather
/// than parsed hopefully.
pub const VERSION: u64 = 1;

const FILE: &str = "connections.json";
const TEMP: &str = "connections.json.tmp";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    Postgres,
    Mariadb,
    Mysql,
    Oracle,
    Sqlite,
}

impl Driver {
    pub fn default_port(self) -> u16 {
        match self {
            Driver::Postgres => 5432,
            Driver::Mariadb | Driver::Mysql => 3306,
            Driver::Oracle => 1521,
            Driver::Sqlite => 0,
        }
    }

    /// Whether a connection of this kind needs a password at all.
    pub fn needs_password(self) -> bool {
        !matches!(self, Driver::Sqlite)
    }
}

/// Where a connection's password comes from. Every record says so; qry never
/// assumes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Secret {
    /// The OS keychain, under the connection's id.
    Keyring,
    /// An environment variable, read at connect time.
    Env { var: String },
    /// Asked for on every connect.
    Prompt,
    /// Trust auth, socket auth, or SQLite.
    None,
}

/// One connection, as it is written to disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredConnection {
    /// Minted once and never changed: the keyring account and the tree's
    /// identity, so renaming cannot orphan a secret.
    pub id: String,
    pub name: String,
    pub driver: Driver,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// Oracle: the service name or SID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// SQLite: the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// An SSL mode. Records written before Oracle spoke TLS may hold a
    /// wallet directory here instead; see [`StoredConnection::oracle_tls`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<String>,
    /// Oracle: the directory holding `ewallet.pem`, used over TCPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet: Option<String>,
    /// SQLite: `ro`, `rw` or `rwc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Used instead of the fields above when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsn: Option<String>,
    pub secret: Secret,
    /// Anything a newer qry wrote, kept so saving does not strip it.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    /// Shown in the tree but never written: the scratch database qry opens
    /// at startup, and anything else that lasts only for the session.
    #[serde(skip)]
    pub ephemeral: bool,
}

impl StoredConnection {
    /// A new record with a fresh id and nothing filled in.
    pub fn new(name: impl Into<String>, driver: Driver) -> Self {
        Self {
            id: mint_id(),
            name: name.into(),
            driver,
            host: None,
            port: None,
            user: None,
            database: None,
            service: None,
            path: None,
            schema: None,
            tls: None,
            wallet: None,
            mode: None,
            dsn: None,
            secret: if driver.needs_password() {
                Secret::Prompt
            } else {
                Secret::None
            },
            extra: Map::new(),
            ephemeral: false,
        }
    }

    /// Whether both open the same database as the same user, whatever they
    /// are called. How an import tells a connection it already has.
    pub fn same_target(&self, other: &Self) -> bool {
        let port = |c: &Self| c.port.unwrap_or(c.driver.default_port());
        self.driver == other.driver
            && self.host == other.host
            && port(self) == port(other)
            && self.user == other.user
            && self.database == other.database
            && self.service == other.service
            && self.path == other.path
    }

    /// What the tree shows.
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    /// Builds what the driver needs, given the password from wherever this
    /// record says it lives. The password is never kept here.
    pub fn to_config(&self, password: &str) -> color_eyre::Result<ConnectionConfig> {
        if self.dsn.is_some() {
            bail!("DSN connections are not supported yet");
        }
        let required = |field: &str, value: &Option<String>| -> color_eyre::Result<String> {
            match value {
                Some(value) if !value.is_empty() => Ok(value.clone()),
                _ => bail!("{} needs a {field}", self.title()),
            }
        };

        if self.driver == Driver::Sqlite {
            let path = required("path", &self.path)?;
            return Ok(ConnectionConfig::Sqlite(SqliteConfig::new(
                path,
                self.mode.as_deref() == Some("ro"),
            )));
        }

        let host = required("host", &self.host)?;
        let port = self.port.unwrap_or(self.driver.default_port());
        let user = required("user", &self.user)?;
        let password = password.to_string();

        if self.driver == Driver::Oracle {
            let (ssl_mode, wallet) = self.oracle_tls()?;
            return Ok(ConnectionConfig::Oracle(OracleConfig {
                host,
                port,
                user,
                password,
                service: required("service", &self.service)?,
                schema: self.schema.clone(),
                ssl_mode,
                wallet,
            }));
        }
        let ssl_mode = self.ssl_mode()?;

        Ok(match self.driver {
            Driver::Postgres => ConnectionConfig::Postgres(PostgresConfig {
                host,
                port,
                user,
                password,
                database: required("database", &self.database)?,
                schema: self.schema.clone(),
                ssl_mode,
            }),
            Driver::Mysql => ConnectionConfig::Mysql(MySqlConfig {
                host,
                port,
                user,
                password,
                database: self.database.clone().unwrap_or_default(),
                ssl_mode,
            }),
            Driver::Mariadb => ConnectionConfig::MariaDb(MariadbConfig {
                host,
                port,
                user,
                password,
                database: self.database.clone().unwrap_or_default(),
                ssl_mode,
            }),
            Driver::Oracle | Driver::Sqlite => unreachable!("handled above"),
        })
    }

    /// Oracle's SSL mode and wallet. Before qry spoke TLS to Oracle, `tls`
    /// held a wallet directory rather than a mode; such a record still means
    /// "TCPS with this wallet", and saving it again writes the new shape.
    pub fn oracle_tls(&self) -> color_eyre::Result<(SslMode, Option<String>)> {
        match &self.tls {
            Some(tls) if !SslMode::ALL.iter().any(|mode| mode.to_string() == *tls) => {
                Ok((SslMode::VerifyFull, Some(tls.clone())))
            }
            _ => Ok((self.ssl_mode()?, self.wallet.clone())),
        }
    }

    fn ssl_mode(&self) -> color_eyre::Result<SslMode> {
        let Some(tls) = &self.tls else {
            return Ok(SslMode::default());
        };
        SslMode::ALL
            .into_iter()
            .find(|mode| mode.to_string() == *tls)
            .ok_or_else(|| color_eyre::eyre::eyre!("`{tls}` is not an SSL mode"))
    }
}

/// The file itself: an object rather than a bare array, so the shape can
/// change later.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectionFile {
    version: u64,
    #[serde(default)]
    connections: Vec<StoredConnection>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Reads the connections. A missing file is not an error: it means a first
/// run. A malformed one is, and the bad file is left untouched.
pub fn load() -> color_eyre::Result<Vec<StoredConnection>> {
    load_from(&get_data_dir())
}

pub fn load_from(dir: &Path) -> color_eyre::Result<Vec<StoredConnection>> {
    let file = dir.join(FILE);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).wrap_err_with(|| format!("cannot read {}", file.display())),
    };
    parse(&text, &file)
}

/// The connections in the text of a connection file, which `file` names in
/// errors. Also how a copy of the file is imported as a backup.
pub fn parse(text: &str, file: &Path) -> color_eyre::Result<Vec<StoredConnection>> {
    let parsed: ConnectionFile = serde_json::from_str(text).wrap_err_with(|| {
        format!(
            "{} is not a connection file qry understands",
            file.display()
        )
    })?;
    if parsed.version > VERSION {
        bail!(
            "{} was written by a newer qry (version {}, this build understands {VERSION})",
            file.display(),
            parsed.version
        );
    }
    Ok(parsed.connections)
}

/// Writes the connections whole, and atomically: a crash mid-save leaves the
/// previous file intact.
pub fn save(connections: &[StoredConnection]) -> color_eyre::Result<()> {
    save_to(&get_data_dir(), connections)
}

pub fn save_to(dir: &Path, connections: &[StoredConnection]) -> color_eyre::Result<()> {
    fs::create_dir_all(dir).wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    restrict(dir, 0o700)?;

    // Anything the file carried that this build does not know about is kept.
    let mut file = load_file(dir).unwrap_or_default();
    file.version = VERSION;
    file.connections = connections.to_vec();

    write_atomically(&file, &dir.join(TEMP), &dir.join(FILE))
}

/// Writes a backup of the connections to `path`, in the format `load` and
/// the importer read, and answers with where it went: `.json` is added to a
/// path that has no extension. An existing file is replaced only if it is
/// already a connection file, so a mistyped path cannot clobber anything
/// else. Holds no password, since a record has nowhere to keep one.
pub fn export(path: &Path, connections: &[StoredConnection]) -> color_eyre::Result<PathBuf> {
    let path = match path.extension() {
        Some(_) => path.to_path_buf(),
        None => path.with_extension("json"),
    };
    if path.is_dir() {
        bail!("{} is a directory", path.display());
    }
    if let Ok(existing) = fs::read_to_string(&path)
        && parse(&existing, &path).is_err()
    {
        bail!(
            "{} already exists and is not a qry backup, so it was left alone",
            path.display()
        );
    }

    let file = ConnectionFile {
        version: VERSION,
        connections: connections.to_vec(),
        extra: Map::new(),
    };
    let mut temp = path.clone().into_os_string();
    temp.push(".tmp");
    write_atomically(&file, Path::new(&temp), &path)?;
    Ok(path)
}

/// Writes `file` to `temp`, then renames it over `target`: a crash midway
/// leaves the previous file intact. Readable by this user only.
fn write_atomically(file: &ConnectionFile, temp: &Path, target: &Path) -> color_eyre::Result<()> {
    let text = serde_json::to_string_pretty(file)? + "\n";
    {
        let mut handle =
            File::create(temp).wrap_err_with(|| format!("cannot write {}", temp.display()))?;
        handle.write_all(text.as_bytes())?;
        // On disk before the rename, so the rename cannot publish a partial file.
        handle.sync_all()?;
    }
    restrict(temp, 0o600)?;

    fs::rename(temp, target).wrap_err_with(|| format!("cannot replace {}", target.display()))?;
    Ok(())
}

fn load_file(dir: &Path) -> color_eyre::Result<ConnectionFile> {
    let text = fs::read_to_string(dir.join(FILE))?;
    Ok(serde_json::from_str(&text)?)
}

/// Keeps a username, host and database name to this user. They are not
/// secrets, but they are nobody else's business.
#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> color_eyre::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .wrap_err_with(|| format!("cannot set the permissions of {}", path.display()))
}

/// On Windows the per-user app data directory's inherited ACL is enough.
#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> color_eyre::Result<()> {
    Ok(())
}

/// Eight hex characters, unique within a run and unlikely to repeat between
/// runs. Enough for a keyring account name.
fn mint_id() -> String {
    use std::{
        hash::{DefaultHasher, Hash, Hasher},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    /// A directory of its own per test, removed when the guard is dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "qry-connections-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn postgres() -> StoredConnection {
        StoredConnection {
            host: Some("db.local".into()),
            user: Some("alice".into()),
            database: Some("app".into()),
            secret: Secret::Keyring,
            ..StoredConnection::new("prod", Driver::Postgres)
        }
    }

    #[test]
    fn ids_are_unique() {
        let ids: Vec<String> = (0..100).map(|_| mint_id()).collect();
        assert!(ids.iter().all(|id| id.len() == 8));
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn a_missing_file_is_an_empty_tree() {
        let dir = TempDir::new();
        assert!(load_from(&dir.0).unwrap().is_empty());
    }

    #[test]
    fn what_is_saved_comes_back() {
        let dir = TempDir::new();
        let connections = vec![postgres(), StoredConnection::new("scratch", Driver::Sqlite)];
        save_to(&dir.0, &connections).unwrap();

        assert_eq!(load_from(&dir.0).unwrap(), connections);
        // The temporary file does not outlive the save.
        assert!(!dir.0.join(TEMP).exists());
    }

    #[test]
    fn the_file_holds_no_password_and_omits_fields_that_do_not_apply() {
        let dir = TempDir::new();
        let mut connection = postgres();
        connection.port = Some(5432);
        save_to(&dir.0, &[connection]).unwrap();

        let text = fs::read_to_string(dir.0.join(FILE)).unwrap();
        assert!(!text.contains("password"), "{text}");
        assert!(text.contains("\"driver\": \"postgres\""), "{text}");
        assert!(text.contains("\"mode\": \"keyring\""), "{text}");
        // SQLite fields have no place on a Postgres record.
        assert!(!text.contains("\"path\""), "{text}");
        assert!(!text.contains("\"service\""), "{text}");
    }

    #[test]
    fn fields_from_a_newer_qry_survive_a_save() {
        let dir = TempDir::new();
        fs::write(
            dir.0.join(FILE),
            r#"{
              "version": 1,
              "future_setting": true,
              "connections": [
                {"id": "c7f3a1e2", "name": "prod", "driver": "postgres", "host": "h",
                 "user": "u", "database": "d", "secret": {"mode": "prompt"},
                 "colour": "red"}
              ]
            }"#,
        )
        .unwrap();

        let mut loaded = load_from(&dir.0).unwrap();
        assert_eq!(
            loaded[0].extra.get("colour").and_then(Value::as_str),
            Some("red")
        );
        loaded[0].name = "renamed".into();
        save_to(&dir.0, &loaded).unwrap();

        let text = fs::read_to_string(dir.0.join(FILE)).unwrap();
        assert!(text.contains("future_setting"), "{text}");
        assert!(text.contains("\"colour\": \"red\""), "{text}");
        assert!(text.contains("renamed"), "{text}");
    }

    #[test]
    fn a_newer_version_is_refused_and_the_file_left_alone() {
        let dir = TempDir::new();
        let original = r#"{"version": 99, "connections": []}"#;
        fs::write(dir.0.join(FILE), original).unwrap();

        let err = load_from(&dir.0).unwrap_err().to_string();
        assert!(err.contains("newer qry"), "{err}");
        assert_eq!(fs::read_to_string(dir.0.join(FILE)).unwrap(), original);
    }

    #[test]
    fn a_malformed_file_is_an_error_and_is_left_alone() {
        let dir = TempDir::new();
        fs::write(dir.0.join(FILE), "{ not json").unwrap();

        let err = load_from(&dir.0).unwrap_err().to_string();
        assert!(err.contains("connection file"), "{err}");
        assert_eq!(fs::read_to_string(dir.0.join(FILE)).unwrap(), "{ not json");
    }

    #[test]
    fn a_record_builds_what_the_driver_needs() {
        let ConnectionConfig::Postgres(config) = postgres().to_config("hunter2").unwrap() else {
            panic!("expected Postgres");
        };
        assert_eq!((config.port, config.password.as_str()), (5432, "hunter2"));
        assert_eq!(config.ssl_mode, SslMode::Prefer);

        let mut sqlite = StoredConnection::new("scratch", Driver::Sqlite);
        sqlite.path = Some("app.db".into());
        sqlite.mode = Some("ro".into());
        let ConnectionConfig::Sqlite(config) = sqlite.to_config("").unwrap() else {
            panic!("expected Sqlite");
        };
        assert!(config.read_only);
    }

    fn oracle() -> StoredConnection {
        let mut connection = StoredConnection::new("erp", Driver::Oracle);
        connection.host = Some("db.local".into());
        connection.user = Some("alice".into());
        connection.service = Some("ORCLPDB1".into());
        connection
    }

    fn oracle_config(connection: &StoredConnection) -> OracleConfig {
        let ConnectionConfig::Oracle(config) = connection.to_config("x").unwrap() else {
            panic!("expected Oracle");
        };
        config
    }

    #[test]
    fn an_oracle_record_carries_its_ssl_mode_and_wallet() {
        let config = oracle_config(&oracle());
        assert_eq!(
            (config.port, config.ssl_mode, config.wallet),
            (1521, SslMode::Prefer, None)
        );

        let mut connection = oracle();
        connection.tls = Some("verify-full".into());
        connection.wallet = Some("/etc/oracle/wallet".into());
        let config = oracle_config(&connection);
        assert_eq!(config.ssl_mode, SslMode::VerifyFull);
        assert_eq!(config.wallet.as_deref(), Some("/etc/oracle/wallet"));
    }

    #[test]
    fn an_old_oracle_record_with_a_wallet_in_tls_means_tcps() {
        let text = r#"{"id":"c1","name":"erp","driver":"oracle","host":"db.local",
            "user":"alice","service":"ORCLPDB1","tls":"/etc/oracle/wallet",
            "secret":{"mode":"prompt"}}"#;
        let connection: StoredConnection = serde_json::from_str(text).unwrap();
        let config = oracle_config(&connection);
        assert_eq!(config.ssl_mode, SslMode::VerifyFull);
        assert_eq!(config.wallet.as_deref(), Some("/etc/oracle/wallet"));
    }

    #[test]
    fn an_export_is_a_backup_the_importer_reads_back() {
        let dir = TempDir::new();
        let mut kept = postgres();
        kept.secret = Secret::Keyring;
        let list = vec![kept, oracle()];

        // No extension: `.json` is added.
        let written = export(&dir.0.join("backup"), &list).unwrap();
        assert_eq!(written, dir.0.join("backup.json"));
        let text = fs::read_to_string(&written).unwrap();
        assert!(
            !text.contains("hunter2"),
            "a record has nowhere to keep a password"
        );
        assert!(
            !dir.0.join("backup.json.tmp").exists(),
            "the temp file was renamed away"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&written).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let imported =
            crate::import::from_file(written.to_str().unwrap(), crate::import::Format::Auto)
                .unwrap();
        assert_eq!(imported.format, crate::import::Format::Qry);
        assert_eq!(
            imported.connections, list,
            "it comes back exactly as written"
        );

        // Writing it again replaces the old backup.
        export(&written, &list[..1]).unwrap();
        assert_eq!(
            parse(&fs::read_to_string(&written).unwrap(), &written)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn an_export_never_overwrites_a_file_that_is_not_a_backup() {
        let dir = TempDir::new();
        let other = dir.0.join("notes.txt");
        fs::write(&other, "my notes").unwrap();
        let err = export(&other, &[postgres()]).unwrap_err().to_string();
        assert!(err.contains("not a qry backup"), "{err}");
        assert_eq!(fs::read_to_string(&other).unwrap(), "my notes");

        let err = export(&dir.0.join("no/such/dir/b.json"), &[postgres()]).unwrap_err();
        assert!(err.to_string().contains("cannot write"), "{err}");
    }

    #[test]
    fn a_record_missing_a_field_says_which() {
        let mut connection = postgres();
        connection.host = None;
        let err = connection.to_config("x").unwrap_err().to_string();
        assert!(err.contains("needs a host"), "{err}");

        connection = postgres();
        connection.tls = Some("nonsense".into());
        assert!(
            connection
                .to_config("x")
                .unwrap_err()
                .to_string()
                .contains("not an SSL mode")
        );

        connection = postgres();
        connection.dsn = Some("postgres://db/app".into());
        assert!(
            connection
                .to_config("x")
                .unwrap_err()
                .to_string()
                .contains("DSN")
        );
    }
}
