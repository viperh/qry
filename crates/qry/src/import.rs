//! Reads connections out of files other than qry's own: a backup of the
//! connection file, a SQL Developer export, or DBeaver's `data-sources.json`.
//!
//! Nothing here imports a password. SQL Developer encrypts them with a key of
//! its own, and DBeaver keeps them in a separate encrypted file, so every
//! imported connection asks for its password on the first connect, which
//! then offers to keep it in the keychain like any other.

use std::fmt;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, bail};
use qry_core::SslMode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::connections::{self, Driver, StoredConnection};

/// Where a file came from. `Auto` looks at its shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[default]
    Auto,
    Qry,
    SqlDeveloper,
    DBeaver,
}

impl Format {
    /// In the order the import form offers them.
    pub const ALL: [Format; 4] = [
        Format::Auto,
        Format::Qry,
        Format::SqlDeveloper,
        Format::DBeaver,
    ];
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Auto => "Detect",
            Format::Qry => "qry backup",
            Format::SqlDeveloper => "SQL Developer",
            Format::DBeaver => "DBeaver",
        })
    }
}

/// A connection in the file that qry could not turn into one of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

/// What a file held.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Imported {
    /// The format it was read as, after detection.
    pub format: Format,
    pub connections: Vec<StoredConnection>,
    pub skipped: Vec<Skipped>,
    /// Connections that were kept but need a look, such as a SID read as a
    /// service name.
    pub warnings: Vec<String>,
}

/// Reads and converts a file. Blocking: call it from `spawn_blocking`.
pub fn from_file(path: &str, format: Format) -> color_eyre::Result<Imported> {
    let path = expand_home(path);
    let bytes = std::fs::read(&path).wrap_err_with(|| format!("cannot read {}", path.display()))?;
    if bytes.starts_with(b"PK") {
        bail!(
            "{} is a zip archive, such as a DBeaver project export; \
             extract its data-sources.json and import that",
            path.display()
        );
    }
    let text = String::from_utf8(bytes)
        .wrap_err_with(|| format!("{} is not a text file", path.display()))?;
    from_text(&text, format, &path)
}

/// Converts the text of a file, which `path` names in errors.
pub fn from_text(text: &str, format: Format, path: &Path) -> color_eyre::Result<Imported> {
    let shown = path.display();
    if text.trim_start().starts_with('<') {
        bail!(
            "{shown} is XML; qry reads the JSON that SQL Developer's \
             Export Connections writes, and DBeaver's data-sources.json"
        );
    }
    let value: Value =
        serde_json::from_str(text).wrap_err_with(|| format!("{shown} is not JSON"))?;
    let format = match format {
        Format::Auto => match detect(&value) {
            Some(format) => format,
            None => bail!(
                "{shown} is not a qry backup, a SQL Developer export or a DBeaver data-sources.json"
            ),
        },
        format => format,
    };

    let mut imported = Imported {
        format,
        connections: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    };
    match format {
        Format::Qry => imported.connections = connections::parse(text, path)?,
        Format::SqlDeveloper => sql_developer(&value, &mut imported)?,
        Format::DBeaver => dbeaver(&value, &mut imported)?,
        Format::Auto => unreachable!("detected above"),
    }
    for record in &imported.connections {
        if record.driver.needs_password() && record.user.is_none() {
            imported.warnings.push(format!(
                "{} has no user name; edit it before connecting",
                record.title()
            ));
        }
    }
    Ok(imported)
}

/// Tells the three apart by shape. Each keeps a `connections` key: qry's is
/// an array beside a `version`, SQL Developer's an array of objects with an
/// `info` block, DBeaver's an object keyed by connection id.
fn detect(value: &Value) -> Option<Format> {
    let connections = value.get("connections")?;
    match connections {
        Value::Object(_) => Some(Format::DBeaver),
        Value::Array(_) if value.get("version").is_some_and(Value::is_u64) => Some(Format::Qry),
        Value::Array(items) if items.iter().all(|item| item.get("info").is_some()) => {
            Some(Format::SqlDeveloper)
        }
        _ => None,
    }
}

