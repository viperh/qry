//! Background task that owns the database connection.
//!
//! The event loop never awaits a query. It sends a [`DbCommand`] here, and the
//! outcome comes back later as [`DbAnswer`]s, tagged with the session's id.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use qry_core::{
    Driver, ExportConfig, Exporter, QueryResult,
    schema::{Children, SchemaPath},
    shared::CancelHandle,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{
    app::{event::DbAnswer, state::SessionId},
    connections::{Secret, StoredConnection},
    notice::Notice,
    secrets::{self, Password},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbCommand {
    /// Open this connection. The record holds no password: the worker
    /// resolves the secret from the keychain, the environment, or a prompt.
    Connect(Box<StoredConnection>),
    /// A prompted password is waiting in the stash, under the id of the
    /// connection the worker last asked about.
    PasswordEntered,
    /// Runs a statement, keeping the first `max_rows` rows.
    Query(String),
    /// Runs a statement and keeps every row.
    QueryAll(String),
    /// Writes the last result to a file; no second trip to the database.
    Export(ExportConfig),
    /// One node of the schema browser: the catalogs, a catalog's objects, or
    /// an object's columns and indexes. The answer carries `generation`
    /// back, so the tree can tell it from a later load of the same node.
    LoadSchema { path: SchemaPath, generation: u64 },
}

/// The connection and the last result it produced.
#[derive(Default)]
struct Worker {
    driver: Driver,
    last: Option<Arc<QueryResult>>,
    /// The connection waiting for a password to be typed.
    asked: Option<StoredConnection>,
    /// The most rows a query keeps; 0 keeps all.
    max_rows: usize,
    /// Where the open connection's cancel handle is left, for the runtime
    /// to stop a statement while this worker waits on it.
    cancel: CancelSlot,
}

/// The cancel handle of a session's open connection, if it has one.
pub(crate) type CancelSlot = Arc<Mutex<Option<Arc<dyn CancelHandle>>>>;

/// Answers as they arrive from the workers, each tagged with its session.
pub(crate) type Answers = UnboundedSender<(SessionId, DbAnswer)>;

/// Starts the worker for session `id`, keeping at most `max_rows` rows of
/// a query, and returns the sender used to reach it and where it leaves
/// its connection's cancel handle. Commands run one at a time, in order.
/// The worker stops once every sender has been dropped, or once nobody
/// listens to its answers.
pub(crate) fn spawn(
    id: SessionId,
    answers: Answers,
    max_rows: usize,
) -> (UnboundedSender<DbCommand>, CancelSlot) {
    let cancel = CancelSlot::default();
    let worker = Worker {
        max_rows,
        cancel: cancel.clone(),
        ..Worker::default()
    };
    (spawn_worker(worker, id, answers), cancel)
}

/// Locks the cancel slot, even if a panic poisoned it: it only ever holds
/// a handle or nothing.
pub(crate) fn lock(slot: &CancelSlot) -> MutexGuard<'_, Option<Arc<dyn CancelHandle>>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `worker` on its own task; tests hand in one whose driver is a fake.
fn spawn_worker(mut worker: Worker, id: SessionId, answers: Answers) -> UnboundedSender<DbCommand> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            for answer in worker.run(command).await {
                if answers.send((id, answer)).is_err() {
                    return;
                }
            }
        }
    });
    tx
}

impl Worker {
    async fn run(&mut self, command: DbCommand) -> Vec<DbAnswer> {
        match command {
            DbCommand::Connect(record) => self.connect(*record).await,
            DbCommand::PasswordEntered => match self.asked.take() {
                Some(record) => {
                    let typed = secrets::take_stash(&record.id).unwrap_or_default();
                    self.open(record, typed, true).await
                }
                None => Vec::new(),
            },
            DbCommand::Query(sql) => self.query(&sql, self.max_rows).await,
            DbCommand::QueryAll(sql) => self.query(&sql, 0).await,
            DbCommand::Export(config) => self.export(config).await,
            DbCommand::LoadSchema { path, generation } => {
                let outcome = self.driver.load_schema(&path).await;
                vec![schema_answer(path, generation, outcome)]
            }
        }
    }

