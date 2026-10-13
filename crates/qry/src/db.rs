//! Background task that owns the database connection.
//!
//! The event loop never awaits a query. It sends a [`DbCommand`] here, and the
//! outcome comes back later as an [`Action`] on the normal action channel.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use qry_core::{Driver, ExportConfig, Exporter, QueryResult};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{
    action::{Action, StatusCode},
    connections::{Secret, StoredConnection},
    secrets::{self, Password},
};

pub(crate) enum DbCommand {
    /// Open this connection. The record holds no password: the worker
    /// resolves the secret from the keychain, the environment, or a prompt.
    Connect(Box<StoredConnection>),
    /// A prompted password is waiting in the stash for the connection the
    /// worker last asked about.
    PasswordEntered,
    Query(String),
    /// Writes the last result to a file; no second trip to the database.
    Export(ExportConfig),
    /// The tables of the open connection, for the tree.
    ListTables,
}

/// The connection and the last result it produced.
#[derive(Default)]
struct Worker {
    driver: Driver,
    last: Option<Arc<QueryResult>>,
    /// The connection waiting for a password to be typed.
    asked: Option<StoredConnection>,
}

/// Starts the worker and returns the sender used to reach it. Commands run one
/// at a time, in order. The worker stops once every sender has been dropped.
pub(crate) fn spawn(action_tx: UnboundedSender<Action>) -> UnboundedSender<DbCommand> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut worker = Worker::default();
        while let Some(command) = rx.recv().await {
            for action in worker.run(command).await {
                if action_tx.send(action).is_err() {
                    return;
                }
            }
        }
    });
    tx
}

impl Worker {
    async fn run(&mut self, command: DbCommand) -> Vec<Action> {
        match command {
            DbCommand::Connect(record) => self.connect(*record).await,
            DbCommand::PasswordEntered => match self.asked.take() {
                Some(record) => {
                    let typed = secrets::take_stash().unwrap_or_default();
                    self.open(record, typed, true).await
                }
                None => Vec::new(),
            },
            DbCommand::Query(sql) => {
                let started = Instant::now();
                match self.driver.query(&sql).await {
                    Ok(result) => {
                        let took = took(started.elapsed());
                        let summary = match result.rows.len() {
                            // A CREATE or an INSERT returns no columns to fetch.
                            _ if result.columns.is_empty() => {
                                format!("Statement executed in {took}")
                            }
                            1 => format!("Fetched 1 row in {took}"),
                            n => format!("Fetched {n} rows in {took}"),
                        };
                        let result = Arc::new(result);
                        self.last = Some(result.clone());
                        vec![Action::QueryDone(result), success(summary)]
                    }
                    Err(e) => vec![error(format!("{e:#}"))],
                }
            }
            DbCommand::Export(config) => self.export(config).await,
            DbCommand::ListTables => match self.driver.tables().await {
                Ok(tables) => vec![Action::TablesLoaded(tables)],
                Err(e) => vec![Action::TablesFailed(e.to_string()), error(format!("{e:#}"))],
            },
        }
    }

    /// Finds the password this record says it uses, and opens the
    /// connection. Anything it cannot find without the user is asked for,
    /// and the stored mode is never rewritten behind their back.
    async fn connect(&mut self, record: StoredConnection) -> Vec<Action> {
        // A password typed into the New Connection form is waiting here.
        if let Some(typed) = secrets::take_stash() {
            return self.open(record, typed, true).await;
        }
        match record.secret.clone() {
            Secret::None => self.open(record, Password::default(), false).await,
            Secret::Env { var } => match secrets::from_env(&var) {
                Some(password) => self.open(record, password, false).await,
                None => self.ask(record),
            },
            Secret::Keyring => {
                let id = record.id.clone();
                match tokio::task::spawn_blocking(move || secrets::fetch(&id)).await {
                    Ok(Ok(Some(password))) => self.open(record, password, false).await,
                    // No entry, or no keychain to ask: prompt for this
                    // connect only.
                    _ => self.ask(record),
                }
            }
            Secret::Prompt => self.ask(record),
        }
    }

