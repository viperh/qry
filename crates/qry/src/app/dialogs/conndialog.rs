//! The connection dialog: new, edit and duplicate.
//!
//! Which rows it shows follows the driver, from the field table in
//! [`qry_core::connspec`]: SQLite asks for a file and a mode, Oracle for a
//! service and, over TLS, a wallet, and "use a DSN instead" swaps them all
//! for one DSN. Rows a change hides keep their values, so turning the DSN
//! off again brings them back. Validation runs before any connect attempt
//! and puts each problem under its field.

use std::collections::BTreeMap;

use qry_core::connspec::{self, Draft, Engine, FieldError, FieldKind, PasswordStore, SQLITE_MODES};

use super::{
    Submission,
    form::{Field, Form, Note, Tone},
};
use crate::{
    action::Action,
    connections::{ConnLabel, Driver, Secret, StoredConnection},
    secrets::Password,
};

/// One row of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Key {
    Driver,
    Name,
    UseDsn,
    /// A connection field from the table, or the DSN, the password or its
    /// variable.
    Conn(FieldKind),
    /// Where the password is kept.
    Store,
    Label,
    /// The `[ Test ]` button.
    Test,
}

impl Key {
    fn label(self) -> &'static str {
        match self {
            Key::Driver => "Driver",
            Key::Name => "Name",
            Key::UseDsn | Key::Test => "",
            Key::Conn(kind) => kind.label(),
            Key::Store => "Password in",
            Key::Label => "Label",
        }
    }
}

/// Where the password is kept, as the Password in choice offers it.
const STORES: [&str; 3] = ["keychain", "ask each time", "environment"];
const KEYCHAIN: usize = 0;
const ASK: usize = 1;
const ENVIRONMENT: usize = 2;

/// The Label choice: none, then `ConnLabel::ALL`.
const LABELS: [&str; 4] = ["none", "dev", "staging", "prod"];

/// SQLite's modes as the Mode choice shows them, same order as
/// `SQLITE_MODES`.
const MODES: [&str; 3] = ["read-write", "read-only", "create"];

/// How a test of the typed connection stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum TestState {
    #[default]
    Idle,
    /// Waiting for the answer to test number `n`; any other is stale.
    Running(u64),
    Passed {
        millis: u64,
        version: String,
    },
    Failed(String),
}

/// What the dialog was opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    New,
    Edit,
    Duplicate,
}

/// The connection dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConnDialog {
    keys: Vec<Key>,
    fields: Vec<Field>,
    labels: Vec<&'static str>,
    /// Rows the current choices hide, with what they held.
    hidden: BTreeMap<Key, Field>,
    focus: usize,
    error: Option<String>,
    field_errors: Vec<(Key, String)>,
    /// Set while the database worker is trying the connection.
    pending: Option<&'static str>,
    /// Minted once, so retrying after a failure does not make a second
    /// connection with a second keychain entry.
    id: String,
    purpose: Purpose,
    /// Where its password lived before an edit.
    was: Secret,
    /// The record being edited or duplicated, for the keys it carries that
    /// this build does not know.
    base: Option<StoredConnection>,
    /// The driver the rows were laid out for.
    engine: Engine,
    /// The name the variable's default was made from.
    last_name: String,
    /// The names of the other saved connections; the name must differ.
    taken: Vec<String>,
    pub(crate) test: TestState,
}

impl ConnDialog {
    /// An empty dialog for a new connection, which will have this id.
    pub(crate) fn new(id: impl Into<String>) -> Self {
        let engine = Engine::Postgres;
        let draft = Draft::new(engine);
        let mut hidden = BTreeMap::new();
        for spec in connspec::fields(engine) {
            hidden.insert(
                Key::Conn(spec.kind),
                field_from(spec.kind, engine, draft.value(spec.kind)),
            );
        }
        let mut dialog = Self {
            keys: Vec::new(),
            fields: Vec::new(),
            labels: Vec::new(),
            hidden,
            focus: 0,
            error: None,
            field_errors: Vec::new(),
            pending: None,
            id: id.into(),
            purpose: Purpose::New,
            was: Secret::None,
            base: None,
            engine,
            last_name: String::new(),
            taken: Vec::new(),
            test: TestState::Idle,
        };
        dialog.relayout();
        // The name is what one types first.
        dialog.focus = dialog.position(Key::Name).unwrap_or(0);
        dialog
    }

