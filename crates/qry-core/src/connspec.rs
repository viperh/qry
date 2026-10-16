//! What a connection dialog asks for, per database engine, and what it
//! accepts: the field table, the defaults and the validation rules.
//!
//! Pure data and functions with no IO, so the rules are tested here rather
//! than through a terminal. The `qry` crate's connection dialog builds a
//! [`Draft`] from what is typed, asks [`visible`] which rows to show, and
//! [`validate`] what is wrong before any connect attempt.

use std::collections::BTreeMap;

/// A database engine, as the dialog's Driver choice offers it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Engine {
    #[default]
    Postgres,
    Mysql,
    Mariadb,
    Sqlite,
    Oracle,
}

impl Engine {
    /// In the order the dialog offers them.
    pub const ALL: [Engine; 5] = [
        Engine::Postgres,
        Engine::Mysql,
        Engine::Mariadb,
        Engine::Sqlite,
        Engine::Oracle,
    ];

    /// What the Driver choice shows, same order as [`Engine::ALL`].
    pub const LABELS: [&'static str; 5] = ["PostgreSQL", "MySQL", "MariaDB", "SQLite", "Oracle"];

    /// The port a server of this kind listens on unless told otherwise;
    /// `None` for SQLite, which has no server.
    pub fn default_port(self) -> Option<u16> {
        match self {
            Engine::Postgres => Some(5432),
            Engine::Mysql | Engine::Mariadb => Some(3306),
            Engine::Oracle => Some(1521),
            Engine::Sqlite => None,
        }
    }

    /// Whether connecting needs a password at all.
    pub fn needs_password(self) -> bool {
        self != Engine::Sqlite
    }

    /// The lower-case name the dialog shows in its corner.
    pub fn name(self) -> &'static str {
        match self {
            Engine::Postgres => "postgres",
            Engine::Mysql => "mysql",
            Engine::Mariadb => "mariadb",
            Engine::Sqlite => "sqlite",
            Engine::Oracle => "oracle",
        }
    }
}

/// One thing a connection dialog can ask for. The connection fields come
/// from [`fields`]; `Name`, `Dsn` and `EnvVar` are asked for whatever the
/// engine, and errors can name any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldKind {
    Name,
    Host,
    Port,
    User,
    Password,
    Database,
    /// Oracle's service name or SID.
    Service,
    Schema,
    Ssl,
    /// Oracle's wallet directory, used over TLS.
    Wallet,
    /// SQLite's file.
    Path,
    /// SQLite's open mode: `rw`, `ro` or `rwc`.
    Mode,
    /// A whole connection string, used instead of the fields.
    Dsn,
    /// The environment variable a password is read from.
    EnvVar,
}

impl FieldKind {
    /// What the dialog's label column calls it.
    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Name => "Name",
            FieldKind::Host => "Host",
            FieldKind::Port => "Port",
            FieldKind::User => "User",
            FieldKind::Password => "Password",
            FieldKind::Database => "Database",
            FieldKind::Service => "Service/SID",
            FieldKind::Schema => "Schema",
            FieldKind::Ssl => "SSL",
            FieldKind::Wallet => "Wallet dir",
            FieldKind::Path => "File",
            FieldKind::Mode => "Mode",
            FieldKind::Dsn => "DSN",
            FieldKind::EnvVar => "Variable",
        }
    }
}

/// One row of the field table: what is asked, and what it starts as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub kind: FieldKind,
    /// What a new connection starts with: the default port, `prefer`, `rw`.
    pub default: Option<&'static str>,
}

const fn spec(kind: FieldKind, default: Option<&'static str>) -> FieldSpec {
    FieldSpec { kind, default }
}

const POSTGRES: &[FieldSpec] = &[
    spec(FieldKind::Host, None),
    spec(FieldKind::Port, Some("5432")),
    spec(FieldKind::User, None),
    spec(FieldKind::Password, None),
    spec(FieldKind::Database, None),
    spec(FieldKind::Schema, None),
    spec(FieldKind::Ssl, Some("prefer")),
];

