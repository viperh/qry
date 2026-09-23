use qry_core::{
    ConnectionConfig, SslMode, mariadb::MariadbConfig, mysql::MySqlConfig,
    postgres::PostgresConfig, sqlite::SqliteConfig,
};

use crate::action::Action;
use crate::components::form::{Field, Form};

const LABELS: [&str; 9] = [
    "Name", "Type", "Host", "Port", "User", "Password", "Database", "Schema", "SSL mode",
];
// Indexes into `LABELS` and `ConnForm::fields`.
const NAME: usize = 0;
const TYPE: usize = 1;
const HOST: usize = 2;
const PORT: usize = 3;
const USER: usize = 4;
const PASSWORD: usize = 5;
const DATABASE: usize = 6;
const SCHEMA: usize = 7;
const SSL_MODE: usize = 8;

const DB_TYPES: [&str; 4] = ["PostgreSQL", "MySQL", "MariaDB", "SQLite"];
/// Same order as `SslMode::ALL`.
const SSL_MODES: [&str; 6] = ["disable", "allow", "prefer", "require", "verify-ca", "verify-full"];

/// The New Connection form.
pub struct ConnForm {
    fields: [Field; 9],
    focus: usize,
    error: Option<String>,
    /// Set while the database worker is trying the connection.
    pending: Option<&'static str>,
}

impl Default for ConnForm {
    fn default() -> Self {
        Self {
            fields: [
                Field::text(),
                Field::choice(&DB_TYPES, 0),
                Field::text(),
                Field::text(),
                Field::text(),
                Field::masked(),
                Field::text(),
                Field::text(),
                // "prefer", libpq's own default
                Field::choice(&SSL_MODES, 2),
            ],
            focus: 0,
            error: None,
            pending: None,
        }
    }
}