    /// Opens an existing connection for changing. The password field starts
    /// empty: leaving it be keeps whatever the connection already used.
    pub(crate) fn editing(record: &StoredConnection) -> Self {
        let mut dialog = Self::filled(record, record.id.clone(), Purpose::Edit);
        dialog.was = record.secret.clone();
        if record.secret == Secret::Keyring
            && let Some(Field::Secret(input)) = dialog.field_mut(Key::Conn(FieldKind::Password))
        {
            input.placeholder = Some("(unchanged)");
        }
        dialog
    }

    /// A copy of `record` under a new id and `name (copy)`. Its password is
    /// not copied: a keychain one becomes "ask each time".
    pub(crate) fn duplicate(record: &StoredConnection, id: impl Into<String>) -> Self {
        let mut copy = record.clone();
        copy.name = format!("{} (copy)", record.title());
        if copy.secret == Secret::Keyring {
            copy.secret = Secret::Prompt;
        }
        Self::filled(&copy, id.into(), Purpose::Duplicate)
    }

    fn filled(record: &StoredConnection, id: String, purpose: Purpose) -> Self {
        let mut dialog = Self::new(id);
        dialog.purpose = purpose;
        dialog.base = Some(record.clone());
        let engine = Engine::from(record.driver);
        dialog.set_choice(
            Key::Driver,
            Engine::ALL.iter().position(|e| *e == engine).unwrap_or(0),
        );
        dialog.relayout();

        // Oracle records written before it spoke TLS keep the wallet in
        // `tls`; `oracle_tls` reads either shape, and saving writes the new.
        let (tls, wallet) = match record.driver {
            Driver::Oracle => match record.oracle_tls() {
                Ok((mode, wallet)) => (Some(mode.to_string()), wallet),
                Err(_) => (record.tls.clone(), record.wallet.clone()),
            },
            _ => (record.tls.clone(), record.wallet.clone()),
        };
        let port = record.port.or(engine.default_port()).map(|p| p.to_string());
        let mode = record.mode.clone().unwrap_or_else(|| "rw".into());
        let values: [(FieldKind, Option<String>); 11] = [
            (FieldKind::Host, record.host.clone()),
            (FieldKind::Port, port),
            (FieldKind::User, record.user.clone()),
            (FieldKind::Database, record.database.clone()),
            (FieldKind::Service, record.service.clone()),
            (FieldKind::Schema, record.schema.clone()),
            (FieldKind::Ssl, tls),
            (FieldKind::Wallet, wallet),
            (FieldKind::Path, record.path.clone()),
            (FieldKind::Mode, Some(mode)),
            (FieldKind::Dsn, record.dsn.clone()),
        ];
        for (kind, value) in values {
            if let Some(value) = value {
                dialog.set_value(Key::Conn(kind), &value);
            }
        }
        dialog.set_text(Key::Name, &record.name);
        dialog.last_name = record.name.clone();
        if record.dsn.is_some() {
            dialog.set_checked(Key::UseDsn, true);
        }
        let store = match &record.secret {
            Secret::Keyring => KEYCHAIN,
            Secret::Env { var } => {
                dialog.set_text(Key::Conn(FieldKind::EnvVar), var);
                ENVIRONMENT
            }
            Secret::Prompt | Secret::None => ASK,
        };
        dialog.set_choice(Key::Store, store);
        let label = record
            .label
            .and_then(|l| ConnLabel::ALL.iter().position(|x| *x == l))
            .map_or(0, |i| i + 1);
        dialog.set_choice(Key::Label, label);
        dialog.relayout();
        dialog.focus = dialog.position(Key::Name).unwrap_or(0);
        dialog
    }

    /// The names the new name must differ from.
    pub(crate) fn set_taken(&mut self, taken: Vec<String>) {
        self.taken = taken;
    }

