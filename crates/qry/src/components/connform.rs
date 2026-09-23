use crate::action::Action;
use crate::components::form::{Field, Form};
use crate::connections::{Driver, Secret, StoredConnection};
use crate::secrets::{self, Password};

const LABELS: [&str; 10] = [
    "Name", "Type", "Host", "Port", "User", "Password", "Database", "Schema", "SSL mode",
    "Password in",
];
/// Same, plus the variable name that only ENVIRONMENT shows.
const LABELS_VAR: [&str; 11] = [
    "Name", "Type", "Host", "Port", "User", "Password", "Database", "Schema", "SSL mode",
    "Password in", "Variable",
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
const STORE: usize = 9;
const VAR: usize = 10;

const DB_TYPES: [&str; 5] = ["PostgreSQL", "MySQL", "MariaDB", "SQLite", "Oracle"];
/// Same order as `DB_TYPES`.
const DRIVERS: [Driver; 5] = [
    Driver::Postgres,
    Driver::Mysql,
    Driver::Mariadb,
    Driver::Sqlite,
    Driver::Oracle,
];
/// Same order as `SslMode::ALL`.
const SSL_MODES: [&str; 6] = ["disable", "allow", "prefer", "require", "verify-ca", "verify-full"];

/// Where the password is kept between sessions. qry never writes one to a
/// file of its own, so these are the only choices.
const STORES: [&str; 3] = ["KEYCHAIN", "ASK EACH TIME", "ENVIRONMENT"];
const KEYCHAIN: usize = 0;
const ASK: usize = 1;
const ENVIRONMENT: usize = 2;

/// The New Connection form.
pub struct ConnForm {
    fields: Vec<Field>,
    focus: usize,
    error: Option<String>,
    /// Set while the database worker is trying the connection.
    pending: Option<&'static str>,
    /// Minted once, so retrying after a failure does not make a second
    /// connection with a second keychain entry.
    id: String,
    /// Editing an existing connection rather than making one, which changes
    /// what an empty password field means.
    editing: bool,
    /// Where its password lived before the edit.
    was: Secret,
}

impl Default for ConnForm {
    fn default() -> Self {
        Self {
            fields: vec![
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
                Field::choice(&STORES, KEYCHAIN),
            ],
            focus: 0,
            error: None,
            pending: None,
            id: StoredConnection::new("", Driver::Sqlite).id,
            editing: false,
            was: Secret::None,
        }
    }
}

impl ConnForm {
    /// Opens an existing connection for changing. The password field starts
    /// empty: leaving it be keeps whatever the connection already used.
    pub fn editing(record: &StoredConnection) -> Self {
        let mut form = Self {
            id: record.id.clone(),
            editing: true,
            was: record.secret.clone(),
            ..Self::default()
        };

        form.set_text(NAME, &record.name);
        form.set_choice(TYPE, DRIVERS.iter().position(|d| *d == record.driver).unwrap_or(0));
        form.set_text(HOST, record.host.as_deref().unwrap_or_default());
        form.set_text(PORT, &record.port.map(|p| p.to_string()).unwrap_or_default());
        form.set_text(USER, record.user.as_deref().unwrap_or_default());
        form.set_text(SCHEMA, record.schema.as_deref().unwrap_or_default());
        // The one field the drivers disagree about.
        let database = match record.driver {
            Driver::Sqlite => record.path.as_deref(),
            Driver::Oracle => record.service.as_deref(),
            _ => record.database.as_deref(),
        };
        form.set_text(DATABASE, database.unwrap_or_default());
        if let Some(tls) = &record.tls
            && let Some(mode) = SSL_MODES.iter().position(|m| m == tls)
        {
            form.set_choice(SSL_MODE, mode);
        }

        match &record.secret {
            Secret::Keyring => {
                form.set_choice(STORE, KEYCHAIN);
                if let Field::Text(input) = &mut form.fields[PASSWORD] {
                    input.placeholder = Some("(unchanged)");
                }
            }
            Secret::Env { var } => {
                form.set_choice(STORE, ENVIRONMENT);
                form.after_change();
                form.set_text(VAR, var);
            }
            Secret::Prompt | Secret::None => form.set_choice(STORE, ASK),
        }
        form
    }

    fn set_text(&mut self, field: usize, value: &str) {
        if let Field::Text(input) = &mut self.fields[field] {
            input.value = value.to_string();
            input.cursor = input.len();
        }
    }

    fn set_choice(&mut self, field: usize, option: usize) {
        if let Field::Choice { selected, .. } = &mut self.fields[field] {
            *selected = option;
        }
    }

    fn driver(&self) -> Driver {
        DRIVERS[self.selected(TYPE)]
    }

    fn wants_var(&self) -> bool {
        self.selected(STORE) == ENVIRONMENT
    }
}

impl Form for ConnForm {
    fn title(&self) -> &'static str {
        if self.editing { "Edit Connection" } else { "New Connection" }
    }

    fn hint(&self) -> &'static str {
        "Enter connect · Esc cancel"
    }

    fn labels(&self) -> &'static [&'static str] {
        if self.fields.len() > LABELS.len() { &LABELS_VAR } else { &LABELS }
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

    /// The variable name is only asked for when the password lives in one.
    fn after_change(&mut self) {
        let base = LABELS.len();
        if self.wants_var() && self.fields.len() == base {
            self.fields.push(Field::text());
        } else if !self.wants_var() && self.fields.len() > base {
            self.fields.truncate(base);
            self.focus = self.focus.min(base - 1);
        }
    }

    /// Builds the record the tree stores and the worker opens. The typed
    /// password is left in the stash, never in the record and never in the
    /// action.
    fn submit(&self) -> Result<Action, String> {
        let trimmed = |field: usize| self.text(field).trim().to_string();
        let some = |value: String| (!value.is_empty()).then_some(value);
        let driver = self.driver();

        let port = match trimmed(PORT).as_str() {
            "" => None,
            port => Some(
                port.parse::<u16>()
                    .ok()
                    .filter(|&port| port != 0)
                    .ok_or("Port must be a number from 1 to 65535")?,
            ),
        };

        let database = some(trimmed(DATABASE));
        let mut record = StoredConnection {
            id: self.id.clone(),
            name: trimmed(NAME),
            driver,
            host: some(trimmed(HOST)),
            port,
            user: some(trimmed(USER)),
            schema: some(trimmed(SCHEMA)),
            ..StoredConnection::new("", driver)
        };
        // The one field the drivers disagree about: a file, a service, or a
        // database name.
        match driver {
            Driver::Sqlite => record.path = database,
            Driver::Oracle => record.service = database,
            _ => record.database = database,
        }
        // SQLite has no transport, and Oracle reads `tls` as a wallet path.
        if !matches!(driver, Driver::Sqlite | Driver::Oracle) {
            record.tls = Some(SSL_MODES[self.selected(SSL_MODE)].to_string());
        }

        // Not trimmed: spaces can be part of a password.
        let password = self.text(PASSWORD).to_string();
        record.secret = match (driver.needs_password(), self.selected(STORE)) {
            (false, _) => Secret::None,
            (true, ENVIRONMENT) => match some(trimmed(VAR)) {
                Some(var) => Secret::Env { var },
                None => return Err("Variable is required".into()),
            },
            // Editing and leaving the password be keeps what it used.
            (true, KEYCHAIN) if password.is_empty() && self.editing => self.was.clone(),
            // Nothing typed means there is nothing to keep, so it is asked
            // for on every connect.
            (true, KEYCHAIN) if password.is_empty() => Secret::Prompt,
            (true, KEYCHAIN) => Secret::Keyring,
            (true, _) => Secret::Prompt,
        };

        // Checks the fields this driver needs, with the same messages the
        // worker would give.
        record.to_config(&password).map_err(|e| e.to_string())?;

        if !password.is_empty() {
            secrets::stash(Password::new(password));
        }
        Ok(Action::Connect(Box::new(record)))
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

    fn record(form: &ConnForm) -> StoredConnection {
        match form.submit() {
            Ok(Action::Connect(record)) => *record,
            other => panic!("expected a connection, got {other:?}"),
        }
    }

    #[test]
    fn the_form_builds_a_record_with_no_password_in_it() {
        let _guard = secrets::test_lock();
        let _ = secrets::take_stash();
        let built = record(&filled());

        assert_eq!(built.name, "prod");
        assert_eq!(built.driver, Driver::Postgres);
        assert_eq!(built.host.as_deref(), Some("db.local"));
        assert_eq!(built.user.as_deref(), Some("alice"));
        assert_eq!(built.database.as_deref(), Some("app"));
        assert_eq!(built.schema.as_deref(), Some("reporting"));
        assert_eq!(built.tls.as_deref(), Some("prefer"));
        assert_eq!(built.port, None, "left to the driver's default");
        assert_eq!(built.secret, Secret::Keyring);

        // The record can be written without leaking anything.
        let written = serde_json::to_string(&built).unwrap();
        assert!(!written.contains("s3cret"), "{written}");

        // The typed password went to the stash instead, spaces and all.
        assert_eq!(
            secrets::take_stash().map(|p| p.as_str().to_string()),
            Some(" s3cret ".to_string())
        );
    }

    #[test]
    fn the_id_survives_a_retry() {
        let _guard = secrets::test_lock();
        let form = filled();
        assert_eq!(record(&form).id, record(&form).id);
    }

    #[test]
    fn where_the_password_lives_follows_the_choice() {
        let _guard = secrets::test_lock();
        let mut form = filled();
        choose(&mut form, STORE, "ASK EACH TIME");
        assert_eq!(record(&form).secret, Secret::Prompt);

        // Nothing typed: there is nothing to keep.
        let mut form = filled();
        set(&mut form, PASSWORD, "");
        choose(&mut form, STORE, "KEYCHAIN");
        assert_eq!(record(&form).secret, Secret::Prompt);

        // SQLite needs no password at all.
        let mut form = ConnForm::default();
        choose(&mut form, TYPE, "SQLite");
        set(&mut form, DATABASE, "app.db");
        let built = record(&form);
        assert_eq!(built.secret, Secret::None);
        assert_eq!(built.path.as_deref(), Some("app.db"), "the file, not a database name");
        assert_eq!(built.tls, None);
    }

    #[test]
    fn an_environment_variable_needs_its_name() {
        let _guard = secrets::test_lock();
        let mut form = filled();
        choose(&mut form, STORE, "ENVIRONMENT");
        form.after_change();
        assert_eq!(form.labels(), LABELS_VAR);
        assert_eq!(form.submit().unwrap_err(), "Variable is required");

        set(&mut form, VAR, "PGPASSWORD");
        assert_eq!(record(&form).secret, Secret::Env { var: "PGPASSWORD".into() });

        // Choosing another store takes the field away again.
        choose(&mut form, STORE, "KEYCHAIN");
        form.after_change();
        assert_eq!(form.labels(), LABELS);
    }

    #[test]
    fn oracle_stores_the_database_field_as_a_service() {
        let _guard = secrets::test_lock();
        let mut form = filled();
        choose(&mut form, TYPE, "Oracle");
        let built = record(&form);
        assert_eq!(built.service.as_deref(), Some("app"));
        assert_eq!(built.database, None);
    }

    #[test]
    fn editing_opens_the_form_filled_in_and_keeps_the_id() {
        let _guard = secrets::test_lock();
        let _ = secrets::take_stash();
        let mut saved = record(&filled());
        saved.port = Some(6543);
        saved.secret = Secret::Keyring;

        let form = ConnForm::editing(&saved);
        assert_eq!(form.title(), "Edit Connection");
        assert_eq!(form.text(NAME), "prod");
        assert_eq!(form.text(HOST), "db.local");
        assert_eq!(form.text(PORT), "6543");
        assert_eq!(form.text(DATABASE), "app");
        assert_eq!(form.text(SCHEMA), "reporting");
        assert_eq!(form.text(PASSWORD), "", "the password is never shown back");

        let shown = lines(&render(&form, 80, 60)).join("\n");
        assert!(shown.contains("(unchanged)"), "{shown}");

        // Saving it again is the same connection, not a second one.
        let edited = record(&form);
        assert_eq!(edited.id, saved.id);
        assert_eq!(edited.secret, Secret::Keyring, "its password is untouched");
        assert_eq!(edited.port, Some(6543));
    }

    #[test]
    fn editing_can_change_where_the_password_lives() {
        let _guard = secrets::test_lock();
        let mut saved = record(&filled());
        saved.secret = Secret::Keyring;

        // Asking each time from now on.
        let mut form = ConnForm::editing(&saved);
        choose(&mut form, STORE, "ASK EACH TIME");
        assert_eq!(record(&form).secret, Secret::Prompt);

        // Or typing a new one, which replaces what the keychain holds.
        let mut form = ConnForm::editing(&saved);
        set(&mut form, PASSWORD, "newer");
        assert_eq!(record(&form).secret, Secret::Keyring);
        assert_eq!(
            secrets::take_stash().map(|p| p.as_str().to_string()),
            Some("newer".to_string())
        );
    }

    #[test]
    fn editing_an_environment_connection_shows_its_variable() {
        let mut saved = record(&filled());
        saved.secret = Secret::Env { var: "PGPASSWORD".into() };

        let form = ConnForm::editing(&saved);
        assert_eq!(form.labels(), LABELS_VAR);
        assert_eq!(form.text(VAR), "PGPASSWORD");
    }

    #[test]
    fn invalid_forms_explain_what_is_wrong() {
        let _guard = secrets::test_lock();
        let mut form = filled();
        set(&mut form, HOST, "   ");
        assert!(form.submit().unwrap_err().contains("needs a host"));

        let mut form = filled();
        for port in ["abc", "0", "70000"] {
            set(&mut form, PORT, port);
            assert_eq!(form.submit().unwrap_err(), "Port must be a number from 1 to 65535");
        }
    }

    #[test]
    fn ssl_mode_labels_line_up_with_core() {
        let core: Vec<String> = qry_core::SslMode::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(core, SSL_MODES);
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

        let shown = lines(&render(&form, 80, 60)).join("\n");
        for label in LABELS {
            assert!(shown.contains(label), "missing {label}: {shown}");
        }
        assert!(shown.contains("•••••••"), "{shown}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