const MYSQL: &[FieldSpec] = &[
    spec(FieldKind::Host, None),
    spec(FieldKind::Port, Some("3306")),
    spec(FieldKind::User, None),
    spec(FieldKind::Password, None),
    spec(FieldKind::Database, None),
    spec(FieldKind::Ssl, Some("prefer")),
];

const SQLITE: &[FieldSpec] = &[
    spec(FieldKind::Path, None),
    spec(FieldKind::Mode, Some("rw")),
];

const ORACLE: &[FieldSpec] = &[
    spec(FieldKind::Host, None),
    spec(FieldKind::Port, Some("1521")),
    spec(FieldKind::User, None),
    spec(FieldKind::Password, None),
    spec(FieldKind::Service, None),
    spec(FieldKind::Schema, None),
    spec(FieldKind::Ssl, Some("prefer")),
    spec(FieldKind::Wallet, None),
];

/// The connection fields of `engine`, in the order the dialog shows them.
/// [`visible`] narrows them to what applies to a draft.
pub fn fields(engine: Engine) -> &'static [FieldSpec] {
    match engine {
        Engine::Postgres => POSTGRES,
        Engine::Mysql | Engine::Mariadb => MYSQL,
        Engine::Sqlite => SQLITE,
        Engine::Oracle => ORACLE,
    }
}

/// The SSL modes `engine` offers, as libpq names them. MySQL and MariaDB
/// have no `allow`; SQLite has no transport.
pub fn ssl_modes(engine: Engine) -> &'static [&'static str] {
    match engine {
        Engine::Postgres | Engine::Oracle => &[
            "disable",
            "allow",
            "prefer",
            "require",
            "verify-ca",
            "verify-full",
        ],
        Engine::Mysql | Engine::Mariadb => {
            &["disable", "prefer", "require", "verify-ca", "verify-full"]
        }
        Engine::Sqlite => &[],
    }
}

/// SQLite's open modes: what the dialog shows, and what is stored.
pub const SQLITE_MODES: [(&str, &str); 3] =
    [("read-write", "rw"), ("read-only", "ro"), ("create", "rwc")];

/// Where a connection's password is kept between sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PasswordStore {
    #[default]
    Keychain,
    Ask,
    Environment,
}

/// What the dialog holds, without the password itself: enough to decide
/// which rows to show and whether it can be saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub engine: Engine,
    pub name: String,
    /// "Use DSN instead": one DSN field replaces the connection fields.
    pub use_dsn: bool,
    pub dsn: String,
    /// The connection fields' text, by kind; a choice holds its stored
    /// value (`prefer`, `rw`).
    pub values: BTreeMap<FieldKind, String>,
    pub store: PasswordStore,
    pub env_var: String,
}

impl Draft {
    /// A new draft for `engine`, with its defaults filled in.
    pub fn new(engine: Engine) -> Self {
        let values = fields(engine)
            .iter()
            .filter_map(|f| Some((f.kind, f.default?.to_string())))
            .collect();
        Self {
            engine,
            values,
            ..Self::default()
        }
    }

    /// The text of field `kind`, or `""`.
    pub fn value(&self, kind: FieldKind) -> &str {
        self.values.get(&kind).map_or("", String::as_str)
    }
}

/// The connection rows the dialog shows for `draft`, in order: the
/// engine's table, with the DSN alone in place of the fields when it is
/// used, the Variable in place of the Password when the password lives in
/// the environment, and Oracle's wallet only while SSL is on.
pub fn visible(draft: &Draft) -> Vec<FieldKind> {
    let engine = draft.engine;
    let password = |kind: FieldKind| match (kind, draft.store) {
        (FieldKind::Password, PasswordStore::Environment) => FieldKind::EnvVar,
        (kind, _) => kind,
    };
    if draft.use_dsn {
        let mut rows = vec![FieldKind::Dsn];
        if engine.needs_password() {
            rows.push(password(FieldKind::Password));
        }
        return rows;
    }
    fields(engine)
        .iter()
        .map(|f| f.kind)
        .filter(|kind| *kind != FieldKind::Wallet || draft.value(FieldKind::Ssl) != "disable")
        .map(password)
        .collect()
}