    /// The id the saved connection will have.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Whether Tab has reached the `[ Test ]` button, so Enter tests.
    pub(crate) fn on_test_button(&self) -> bool {
        self.keys.get(self.focus) == Some(&Key::Test)
    }

    /// The rows, in order, for tests.
    #[cfg(test)]
    pub(crate) fn keys(&self) -> &[Key] {
        &self.keys
    }

    pub(crate) fn position(&self, key: Key) -> Option<usize> {
        self.keys.iter().position(|k| *k == key)
    }

    fn field(&self, key: Key) -> Option<&Field> {
        match self.position(key) {
            Some(i) => self.fields.get(i),
            None => self.hidden.get(&key),
        }
    }

    pub(crate) fn field_mut(&mut self, key: Key) -> Option<&mut Field> {
        match self.position(key) {
            Some(i) => self.fields.get_mut(i),
            None => self.hidden.get_mut(&key),
        }
    }

    pub(crate) fn set_text(&mut self, key: Key, value: &str) {
        let field = match self.field_mut(key) {
            Some(field) => field,
            None => self.hidden.entry(key).or_insert_with(Field::text),
        };
        match field {
            Field::Text(input) => {
                input.value = value.to_string();
                input.cursor = input.len();
            }
            Field::Secret(input) => {
                input.value = Password::new(value);
                input.cursor = input.len();
            }
            _ => {}
        }
    }

    pub(crate) fn set_choice(&mut self, key: Key, option: usize) {
        if let Some(Field::Choice { selected, options }) = self.field_mut(key)
            && option < options.len()
        {
            *selected = option;
        }
    }

    pub(crate) fn set_checked(&mut self, key: Key, value: bool) {
        if let Some(Field::Checkbox { checked, .. }) = self.field_mut(key) {
            *checked = value;
        }
        self.relayout();
    }

    /// Sets a connection field from its stored value: the text, or the
    /// choice whose stored value it is.
    fn set_value(&mut self, key: Key, value: &str) {
        let Key::Conn(kind) = key else {
            return self.set_text(key, value);
        };
        match kind {
            FieldKind::Ssl | FieldKind::Mode => {
                let field = field_from(kind, self.engine, value);
                match self.position(key) {
                    Some(i) => self.fields[i] = field,
                    None => _ = self.hidden.insert(key, field),
                }
            }
            _ => self.set_text(key, value),
        }
    }

    fn text_of(&self, key: Key) -> String {
        match self.field(key) {
            Some(Field::Text(input)) => input.value.clone(),
            _ => String::new(),
        }
    }

    fn choice_of(&self, key: Key) -> usize {
        match self.field(key) {
            Some(Field::Choice { selected, .. }) => *selected,
            _ => 0,
        }
    }

    fn store(&self) -> PasswordStore {
        match self.choice_of(Key::Store) {
            KEYCHAIN => PasswordStore::Keychain,
            ENVIRONMENT => PasswordStore::Environment,
            _ => PasswordStore::Ask,
        }
    }

    /// What is typed, as the field table's rules read it.
    pub(crate) fn draft(&self) -> Draft {
        let mut values = BTreeMap::new();
        for spec in connspec::fields(self.engine) {
            let key = Key::Conn(spec.kind);
            let value = match (spec.kind, self.field(key)) {
                (FieldKind::Ssl, Some(Field::Choice { options, selected })) => options
                    .get(*selected)
                    .copied()
                    .unwrap_or_default()
                    .to_string(),
                (FieldKind::Mode, Some(Field::Choice { selected, .. })) => {
                    SQLITE_MODES[(*selected).min(2)].1.to_string()
                }
                (_, Some(Field::Text(input))) => input.value.clone(),
                _ => continue,
            };
            values.insert(spec.kind, value);
        }
        Draft {
            engine: self.engine,
            name: self.text_of(Key::Name),
            use_dsn: matches!(
                self.field(Key::UseDsn),
                Some(Field::Checkbox { checked: true, .. })
            ),
            dsn: self.text_of(Key::Conn(FieldKind::Dsn)),
            values,
            store: self.store(),
            env_var: self.text_of(Key::Conn(FieldKind::EnvVar)),
        }
    }