/// `~/x` is the home directory's `x`, as a shell would read it.
fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(dirs) = directories::BaseDirs::new()
    {
        return dirs.home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// Where a connection points, before it becomes a record.
#[derive(Debug, Default, PartialEq, Eq)]
struct Target {
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    /// A database name, an Oracle service, or a SQLite file: the record
    /// puts it in the field its driver reads.
    database: Option<String>,
    /// Oracle: the database was given as a SID, not a service name.
    sid: bool,
    /// Oracle over TCPS.
    tls: bool,
}

impl Target {
    fn into_record(self, name: &str, driver: Driver) -> StoredConnection {
        let mut record = StoredConnection::new(name, driver);
        record.host = self.host;
        record.port = self.port.filter(|&port| port != driver.default_port());
        record.user = self.user;
        match driver {
            Driver::Sqlite => record.path = self.database,
            Driver::Oracle => record.service = self.database,
            _ => record.database = self.database,
        }
        if self.tls {
            record.tls = Some(SslMode::VerifyFull.to_string());
        }
        record
    }
}

/// A non-empty string field, trimmed.
fn text(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// A port, written as a number or, as both tools do, as a string.
fn port(object: &Map<String, Value>, key: &str) -> Result<Option<u16>, String> {
    let parsed = match object.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(None),
        Some(Value::Number(n)) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
        Some(Value::String(s)) => s.trim().parse().ok(),
        Some(_) => None,
    };
    match parsed {
        Some(port) if port != 0 => Ok(Some(port)),
        _ => Err(format!("`{}` is not a port", object[key])),
    }
}

fn flag(object: &Map<String, Value>, key: &str) -> bool {
    match object.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// SQL Developer's Export Connections, and the `connections.json` in its
/// settings directory, which has the same shape:
/// `{"connections": [{"name": …, "type": "jdbc", "info": {…}}]}`.
fn sql_developer(value: &Value, imported: &mut Imported) -> color_eyre::Result<()> {
    let Some(entries) = value.get("connections").and_then(Value::as_array) else {
        bail!("a SQL Developer export holds a `connections` array");
    };
    for entry in entries {
        let info = entry
            .get("info")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| text(&info, "ConnName"))
            .unwrap_or_else(|| "unnamed".into());
        match sql_developer_one(&info) {
            Ok((driver, target)) => {
                if target.sid {
                    imported
                        .warnings
                        .push(sid_warning(&name, target.database.as_deref()));
                }
                imported.connections.push(target.into_record(&name, driver));
            }
            Err(reason) => imported.skipped.push(Skipped { name, reason }),
        }
    }
    Ok(())
}

fn sql_developer_one(info: &Map<String, Value>) -> Result<(Driver, Target), String> {
    if flag(info, "OS_AUTHENTICATION") {
        return Err("uses OS authentication, which qry cannot do".into());
    }
    if flag(info, "KERBEROS_AUTHENTICATION") {
        return Err("uses Kerberos, which qry cannot do".into());
    }
    let kind = text(info, "RaptorConnectionType").unwrap_or_else(|| "Oracle".into());
    let user = text(info, "user");

    // Other databases go through a third-party JDBC driver and a URL.
    if !kind.eq_ignore_ascii_case("Oracle") {
        let url = text(info, "customUrl")
            .ok_or_else(|| format!("is a {kind} connection with no URL to read"))?;
        let (driver, mut target) = jdbc(&url)?;
        target.user = target.user.or(user);
        return Ok((driver, target));
    }

    match text(info, "OracleConnectionType")
        .as_deref()
        .unwrap_or("BASIC")
    {
        "BASIC" => {
            let (database, sid) = match (text(info, "serviceName"), text(info, "sid")) {
                (Some(service), _) => (service, false),
                (None, Some(sid)) => (sid, true),
                (None, None) => return Err("names neither a service nor a SID".into()),
            };
            Ok((
                Driver::Oracle,
                Target {
                    host: text(info, "hostname").or_else(|| text(info, "host")),
                    port: port(info, "port")?,
                    user,
                    database: Some(database),
                    sid,
                    tls: false,
                },
            ))
        }
        "ADVANCED" => {
            let url = text(info, "customUrl").ok_or("is an advanced connection with no URL")?;
            let (driver, mut target) = jdbc(&url)?;
            target.user = target.user.or(user);
            Ok((driver, target))
        }
        "TNS" => Err(format!(
            "uses the TNS alias `{}`; qry needs the host, port and service",
            text(info, "TNS_ALIAS")
                .or_else(|| text(info, "customUrl"))
                .unwrap_or_default()
        )),
        other => Err(format!("is a {other} connection, which qry cannot open")),
    }
}

/// DBeaver's `data-sources.json`: `{"connections": {"<id>": {"provider":
/// …, "driver": …, "name": …, "configuration": {…}}}}`. User names live in
/// the encrypted credentials file in newer versions; older ones put them in
/// the configuration, and those are read.
fn dbeaver(value: &Value, imported: &mut Imported) -> color_eyre::Result<()> {
    let Some(entries) = value.get("connections").and_then(Value::as_object) else {
        bail!("a DBeaver data-sources.json holds a `connections` object");
    };
    for (id, entry) in entries {
        let empty = Map::new();
        let entry = entry.as_object().unwrap_or(&empty);
        let name = text(entry, "name").unwrap_or_else(|| id.clone());
        match dbeaver_one(entry) {
            Ok((driver, target)) => {
                if target.sid {
                    imported
                        .warnings
                        .push(sid_warning(&name, target.database.as_deref()));
                }
                let mut record = target.into_record(&name, driver);
                if driver == Driver::Sqlite && flag(entry, "read-only") {
                    record.mode = Some("ro".into());
                }
                imported.connections.push(record);
            }
            Err(reason) => imported.skipped.push(Skipped { name, reason }),
        }
    }
    Ok(())
}

fn dbeaver_one(entry: &Map<String, Value>) -> Result<(Driver, Target), String> {
    let provider = text(entry, "provider").unwrap_or_default().to_lowercase();
    let driver_id = text(entry, "driver").unwrap_or_default().to_lowercase();
    let driver = match provider.as_str() {
        "postgresql" => Driver::Postgres,
        "mysql" | "mariadb" if driver_id.contains("maria") || provider == "mariadb" => {
            Driver::Mariadb
        }
        "mysql" => Driver::Mysql,
        "oracle" => Driver::Oracle,
        "sqlite" => Driver::Sqlite,
        "generic" if driver_id.contains("sqlite") => Driver::Sqlite,
        "" => return Err("names no provider".into()),
        _ => {
            return Err(format!(
                "uses the DBeaver provider `{provider}`, which qry cannot open"
            ));
        }
    };

    let empty = Map::new();
    let config = entry
        .get("configuration")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let props = config
        .get("provider-properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let user = text(config, "user");

    if driver == Driver::Oracle
        && let Some(kind) = text(props, "@dbeaver-connection-type@")
        && kind.eq_ignore_ascii_case("TNS")
    {
        return Err("uses a TNS alias; qry needs the host, port and service".into());
    }

    // A connection set up as a URL keeps everything in it.
    let by_url = text(config, "configurationType").is_some_and(|t| t.eq_ignore_ascii_case("URL"));
    let host = text(config, "host");
    if by_url || (host.is_none() && driver != Driver::Sqlite) {
        let url = text(config, "url").ok_or("has neither a host nor a URL")?;
        let (from_url, mut target) = jdbc(&url)?;
        if from_url != driver {
            return Err(format!(
                "is a {provider} connection with a URL for another database"
            ));
        }
        target.user = target.user.or(user);
        return Ok((driver, target));
    }

    if driver == Driver::Sqlite {
        let database = match text(config, "database") {
            Some(path) => path,
            None => match jdbc(&text(config, "url").ok_or("names no database file")?)? {
                (Driver::Sqlite, target) => target.database.ok_or("names no database file")?,
                _ => return Err("is a SQLite connection with a URL for another database".into()),
            },
        };
        return Ok((
            driver,
            Target {
                database: Some(database),
                ..Target::default()
            },
        ));
    }

    let sid = driver == Driver::Oracle
        && text(props, "@dbeaver-sid-service@")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("SID"));
    Ok((
        driver,
        Target {
            host,
            port: port(config, "port")?,
            user,
            database: text(config, "database"),
            sid,
            tls: false,
        },
    ))
}

fn sid_warning(name: &str, sid: Option<&str>) -> String {
    format!(
        "{name}: the SID {} was read as a service name; \
         most databases register a service by the same name",
        sid.unwrap_or_default()
    )
}

/// The JDBC URLs both tools fall back on, for the databases qry speaks.
fn jdbc(url: &str) -> Result<(Driver, Target), String> {
    let unsupported = || format!("has the URL `{url}`, which qry cannot read");
    let rest = url
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("jdbc:"))
        .map(|_| &url[5..])
        .ok_or_else(unsupported)?;
    let (scheme, rest) = rest.split_once(':').ok_or_else(unsupported)?;

    match scheme.to_lowercase().as_str() {
        "postgresql" => Ok((Driver::Postgres, server_url(rest).ok_or_else(unsupported)?)),
        "mysql" => Ok((Driver::Mysql, server_url(rest).ok_or_else(unsupported)?)),
        "mariadb" => Ok((Driver::Mariadb, server_url(rest).ok_or_else(unsupported)?)),
        "sqlite" => {
            let path = rest.split('?').next().unwrap_or_default();
            let path = path.strip_prefix("file:").unwrap_or(path);
            if path.is_empty() {
                return Err(unsupported());
            }
            Ok((
                Driver::Sqlite,
                Target {
                    database: Some(path.into()),
                    ..Target::default()
                },
            ))
        }
        "oracle" => oracle_url(rest).map(|target| (Driver::Oracle, target)),
        _ => Err(unsupported()),
    }
}

