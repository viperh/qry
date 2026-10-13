use super::{
    Submission,
    form::{Field, Form},
};
use crate::{
    connections::{Driver, Secret, StoredConnection},
    secrets::Password,
};

const LABELS: [&str; 11] = [
    "Name",
    "Type",
    "Host",
    "Port",
    "User",
    "Password",
    "Database",
    "Schema",
    "SSL mode",
    "Wallet",
    "Password in",
];
/// Same, plus the variable name that only ENVIRONMENT shows.
const LABELS_VAR: [&str; 12] = [
    "Name",
    "Type",
    "Host",
    "Port",
    "User",
    "Password",
    "Database",
    "Schema",
    "SSL mode",
    "Wallet",
    "Password in",
    "Variable",
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
const WALLET: usize = 9;
const STORE: usize = 10;
const VAR: usize = 11;

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
const SSL_MODES: [&str; 6] = [
    "disable",
    "allow",
    "prefer",
    "require",
    "verify-ca",
    "verify-full",
];

/// Where the password is kept between sessions. qry never writes one to a
/// file of its own, so these are the only choices.
const STORES: [&str; 3] = ["KEYCHAIN", "ASK EACH TIME", "ENVIRONMENT"];
const KEYCHAIN: usize = 0;
const ASK: usize = 1;
const ENVIRONMENT: usize = 2;

/// The New Connection form.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConnForm {
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

impl ConnForm {
    /// An empty form for a new connection, which will have this id.
    pub(crate) fn new(id: impl Into<String>) -> Self {
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
                Field::text_with_placeholder("Oracle only: folder with ewallet.pem"),
                Field::choice(&STORES, KEYCHAIN),
            ],
            focus: 0,
            error: None,
            pending: None,
            id: id.into(),
            editing: false,
            was: Secret::None,
        }
    }

    /// Opens an existing connection for changing. The password field starts
    /// empty: leaving it be keeps whatever the connection already used.
    pub(crate) fn editing(record: &StoredConnection) -> Self {
        let mut form = Self {
            editing: true,
            was: record.secret.clone(),
            ..Self::new(record.id.clone())
        };

        form.set_text(NAME, &record.name);
        form.set_choice(
            TYPE,
            DRIVERS
                .iter()
                .position(|d| *d == record.driver)
                .unwrap_or(0),
        );
        form.set_text(HOST, record.host.as_deref().unwrap_or_default());
        form.set_text(
            PORT,
            &record.port.map(|p| p.to_string()).unwrap_or_default(),
        );
        form.set_text(USER, record.user.as_deref().unwrap_or_default());
        form.set_text(SCHEMA, record.schema.as_deref().unwrap_or_default());
        // The one field the drivers disagree about.
        let database = match record.driver {
            Driver::Sqlite => record.path.as_deref(),
            Driver::Oracle => record.service.as_deref(),
            _ => record.database.as_deref(),
        };
        form.set_text(DATABASE, database.unwrap_or_default());
        // An old Oracle record keeps its wallet in `tls`; `oracle_tls`
        // reads either shape, so saving the form writes the new one.
        let tls = match record.driver {
            Driver::Oracle => record.oracle_tls().ok().map(|(mode, wallet)| {
                form.set_text(WALLET, wallet.as_deref().unwrap_or_default());
                mode.to_string()
            }),
            _ => record.tls.clone(),
        };
        if let Some(tls) = tls
            && let Some(mode) = SSL_MODES.iter().position(|m| *m == tls)
        {
            form.set_choice(SSL_MODE, mode);
        }

        match &record.secret {
            Secret::Keyring => {
                form.set_choice(STORE, KEYCHAIN);
                if let Field::Secret(input) = &mut form.fields[PASSWORD] {
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
        if self.editing {
            "Edit Connection"
        } else {
            "New Connection"
        }
    }

    fn hint(&self) -> &'static str {
        "Enter connect · Esc cancel"
    }

    fn labels(&self) -> &'static [&'static str] {
        if self.fields.len() > LABELS.len() {
            &LABELS_VAR
        } else {
            &LABELS
        }
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
    /// password travels beside it, for the runtime to leave in the stash;
    /// it is never in the record.
    fn submit(&self) -> Result<Submission, String> {
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
            ..StoredConnection::with_id("", "", driver)
        };
        // The one field the drivers disagree about: a file, a service, or a
        // database name.
        match driver {
            Driver::Sqlite => record.path = database,
            Driver::Oracle => record.service = database,
            _ => record.database = database,
        }
        // SQLite has no transport.
        if driver != Driver::Sqlite {
            record.tls = Some(SSL_MODES[self.selected(SSL_MODE)].to_string());
        }
        record.wallet = some(trimmed(WALLET));
        if record.wallet.is_some() && driver != Driver::Oracle {
            return Err("Wallet is only used by Oracle".into());
        }

        // Not trimmed: spaces can be part of a password.
        let password = self
            .secret(PASSWORD)
            .map(Password::duplicate)
            .unwrap_or_default();
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
        record
            .to_config(password.as_str())
            .map_err(|e| e.to_string())?;

        Ok(Submission::Connect {
            record: Box::new(record),
            password: (!password.is_empty()).then_some(password),
        })
    }
}

#[cfg(test)]
#[path = "connform_tests.rs"]
mod connform_tests;