    /// Lays the rows out for the current choices: a new driver takes the
    /// values both share (and its default port, if the port was still the
    /// old default), and the variable's default follows the name.
    fn relayout(&mut self) {
        let focused = self.keys.get(self.focus).copied();
        let mut pool = std::mem::take(&mut self.hidden);
        for (key, field) in self.keys.drain(..).zip(self.fields.drain(..)) {
            pool.insert(key, field);
        }
        let chosen = pool
            .get(&Key::Driver)
            .and_then(|f| match f {
                Field::Choice { selected, .. } => Engine::ALL.get(*selected).copied(),
                _ => None,
            })
            .unwrap_or(self.engine);
        // The draft reads the fields through `self`, so put them back first.
        self.hidden = pool;
        if chosen != self.engine {
            let mut draft = self.draft();
            connspec::switch_engine(&mut draft, chosen);
            self.hidden.retain(|key, _| {
                !matches!(key, Key::Conn(kind) if !matches!(kind, FieldKind::Password | FieldKind::EnvVar | FieldKind::Dsn))
            });
            for spec in connspec::fields(chosen) {
                if spec.kind != FieldKind::Password {
                    self.hidden.insert(
                        Key::Conn(spec.kind),
                        field_from(spec.kind, chosen, draft.value(spec.kind)),
                    );
                }
            }
            self.engine = chosen;
        }

        let draft = self.draft();
        if draft.store == PasswordStore::Environment {
            let name = draft.name.trim().to_string();
            let var = draft.env_var.trim().to_string();
            if var.is_empty() || var == connspec::default_env_var(&self.last_name) {
                self.set_text(
                    Key::Conn(FieldKind::EnvVar),
                    &connspec::default_env_var(&name),
                );
            }
        }
        self.last_name = draft.name.trim().to_string();

        let mut keys = vec![Key::Driver, Key::Name, Key::UseDsn];
        for kind in connspec::visible(&draft) {
            keys.push(Key::Conn(kind));
            if matches!(kind, FieldKind::Password | FieldKind::EnvVar) {
                keys.push(Key::Store);
            }
        }
        keys.extend([Key::Label, Key::Test]);
        for key in &keys {
            let field = self
                .hidden
                .remove(key)
                .unwrap_or_else(|| new_field(*key, self.engine));
            self.fields.push(field);
        }
        self.labels = keys.iter().map(|k| k.label()).collect();
        self.keys = keys;
        self.focus = match focused.and_then(|k| self.position(k)) {
            Some(i) => i,
            None => self.focus.min(self.keys.len() - 1),
        };
    }

    /// Every problem with what is typed, by row, as the field table's rules
    /// see it.
    fn problems(&self) -> Vec<FieldError> {
        let taken: Vec<&str> = self.taken.iter().map(String::as_str).collect();
        connspec::validate(&self.draft(), &taken)
    }

    /// Puts `errors` under their fields and says which in the footer.
    /// Returns whether there were none.
    fn show(&mut self, errors: Vec<FieldError>) -> bool {
        self.field_errors = errors
            .iter()
            .map(|e| (Key::Conn(e.field), e.message.clone()))
            .map(|(key, message)| match key {
                Key::Conn(FieldKind::Name) => (Key::Name, message),
                key => (key, message),
            })
            .collect();
        let Some(first) = errors.first() else {
            return true;
        };
        self.error = Some(format!("{}: {}", first.field.label(), first.message));
        let key = self.field_errors[0].0;
        if let Some(i) = self.position(key) {
            self.focus = i;
        }
        false
    }

    /// Checks what a test needs, which is everything but the name.
    pub(crate) fn check_for_test(&mut self) -> bool {
        let errors = self
            .problems()
            .into_iter()
            .filter(|e| e.field != FieldKind::Name)
            .collect();
        self.show(errors)
    }