impl Form for ConnForm {
    fn title(&self) -> &'static str {
        "New Connection"
    }

    fn hint(&self) -> &'static str {
        "Enter connect · Esc cancel"
    }

    fn labels(&self) -> &'static [&'static str] {
        &LABELS
    }

    fn fields(&self) -> &[Field] {
        &self.fields
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    fn focus(&self) -> usize {
        self.focus
    }

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus;
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn pending(&self) -> Option<&'static str> {
        self.pending
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        self.pending = pending;
    }

    /// Turns the form into a connection name and config, or explains what is
    /// wrong in a message short enough for the popup's bottom border.
    fn submit(&self) -> Result<Action, String> {
        let trimmed = |field: usize| self.text(field).trim().to_string();
        let required = |field: usize| {
            let value = trimmed(field);
            if value.is_empty() {
                Err(format!("{} is required", LABELS[field]))
            } else {
                Ok(value)
            }
        };
        let name = trimmed(NAME);
        let schema = trimmed(SCHEMA);
        let db_type = DB_TYPES[self.selected(TYPE)];

        if db_type == "SQLite" {
            if !schema.is_empty() {
                return Err("SQLite has no schemas; leave Schema empty".into());
            }
            let path = required(DATABASE)
                .map_err(|_| "Database (the SQLite file path) is required".to_string())?;
            return Ok(Action::Connect(
                name,
                ConnectionConfig::Sqlite(SqliteConfig::new(path, false)),
            ));
        }

        let host = required(HOST)?;
        let port = match trimmed(PORT).as_str() {
            "" if db_type == "PostgreSQL" => 5432,
            "" => 3306,
            port => port
                .parse::<u16>()
                .ok()
                .filter(|&port| port != 0)
                .ok_or("Port must be a number from 1 to 65535")?,
        };
        let user = required(USER)?;
        // Not trimmed: leading or trailing spaces can be part of a password.
        let password = self.text(PASSWORD).to_string();
        let database = required(DATABASE)?;
        let ssl_mode = SslMode::ALL[self.selected(SSL_MODE)];

        let config = match db_type {
            "PostgreSQL" => ConnectionConfig::Postgres(PostgresConfig {
                host,
                port,
                user,
                password,
                database,
                schema: (!schema.is_empty()).then_some(schema),
                ssl_mode,
            }),
            _ if !schema.is_empty() => {
                return Err(format!("{db_type} has no separate schemas; use Database"));
            }
            "MySQL" => ConnectionConfig::Mysql(MySqlConfig { host, port, user, password, database, ssl_mode }),
            _ => ConnectionConfig::MariaDb(MariadbConfig { host, port, user, password, database, ssl_mode }),
        };
        Ok(Action::Connect(name, config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::form::tests::{key, lines, press, render, type_str};
    use crossterm::event::KeyCode;

    fn set(form: &mut ConnForm, field: usize, value: &str) {
        let Field::Text(input) = &mut form.fields[field] else {
            panic!("{} is not a text field", LABELS[field]);
        };
        input.value = value.into();
        input.cursor = input.len();
    }

    fn choose(form: &mut ConnForm, field: usize, option: &str) {
        let Field::Choice { options, selected } = &mut form.fields[field] else {
            panic!("{} is not a choice", LABELS[field]);
        };
        *selected = options.iter().position(|o| *o == option).unwrap();
    }

    fn filled() -> ConnForm {
        let mut form = ConnForm::default();
        set(&mut form, NAME, " prod ");
        set(&mut form, HOST, "db.local");
        set(&mut form, USER, "alice");
        set(&mut form, PASSWORD, " s3cret ");
        set(&mut form, DATABASE, "app");
        set(&mut form, SCHEMA, "reporting");
        form
    }

    #[test]
    fn postgres_form_builds_its_config() {
        assert_eq!(
            filled().submit().unwrap(),
            Action::Connect(
                "prod".into(),
                ConnectionConfig::Postgres(PostgresConfig {
                    host: "db.local".into(),
                    port: 5432,
                    user: "alice".into(),
                    password: " s3cret ".into(),
                    database: "app".into(),
                    schema: Some("reporting".into()),
                    ssl_mode: SslMode::Prefer,
                })
            )
        );
    }

    #[test]
    fn mysql_and_mariadb_default_to_port_3306_and_reject_a_schema() {
        let mut form = filled();
        choose(&mut form, TYPE, "MySQL");
        choose(&mut form, SSL_MODE, "disable");
        assert_eq!(
            form.submit().unwrap_err(),
            "MySQL has no separate schemas; use Database"
        );

        set(&mut form, SCHEMA, "");
        let Ok(Action::Connect(_, ConnectionConfig::Mysql(mysql))) = form.submit() else {
            panic!("expected a MySQL connection");
        };
        assert_eq!((mysql.port, mysql.ssl_mode), (3306, SslMode::Disable));

        choose(&mut form, TYPE, "MariaDB");
        set(&mut form, PORT, "3307");
        let Ok(Action::Connect(_, ConnectionConfig::MariaDb(mariadb))) = form.submit() else {
            panic!("expected a MariaDB connection");
        };
        assert_eq!(mariadb.port, 3307);
    }

    #[test]
    fn sqlite_only_needs_a_file_path() {
        let mut form = ConnForm::default();
        choose(&mut form, TYPE, "SQLite");
        assert_eq!(
            form.submit().unwrap_err(),
            "Database (the SQLite file path) is required"
        );
        set(&mut form, DATABASE, "C:/data/app.db");
        assert_eq!(
            form.submit().unwrap(),
            Action::Connect(
                String::new(),
                ConnectionConfig::Sqlite(SqliteConfig::new("C:/data/app.db", false))
            )
        );
    }

    #[test]
    fn invalid_forms_explain_what_is_wrong() {
        let mut form = filled();
        set(&mut form, HOST, "   ");
        assert_eq!(form.submit().unwrap_err(), "Host is required");

        let mut form = filled();
        for port in ["abc", "0", "70000"] {
            set(&mut form, PORT, port);
            assert_eq!(form.submit().unwrap_err(), "Port must be a number from 1 to 65535");
        }
    }

    #[test]
    fn ssl_mode_labels_line_up_with_core() {
        let core: Vec<String> = SslMode::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(core, SSL_MODES);
    }

    #[test]
    fn type_and_ssl_mode_are_choices() {
        let mut form = ConnForm::default();
        form.set_focus(TYPE);
        press(&mut form, key(KeyCode::Right));
        assert_eq!(DB_TYPES[form.selected(TYPE)], "MySQL");
        press(&mut form, key(KeyCode::Char('x')));
        assert_eq!(DB_TYPES[form.selected(TYPE)], "MySQL");

        form.set_focus(SSL_MODE);
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "prefer");
        press(&mut form, key(KeyCode::Left));
        press(&mut form, key(KeyCode::Left));
        assert_eq!(SSL_MODES[form.selected(SSL_MODE)], "disable");
    }

    #[test]
    fn typing_reaches_the_focused_field_and_the_password_is_masked() {
        let mut form = ConnForm::default();
        type_str(&mut form, "prod");
        press(&mut form, key(KeyCode::Tab));
        press(&mut form, key(KeyCode::Tab));
        type_str(&mut form, "db.local");
        form.set_focus(PASSWORD);
        type_str(&mut form, "hunter2");

        assert_eq!(form.text(NAME), "prod");
        assert_eq!(form.text(HOST), "db.local");

        let shown = lines(&render(&form, 80, 50)).join("\n");
        for label in LABELS {
            assert!(shown.contains(label), "missing {label}: {shown}");
        }
        assert!(shown.contains("•••••••"), "{shown}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