    /// Runs `sql`, keeping at most `max_rows` rows (0 keeps all), and
    /// remembers the result for an export.
    async fn query(&mut self, sql: &str, max_rows: usize) -> Vec<DbAnswer> {
        let started = Instant::now();
        let outcome = self.driver.query_limited(sql, max_rows).await;
        // Where in the statement the server stopped, when it says.
        let position = outcome
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<qry_core::shared::QueryError>())
            .and_then(|e| e.position);
        let answers = query_answer(outcome, started.elapsed(), position);
        if let Some(DbAnswer::QueryDone(result)) = answers.first() {
            self.last = Some(result.clone());
        }
        answers
    }

    /// Finds the password this record says it uses, and opens the
    /// connection. Anything it cannot find without the user is asked for,
    /// and the stored mode is never rewritten behind their back.
    async fn connect(&mut self, record: StoredConnection) -> Vec<DbAnswer> {
        // A password typed into the New Connection form is waiting here.
        if let Some(typed) = secrets::take_stash(&record.id) {
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

    fn ask(&mut self, record: StoredConnection) -> Vec<DbAnswer> {
        let answer = DbAnswer::NeedPassword {
            id: record.id.clone(),
            name: record.title().to_string(),
        };
        self.asked = Some(record);
        vec![answer]
    }

    /// Opens the connection with the password in hand. `typed` says the user
    /// just entered it, which is what makes it worth keeping.
    async fn open(
        &mut self,
        record: StoredConnection,
        password: Password,
        typed: bool,
    ) -> Vec<DbAnswer> {
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

        *lock(&self.cancel) = self.driver.cancel_handle();
        let mut answers = connected_answer(&record, label);

        // Only a freshly typed password is worth storing, and only once it is
        // known to work.
        if typed && record.secret == Secret::Keyring && !password.is_empty() {
            let id = record.id.clone();
            let stored = tokio::task::spawn_blocking(move || secrets::store(&id, &password)).await;
            if !matches!(stored, Ok(Ok(()))) {
                answers.push(DbAnswer::SecretNotStored(record.id.clone()));
                answers.push(error(format!(
                    "{} is saved, but the keychain would not take its password: you will be asked each time",
                    record.title()
                )));
            }
        }
        answers
    }

    async fn export(&self, config: ExportConfig) -> Vec<DbAnswer> {
        let Some(result) = &self.last else {
            let message = "Nothing to export yet: run a query first";
            return vec![
                DbAnswer::ExportFailed(message.into()),
                error(message.into()),
            ];
        };
        let exporter = Exporter::from_result(result, &config);
        let etype = config.etype;

        // Writing a big file would block this task, and with it every later
        // query, so it goes to a thread that is allowed to block.
        let written = tokio::task::spawn_blocking(move || exporter.write(etype)).await;
        match written {
            Ok(Ok(rows)) => vec![
                DbAnswer::Exported(config.path.clone()),
                success(format!("Exported {rows} rows to {}", config.path)),
            ],
            // As with a connection, the modal gets the short message and the
            // status bar the whole chain.
            Ok(Err(e)) => vec![
                DbAnswer::ExportFailed(e.to_string()),
                error(format!("Export failed: {e:#}")),
            ],
            Err(e) => vec![
                DbAnswer::ExportFailed(e.to_string()),
                error(format!("Export failed: {e}")),
            ],
        }
    }
}

/// What the worker reports for a finished query: the result and a summary
/// for the status bar, or the error. The app harness in [`crate::testing`]
/// answers with the same answers, so its stand-in worker cannot drift from
/// this one.
pub(crate) fn query_answer<E: std::fmt::Display>(
    outcome: Result<QueryResult, E>,
    elapsed: Duration,
    position: Option<usize>,
) -> Vec<DbAnswer> {
    match outcome {
        Ok(mut result) => {
            result.elapsed = elapsed;
            let took = took(elapsed);
            let summary = match result.rows.len() {
                // A CREATE or an INSERT returns no columns to fetch.
                _ if result.columns.is_empty() => format!("Statement executed in {took}"),
                1 => format!("Fetched 1 row in {took}"),
                n => format!("Fetched {n} rows in {took}"),
            };
            vec![DbAnswer::QueryDone(Arc::new(result)), success(summary)]
        }
        Err(e) => vec![
            DbAnswer::QueryFailed {
                message: e.to_string(),
                position,
            },
            error(format!("{e:#}")),
        ],
    }
}

/// What the worker reports for a schema load, for the tree. A failure is
/// not a toast: the tree shows it on the node that failed.
pub(crate) fn schema_answer<E: std::fmt::Display>(
    path: SchemaPath,
    generation: u64,
    outcome: Result<Children, E>,
) -> DbAnswer {
    DbAnswer::Schema {
        path,
        generation,
        outcome: outcome.map_err(|e| {
            let message = secrets::redact(&format!("{e:#}"));
            message.lines().next().unwrap_or_default().to_string()
        }),
    }
}

/// What the worker reports once `record` is open as `label`: the answer the
/// modal waits for, and the name the status bar shows.
pub(crate) fn connected_answer(record: &StoredConnection, label: String) -> Vec<DbAnswer> {
    let shown = if record.name.is_empty() {
        label.clone()
    } else {
        format!("{} ({label})", record.name)
    };
    vec![
        DbAnswer::Connected(label),
        success(format!("Connected to {shown}")),
    ]
}

/// How long something took, in the unit that reads best: `0.4 ms`, `12 ms`,
/// `1.40 s`.
pub(crate) fn took(elapsed: Duration) -> String {
    qry_core::jobs::format_elapsed(elapsed)
}

fn success(message: String) -> DbAnswer {
    DbAnswer::Notice(Notice::success(message))
}

fn error(message: String) -> DbAnswer {
    DbAnswer::Notice(Notice::error(message))
}

/// A connection that did not open: the short message for the modal, the whole
/// chain for the status bar, both with any password taken out of them.
fn failed(message: &str) -> Vec<DbAnswer> {
    let message = secrets::redact(message);
    let short = message.lines().next().unwrap_or_default().to_string();
    vec![
        DbAnswer::ConnectFailed(short),
        error(format!("Connection failed: {message}")),
    ]
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod db_tests;