    fn ask(&mut self, record: StoredConnection) -> Vec<Action> {
        let action = Action::NeedPassword {
            id: record.id.clone(),
            name: record.title().to_string(),
        };
        self.asked = Some(record);
        vec![action]
    }

    /// Opens the connection with the password in hand. `typed` says the user
    /// just entered it, which is what makes it worth keeping.
    async fn open(
        &mut self,
        record: StoredConnection,
        password: Password,
        typed: bool,
    ) -> Vec<Action> {
        let config = match record.to_config(password.as_str()) {
            Ok(config) => config,
            Err(e) => return failed(&format!("{e:#}")),
        };
        let label = match self.driver.connect(config).await {
            Ok(label) => label,
            // Several drivers put the whole DSN, password included, in their
            // error text, so nothing goes out without passing through here.
            Err(e) => return failed(&format!("{e:#}")),
        };

        let shown = if record.name.is_empty() {
            label.clone()
        } else {
            format!("{} ({label})", record.name)
        };
        let mut actions = vec![
            Action::Connected(label),
            success(format!("Connected to {shown}")),
        ];

        // Only a freshly typed password is worth storing, and only once it is
        // known to work.
        if typed && record.secret == Secret::Keyring && !password.is_empty() {
            let id = record.id.clone();
            let stored = tokio::task::spawn_blocking(move || secrets::store(&id, &password)).await;
            if !matches!(stored, Ok(Ok(()))) {
                actions.push(Action::SecretNotStored(record.id.clone()));
                actions.push(error(format!(
                    "{} is saved, but the keychain would not take its password: you will be asked each time",
                    record.title()
                )));
            }
        }
        actions
    }

    async fn export(&self, config: ExportConfig) -> Vec<Action> {
        let Some(result) = &self.last else {
            let message = "Nothing to export yet: run a query first";
            return vec![Action::ExportFailed(message.into()), error(message.into())];
        };
        let exporter = Exporter::from_result(result, &config);
        let etype = config.etype;

        // Writing a big file would block this task, and with it every later
        // query, so it goes to a thread that is allowed to block.
        let written = tokio::task::spawn_blocking(move || exporter.write(etype)).await;
        match written {
            Ok(Ok(rows)) => vec![
                Action::Exported(config.path.clone()),
                success(format!("Exported {rows} rows to {}", config.path)),
            ],
            // As with a connection, the modal gets the short message and the
            // status bar the whole chain.
            Ok(Err(e)) => vec![
                Action::ExportFailed(e.to_string()),
                error(format!("Export failed: {e:#}")),
            ],
            Err(e) => vec![
                Action::ExportFailed(e.to_string()),
                error(format!("Export failed: {e}")),
            ],
        }
    }
}

/// How long something took, in the unit that reads best: `0.4 ms`, `12 ms`,
/// `1.40 s`.
fn took(elapsed: Duration) -> String {
    match elapsed.as_secs_f64() {
        seconds if seconds >= 1.0 => format!("{seconds:.2} s"),
        _ if elapsed.as_millis() >= 1 => format!("{} ms", elapsed.as_millis()),
        _ => format!("{:.1} ms", elapsed.as_secs_f64() * 1000.0),
    }
}

fn success(message: String) -> Action {
    Action::Status(StatusCode::Success(message))
}

fn error(message: String) -> Action {
    Action::Status(StatusCode::Error(message))
}

/// A connection that did not open: the short message for the modal, the whole
/// chain for the status bar, both with any password taken out of them.
fn failed(message: &str) -> Vec<Action> {
    let message = secrets::redact(message);
    let short = message.lines().next().unwrap_or_default().to_string();
    vec![
        Action::ConnectFailed(short),
        error(format!("Connection failed: {message}")),
    ]
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod db_tests;