/// `//host[:port][/database][?…]`, the shape PostgreSQL, MySQL and MariaDB
/// URLs share.
fn server_url(rest: &str) -> Option<Target> {
    let rest = rest.strip_prefix("//")?;
    let rest = rest.split('?').next().unwrap_or_default();
    let (authority, database) = match rest.split_once('/') {
        Some((authority, database)) => (authority, Some(database)),
        None => (rest, None),
    };
    // Several hosts, for failover: qry opens one.
    let authority = authority.split(',').next().unwrap_or_default();
    let (host, port) = host_port(authority)?;
    Some(Target {
        host: Some(host),
        port,
        database: database.filter(|d| !d.is_empty()).map(str::to_string),
        ..Target::default()
    })
}

/// `host`, `host:port`, `[::1]` or `[::1]:port`.
fn host_port(authority: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        (host, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let port = match port {
        Some(port) => Some(port.parse::<u16>().ok().filter(|&p| p != 0)?),
        None => None,
    };
    Some((host.to_string(), port))
}

/// What follows `jdbc:oracle:`: `thin:[user[/password]]@` then
/// `[//]host:port/service`, `host:port:SID`, or `tcps://host:port/service`.
/// A full descriptor or a TNS alias names no host qry could use.
fn oracle_url(rest: &str) -> Result<Target, String> {
    let rest = rest
        .strip_prefix("thin:")
        .ok_or_else(|| "uses the Oracle OCI driver; qry needs a thin URL".to_string())?;
    let (credentials, address) = rest
        .split_once('@')
        .ok_or_else(|| format!("has the Oracle URL `{rest}`, which qry cannot read"))?;
    // A password written into the URL is left behind on purpose.
    let user = credentials
        .split('/')
        .next()
        .filter(|u| !u.is_empty())
        .map(str::to_string);

    if address.starts_with('(') {
        return Err("uses a full connect descriptor; qry needs the host, port and service".into());
    }
    let (tls, address) = match address.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("tcps://") => (true, &address[7..]),
        _ => (false, address.strip_prefix("//").unwrap_or(address)),
    };
    let address = address.split('?').next().unwrap_or_default();

    let (authority, database, sid) = match address.split_once('/') {
        Some((authority, service)) => (authority.to_string(), service.to_string(), false),
        // host:port:SID, the oldest shape.
        None => match address.splitn(3, ':').collect::<Vec<_>>()[..] {
            [host, port, sid] => (format!("{host}:{port}"), sid.to_string(), true),
            _ => {
                return Err(format!(
                    "uses the TNS alias `{address}`; qry needs the host, port and service"
                ));
            }
        },
    };
    let (host, port) = host_port(&authority)
        .ok_or_else(|| format!("has the Oracle address `{address}`, which qry cannot read"))?;
    if database.is_empty() {
        return Err("names no Oracle service".into());
    }
    Ok(Target {
        host: Some(host),
        port,
        user,
        database: Some(database),
        sid,
        tls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::Secret;

    fn import(text: &str) -> Imported {
        from_text(text, Format::Auto, Path::new("test.json")).unwrap()
    }

    fn only(imported: &Imported) -> &StoredConnection {
        assert_eq!(imported.connections.len(), 1, "{imported:#?}");
        &imported.connections[0]
    }

    /// The shape SQL Developer's Export Connections writes. Values are
    /// strings, the password is encrypted, and it is never read.
    const SQL_DEVELOPER: &str = r#"{"connections": [
        {"name": "hr", "type": "jdbc", "info": {
            "role": "", "SavePassword": "true", "OracleConnectionType": "BASIC",
            "RaptorConnectionType": "Oracle", "serviceName": "ORCLPDB1",
            "customUrl": "jdbc:oracle:thin:@//db.local:1521/ORCLPDB1",
            "password": "v2_mBJrjLwxPLQ==", "hostname": "db.local",
            "driver": "oracle.jdbc.OracleDriver", "port": "1521",
            "subtype": "oraJDBC", "OS_AUTHENTICATION": "false",
            "ConnName": "hr", "KERBEROS_AUTHENTICATION": "false", "user": "hr"}},
        {"name": "legacy", "type": "jdbc", "info": {
            "OracleConnectionType": "BASIC", "RaptorConnectionType": "Oracle",
            "sid": "XE", "hostname": "old.local", "port": "1522", "user": "scott"}},
        {"name": "url", "type": "jdbc", "info": {
            "OracleConnectionType": "ADVANCED", "RaptorConnectionType": "Oracle",
            "customUrl": "jdbc:oracle:thin:@tcps://adb.example.com:1522/svc_high",
            "user": "admin"}},
        {"name": "tns", "type": "jdbc", "info": {
            "OracleConnectionType": "TNS", "RaptorConnectionType": "Oracle",
            "TNS_ALIAS": "PRODDB", "user": "app"}},
        {"name": "mysql", "type": "jdbc", "info": {
            "RaptorConnectionType": "MySQL",
            "customUrl": "jdbc:mysql://my.local:3307/shop", "user": "root"}},
        {"name": "kerb", "type": "jdbc", "info": {
            "OracleConnectionType": "BASIC", "KERBEROS_AUTHENTICATION": "true",
            "serviceName": "S", "hostname": "h"}}
    ]}"#;

    #[test]
    fn a_sql_developer_export_is_detected_and_read() {
        let imported = import(SQL_DEVELOPER);
        assert_eq!(imported.format, Format::SqlDeveloper);
        let names: Vec<&str> = imported
            .connections
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["hr", "legacy", "url", "mysql"]);

        let hr = &imported.connections[0];
        assert_eq!(hr.driver, Driver::Oracle);
        assert_eq!(hr.host.as_deref(), Some("db.local"));
        assert_eq!(hr.port, None, "1521 is the default, so it is left out");
        assert_eq!(hr.service.as_deref(), Some("ORCLPDB1"));
        assert_eq!(hr.user.as_deref(), Some("hr"));
        assert_eq!(hr.secret, Secret::Prompt, "no password is ever imported");

        let legacy = &imported.connections[1];
        assert_eq!(
            (legacy.port, legacy.service.as_deref()),
            (Some(1522), Some("XE"))
        );
        assert!(
            imported.warnings.iter().any(|w| w.contains("SID XE")),
            "{:?}",
            imported.warnings
        );

        let url = &imported.connections[2];
        assert_eq!(url.host.as_deref(), Some("adb.example.com"));
        assert_eq!(url.service.as_deref(), Some("svc_high"));
        assert_eq!(url.tls.as_deref(), Some("verify-full"), "tcps:// means TLS");

        let mysql = &imported.connections[3];
        assert_eq!(mysql.driver, Driver::Mysql);
        assert_eq!(
            (mysql.port, mysql.database.as_deref()),
            (Some(3307), Some("shop"))
        );
        assert_eq!(mysql.user.as_deref(), Some("root"));

        let skipped: Vec<(&str, &str)> = imported
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason.as_str()))
            .collect();
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        assert!(
            skipped[0].0 == "tns" && skipped[0].1.contains("PRODDB"),
            "{skipped:?}"
        );
        assert!(
            skipped[1].0 == "kerb" && skipped[1].1.contains("Kerberos"),
            "{skipped:?}"
        );

        let written = serde_json::to_string(&imported.connections).unwrap();
        assert!(
            !written.contains("v2_mBJ"),
            "the encrypted password stays behind"
        );
    }

    /// The shape of DBeaver's `data-sources.json`, from its reference docs.
    const DBEAVER: &str = r#"{
        "folders": {},
        "connections": {
            "postgres-jdbc-19a9ccb2c0e-201a227bc74f6a48": {
                "provider": "postgresql", "driver": "postgres-jdbc",
                "name": "PostgreSQL - prod", "save-password": true,
                "read-only": false, "folder": "prod",
                "configuration": {
                    "host": "db.example.com", "port": "5432", "database": "appdb",
                    "url": "jdbc:postgresql://db.example.com:5432/appdb",
                    "configurationType": "MANUAL", "type": "dev", "auth-model": "native"
                }
            },
            "mariaDB-1": {
                "provider": "mysql", "driver": "mariaDB", "name": "maria",
                "configuration": {"host": "m.local", "port": "3306", "database": "shop",
                                  "user": "alice"}
            },
            "oracle-1": {
                "provider": "oracle", "driver": "oracle_thin", "name": "ora",
                "configuration": {"host": "o.local", "port": "1521", "database": "XE",
                    "provider-properties": {"@dbeaver-sid-service@": "SID"}}
            },
            "sqlite-1": {
                "provider": "generic", "driver": "sqlite_jdbc", "name": "local",
                "read-only": true,
                "configuration": {"url": "jdbc:sqlite:/home/me/app.db",
                                  "configurationType": "URL"}
            },
            "url-1": {
                "provider": "postgresql", "driver": "postgres-jdbc", "name": "by url",
                "configuration": {"url": "jdbc:postgresql://[::1]:6543/reports?ssl=true",
                                  "configurationType": "URL"}
            },
            "mongo-1": {"provider": "mongodb", "driver": "mongo", "name": "docs",
                        "configuration": {"host": "x"}}
        }
    }"#;

    #[test]
    fn a_dbeaver_data_sources_file_is_detected_and_read() {
        let imported = import(DBEAVER);
        assert_eq!(imported.format, Format::DBeaver);
        let find = |name: &str| {
            imported
                .connections
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| {
                    panic!("{name} missing: {imported:#?}");
                })
        };

        let prod = find("PostgreSQL - prod");
        assert_eq!(prod.driver, Driver::Postgres);
        assert_eq!(prod.host.as_deref(), Some("db.example.com"));
        assert_eq!((prod.port, prod.database.as_deref()), (None, Some("appdb")));
        assert!(
            imported
                .warnings
                .iter()
                .any(|w| w.contains("PostgreSQL - prod has no user")),
            "{:?}",
            imported.warnings
        );

        let maria = find("maria");
        assert_eq!(maria.driver, Driver::Mariadb);
        assert_eq!(maria.user.as_deref(), Some("alice"));

        let ora = find("ora");
        assert_eq!(
            (ora.driver, ora.service.as_deref()),
            (Driver::Oracle, Some("XE"))
        );
        assert!(imported.warnings.iter().any(|w| w.contains("SID XE")));

        let local = find("local");
        assert_eq!(local.driver, Driver::Sqlite);
        assert_eq!(local.path.as_deref(), Some("/home/me/app.db"));
        assert_eq!(local.mode.as_deref(), Some("ro"));
        assert_eq!(local.secret, Secret::None);

        let by_url = find("by url");
        assert_eq!(by_url.host.as_deref(), Some("::1"));
        assert_eq!(
            (by_url.port, by_url.database.as_deref()),
            (Some(6543), Some("reports"))
        );

        assert_eq!(imported.skipped.len(), 1);
        assert_eq!(imported.skipped[0].name, "docs");
        assert!(imported.skipped[0].reason.contains("mongodb"));
    }

    #[test]
    fn a_qry_backup_keeps_ids_and_password_modes() {
        let mut record = StoredConnection::new("prod", Driver::Postgres);
        record.host = Some("db.local".into());
        record.user = Some("alice".into());
        record.database = Some("app".into());
        record.secret = Secret::Env {
            var: "PGPASSWORD".into(),
        };
        let text = serde_json::json!({"version": 1, "connections": [record]}).to_string();

        let imported = import(&text);
        assert_eq!(imported.format, Format::Qry);
        assert_eq!(
            only(&imported),
            &record,
            "a backup comes back exactly as written"
        );
    }

    #[test]
    fn a_backup_from_a_newer_qry_is_refused() {
        let text = r#"{"version": 99, "connections": []}"#;
        let err = from_text(text, Format::Auto, Path::new("b.json")).unwrap_err();
        assert!(err.to_string().contains("newer qry"), "{err}");
    }

    #[test]
    fn files_qry_cannot_read_say_why() {
        let path = Path::new("x");
        let err = |text: &str| from_text(text, Format::Auto, path).unwrap_err().to_string();
        assert!(err("<?xml version=\"1.0\"?><References/>").contains("XML"));
        assert!(err("not json").contains("not JSON"));
        assert!(err(r#"{"something": 1}"#).contains("not a qry backup"));

        // A format chosen by hand that the file does not match.
        let err = from_text(DBEAVER, Format::SqlDeveloper, path)
            .unwrap_err()
            .to_string();
        assert!(err.contains("connections` array"), "{err}");
    }

    #[test]
    fn a_zip_archive_is_explained() {
        let dir = std::env::temp_dir().join(format!("qry-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("project.dbp");
        std::fs::write(&file, b"PK\x03\x04rest").unwrap();
        let err = from_file(file.to_str().unwrap(), Format::Auto)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("zip archive") && err.contains("data-sources.json"),
            "{err}"
        );

        let err = from_file(dir.join("missing.json").to_str().unwrap(), Format::Auto)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot read"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jdbc_urls_for_each_database() {
        let (driver, target) = jdbc("jdbc:postgresql://pg.local/app?sslmode=require").unwrap();
        assert_eq!(driver, Driver::Postgres);
        assert_eq!(
            target,
            Target {
                host: Some("pg.local".into()),
                database: Some("app".into()),
                ..Target::default()
            }
        );

        let (driver, target) = jdbc("jdbc:mariadb://a.local:3310,b.local/shop").unwrap();
        assert_eq!(
            (driver, target.host.as_deref(), target.port),
            (Driver::Mariadb, Some("a.local"), Some(3310))
        );

        let (_, target) = jdbc("jdbc:oracle:thin:scott/tiger@db.local:1521:ORCL").unwrap();
        assert_eq!(target.user.as_deref(), Some("scott"));
        assert_eq!(
            (target.database.as_deref(), target.sid),
            (Some("ORCL"), true)
        );
        assert!(
            !format!("{target:?}").contains("tiger"),
            "the password is dropped"
        );

        let (_, target) = jdbc("jdbc:oracle:thin:@db.local:1521/svc").unwrap();
        assert_eq!(
            (target.database.as_deref(), target.sid, target.tls),
            (Some("svc"), false, false)
        );

        let (driver, target) = jdbc("jdbc:sqlite:file:/tmp/x.db").unwrap();
        assert_eq!(
            (driver, target.database.as_deref()),
            (Driver::Sqlite, Some("/tmp/x.db"))
        );

        for bad in [
            "jdbc:oracle:thin:@(DESCRIPTION=(ADDRESS=(HOST=x)))",
            "jdbc:oracle:thin:@PRODDB",
            "jdbc:oracle:oci:@db",
            "jdbc:sqlserver://x",
            "postgresql://x/y",
            "jdbc:postgresql://x:99999/y",
        ] {
            assert!(jdbc(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_tilde_means_the_home_directory() {
        let home = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        assert_eq!(expand_home("~/a/b.json"), home.join("a/b.json"));
        assert_eq!(expand_home(" /abs.json "), PathBuf::from("/abs.json"));
    }
}