    /// The record to save and open, and the password typed beside it. The
    /// password is never in the record.
    pub(crate) fn build(&self) -> (StoredConnection, Option<Password>) {
        let draft = self.draft();
        let driver = Driver::from(self.engine);
        let some = |value: &str| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        };
        let mut record = match &self.base {
            Some(base) => StoredConnection {
                id: self.id.clone(),
                driver,
                extra: base.extra.clone(),
                ..StoredConnection::with_id(self.id.clone(), "", driver)
            },
            None => StoredConnection::with_id(self.id.clone(), "", driver),
        };
        record.name = draft.name.trim().to_string();
        let shown = connspec::visible(&draft);
        let value = |kind: FieldKind| {
            shown
                .contains(&kind)
                .then(|| some(draft.value(kind)))
                .flatten()
        };
        if draft.use_dsn {
            record.dsn = some(&draft.dsn);
        } else {
            record.host = value(FieldKind::Host);
            record.port = value(FieldKind::Port)
                .and_then(|p| p.parse::<u16>().ok())
                .filter(|p| Some(*p) != self.engine.default_port());
            record.user = value(FieldKind::User);
            record.database = value(FieldKind::Database);
            record.service = value(FieldKind::Service);
            record.schema = value(FieldKind::Schema);
            record.tls = value(FieldKind::Ssl);
            record.wallet = value(FieldKind::Wallet);
            record.path = value(FieldKind::Path);
            record.mode = value(FieldKind::Mode).filter(|m| m != "rw");
        }
        let label = self.choice_of(Key::Label);
        record.label = label.checked_sub(1).map(|i| ConnLabel::ALL[i]);

        // Not trimmed: spaces can be part of a password.
        let password = match self.field(Key::Conn(FieldKind::Password)) {
            Some(Field::Secret(input)) if draft.store != PasswordStore::Environment => {
                input.value.duplicate()
            }
            _ => Password::default(),
        };
        let editing = self.purpose == Purpose::Edit;
        record.secret = match (self.engine.needs_password(), draft.store) {
            (false, _) => Secret::None,
            (true, PasswordStore::Environment) => Secret::Env {
                var: draft.env_var.trim().to_string(),
            },
            // Editing and leaving the password be keeps what it used.
            (true, PasswordStore::Keychain) if password.is_empty() && editing => match &self.was {
                Secret::Keyring => Secret::Keyring,
                _ => Secret::Prompt,
            },
            // Nothing typed means there is nothing to keep, so it is asked
            // for on every connect.
            (true, PasswordStore::Keychain) if password.is_empty() => Secret::Prompt,
            (true, PasswordStore::Keychain) => Secret::Keyring,
            (true, PasswordStore::Ask) => Secret::Prompt,
        };
        let password = (!password.is_empty()).then_some(password);
        (record, password)
    }
}

/// A connection field of `engine` holding `value`: text, or the choice
/// whose stored value it is.
fn field_from(kind: FieldKind, engine: Engine, value: &str) -> Field {
    match kind {
        FieldKind::Ssl => {
            let options = connspec::ssl_modes(engine);
            let at = options
                .iter()
                .position(|o| *o == value)
                .or_else(|| options.iter().position(|o| *o == "prefer"))
                .unwrap_or(0);
            Field::choice(options, at)
        }
        FieldKind::Mode => Field::choice(
            &MODES,
            SQLITE_MODES
                .iter()
                .position(|(_, v)| *v == value)
                .unwrap_or(0),
        ),
        _ => {
            let mut field = new_field(Key::Conn(kind), engine);
            if let Field::Text(input) = &mut field {
                input.value = value.to_string();
                input.cursor = input.len();
            }
            field
        }
    }
}

/// A row as a new dialog starts it.
fn new_field(key: Key, engine: Engine) -> Field {
    match key {
        Key::Driver => Field::choice(&Engine::LABELS, 0),
        Key::Name => Field::text(),
        Key::UseDsn => Field::checkbox(false, "use a DSN instead"),
        Key::Store => Field::choice(&STORES, KEYCHAIN),
        Key::Label => Field::choice(&LABELS, 0),
        Key::Test => Field::button("Test"),
        Key::Conn(FieldKind::Password) => Field::masked(),
        Key::Conn(FieldKind::Path) => Field::text_with_placeholder("~/app.db or :memory:"),
        Key::Conn(FieldKind::Dsn) => Field::text_with_placeholder("postgres://user@host/db"),
        Key::Conn(FieldKind::Wallet) => Field::text_with_placeholder("folder with ewallet.pem"),
        Key::Conn(kind @ (FieldKind::Ssl | FieldKind::Mode)) => field_from(kind, engine, ""),
        Key::Conn(_) => Field::text(),
    }
}