/// Switches `draft` to `to`. What both engines ask for keeps its value; the
/// rest goes. The port moves to the new default only if it was still the
/// old one (or empty), so a port the user typed stays. A choice whose value
/// the new engine does not offer goes back to its default.
pub fn switch_engine(draft: &mut Draft, to: Engine) {
    let from = draft.engine;
    let old_port = from.default_port().map(|p| p.to_string());
    let port = draft.value(FieldKind::Port).to_string();
    let untouched = port.is_empty() || Some(&port) == old_port.as_ref();
    let kept: Vec<FieldKind> = fields(to).iter().map(|f| f.kind).collect();
    draft.values.retain(|kind, _| kept.contains(kind));
    for spec in fields(to) {
        let entry = draft.values.entry(spec.kind).or_default();
        let offered = match spec.kind {
            FieldKind::Ssl => ssl_modes(to).contains(&entry.as_str()),
            FieldKind::Mode => SQLITE_MODES.iter().any(|(_, v)| v == entry),
            _ => true,
        };
        if entry.is_empty() || !offered {
            *entry = spec.default.unwrap_or_default().to_string();
        }
    }
    if untouched && let Some(port) = to.default_port() {
        draft.values.insert(FieldKind::Port, port.to_string());
    }
    draft.values.retain(|_, value| !value.is_empty());
    draft.engine = to;
}

/// The environment variable a password is read from by default:
/// `QRY_PW_` and the name, upper-cased, with anything else as `_`.
pub fn default_env_var(name: &str) -> String {
    let tail: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("QRY_PW_{tail}")
}

/// Whether `name` can name an environment variable: `[A-Z_][A-Z0-9_]*`.
pub fn is_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// What is wrong with one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: FieldKind,
    pub message: String,
}

impl FieldError {
    fn new(field: FieldKind, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// Everything wrong with `draft`, one error per field, in the order the
/// fields are shown. `taken` is the names of the other saved connections:
/// a name must differ from all of them, ignoring case.
pub fn validate(draft: &Draft, taken: &[&str]) -> Vec<FieldError> {
    let mut errors = Vec::new();
    let name = draft.name.trim();
    if name.is_empty() {
        errors.push(FieldError::new(FieldKind::Name, "a name is required"));
    } else if taken.iter().any(|t| t.trim().eq_ignore_ascii_case(name)) {
        errors.push(FieldError::new(
            FieldKind::Name,
            format!("\"{name}\" is already a saved connection"),
        ));
    }
    let engine = draft.engine;
    for kind in visible(draft) {
        let value = match kind {
            FieldKind::Dsn => draft.dsn.trim(),
            FieldKind::EnvVar => draft.env_var.trim(),
            kind => draft.value(kind).trim(),
        };
        let message = match kind {
            FieldKind::Dsn if value.is_empty() => Some("a DSN is required".to_string()),
            FieldKind::Port if !value.is_empty() => match value.parse::<u32>() {
                Ok(1..=65535) => None,
                _ => Some("must be a number from 1 to 65535".to_string()),
            },
            FieldKind::Path if value.is_empty() => Some("a file is required".to_string()),
            FieldKind::Host if value.is_empty() => Some("a host is required".to_string()),
            FieldKind::User if value.is_empty() => Some("a user is required".to_string()),
            FieldKind::Database if value.is_empty() && engine == Engine::Postgres => {
                Some("a database is required".to_string())
            }
            FieldKind::Service if value.is_empty() => {
                Some("a service name or SID is required".to_string())
            }
            FieldKind::EnvVar if value.is_empty() => Some("a variable is required".to_string()),
            FieldKind::EnvVar if !is_env_var_name(value) => {
                Some("must look like [A-Z_][A-Z0-9_]*".to_string())
            }
            _ => None,
        };
        if let Some(message) = message {
            errors.push(FieldError::new(kind, message));
        }
    }
    errors
}

#[cfg(test)]
#[path = "connspec_tests.rs"]
mod connspec_tests;