impl Form for ConnDialog {
    fn title(&self) -> &'static str {
        match self.purpose {
            Purpose::New => "New Connection",
            Purpose::Edit => "Edit Connection",
            Purpose::Duplicate => "Duplicate Connection",
        }
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "save & connect"),
            (Action::TestConnection, "test"),
            (Action::NextField, "next"),
            (Action::Right, "change"),
            (Action::Cancel, "cancel"),
        ]
    }

    fn width(&self) -> u16 {
        76
    }

    fn subtitle(&self) -> Option<String> {
        Some(self.engine.name().to_string())
    }

    fn labels(&self) -> &[&'static str] {
        &self.labels
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
        self.focus = focus.min(self.keys.len().saturating_sub(1));
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn field_error(&self, field: usize) -> Option<&str> {
        let key = self.keys.get(field)?;
        self.field_errors
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, message)| message.as_str())
    }

    fn note(&self) -> Option<Note> {
        match &self.test {
            TestState::Idle => None,
            TestState::Running(_) => Some(Note::new(Tone::Busy, "testing the connection…")),
            TestState::Passed { millis, version } => Some(Note::new(
                Tone::Ok,
                format!("✓ connected in {millis} ms · {version}"),
            )),
            TestState::Failed(message) => Some(Note::new(Tone::Error, format!("✗ {message}"))),
        }
    }

    fn pending(&self) -> Option<&'static str> {
        self.pending
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        self.pending = pending;
    }

    /// The rows follow the driver, the DSN toggle, the password's store and
    /// the SSL mode. Editing a field clears its error.
    fn after_change(&mut self) {
        if let Some(key) = self.keys.get(self.focus).copied() {
            self.field_errors.retain(|(k, _)| *k != key);
        }
        self.relayout();
    }

    fn check(&mut self) -> bool {
        let problems = self.problems();
        self.show(problems)
    }

    fn submit(&self) -> Result<Submission, String> {
        if let Some(first) = self.problems().first() {
            return Err(format!("{}: {}", first.field.label(), first.message));
        }
        let (record, password) = self.build();
        Ok(Submission::Connect {
            record: Box::new(record),
            password,
        })
    }
}

/// Helpers for tests elsewhere that fill the dialog in.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{ConnDialog, Key};
    use crate::{app::dialogs::DialogState, testing::AppHarness};
    pub(crate) use qry_core::connspec::FieldKind;

    /// The open connection dialog.
    pub(crate) fn dialog(h: &mut AppHarness) -> &mut ConnDialog {
        match h.state_mut().dialog_mut() {
            Some(DialogState::Connection(dialog)) => dialog,
            other => panic!("no connection dialog: {other:?}"),
        }
    }

    /// Moves to the row `key` and types `value` into it, as a user would.
    pub(crate) fn type_into(h: &mut AppHarness, key: Key, value: &str) {
        let dialog = dialog(h);
        let at = dialog
            .position(key)
            .unwrap_or_else(|| panic!("no {key:?} row in {:?}", dialog.keys()));
        crate::app::dialogs::form::Form::set_focus(dialog, at);
        h.type_str(value);
    }

    /// Fills a Postgres connection called `name` on db.local, enough to
    /// submit.
    pub(crate) fn fill(h: &mut AppHarness, name: &str) {
        type_into(h, Key::Name, name);
        type_into(h, Key::Conn(FieldKind::Host), "db.local");
        type_into(h, Key::Conn(FieldKind::User), "alice");
        type_into(h, Key::Conn(FieldKind::Database), "app");
    }
}

#[cfg(test)]
#[path = "conndialog_tests.rs"]
mod conndialog_tests;
