//! Background task that owns the database connection.
//!
//! The event loop never awaits a query. It sends a [`DbCommand`] here, and the
//! outcome comes back later as an [`Action`] on the normal action channel.

use std::sync::Arc;
use std::time::{Duration, Instant};

use qry_core::{Driver, ExportConfig, Exporter, QueryResult};

use crate::connections::{Secret, StoredConnection};
use crate::secrets::{self, Password};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::action::{Action, StatusCode};

pub enum DbCommand {
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
pub fn spawn(action_tx: UnboundedSender<Action>) -> UnboundedSender<DbCommand> {
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
                        _ if result.columns.is_empty() => format!("Statement executed in {took}"),
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
        let mut actions = vec![Action::Connected(label), success(format!("Connected to {shown}"))];

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
            Ok(Err(e)) => vec![Action::ExportFailed(e.to_string()), error(format!("Export failed: {e:#}"))],
            Err(e) => vec![Action::ExportFailed(e.to_string()), error(format!("Export failed: {e}"))],
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
mod tests {
    use super::*;
    use crate::connections::Driver as StoredDriver;
    use qry_core::ExportType;

    /// A SQLite connection that needs no password.
    fn sqlite(name: &str, path: &str) -> StoredConnection {
        let mut record = StoredConnection::new(name, StoredDriver::Sqlite);
        record.path = Some(path.to_string());
        record.secret = Secret::None;
        record
    }

    fn in_memory() -> DbCommand {
        DbCommand::Connect(Box::new(sqlite("", ":memory:")))
    }

    #[tokio::test]
    async fn named_connection_shows_its_name() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        db.send(DbCommand::Connect(Box::new(sqlite("scratch", ":memory:")))).unwrap();
        assert_eq!(action_rx.recv().await, Some(Action::Connected(":memory:".into())));
        assert_eq!(
            action_rx.recv().await,
            Some(success("Connected to scratch (:memory:)".into()))
        );
    }

    #[tokio::test]
    // The guard only serialises tests against each other; nothing in the
    // worker waits on it, so holding it across an await cannot deadlock.
    #[allow(clippy::await_holding_lock)]
    async fn a_password_in_the_environment_is_used_without_asking() {
        let _guard = secrets::test_lock();
        let _ = secrets::take_stash();
        // SAFETY: the name belongs to this test.
        unsafe { std::env::set_var("QRY_TEST_DB_PASSWORD", "hunter2") };

        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        let mut record = sqlite("env", ":memory:");
        record.secret = Secret::Env { var: "QRY_TEST_DB_PASSWORD".into() };
        db.send(DbCommand::Connect(Box::new(record))).unwrap();

        assert_eq!(action_rx.recv().await, Some(Action::Connected(":memory:".into())));
    }

    #[tokio::test]
    // The guard only serialises tests against each other; nothing in the
    // worker waits on it, so holding it across an await cannot deadlock.
    #[allow(clippy::await_holding_lock)]
    async fn a_password_that_is_not_stored_is_asked_for_then_used() {
        let _guard = secrets::test_lock();
        let _ = secrets::take_stash();

        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        let mut record = sqlite("prompt", ":memory:");
        record.secret = Secret::Prompt;
        let id = record.id.clone();
        db.send(DbCommand::Connect(Box::new(record))).unwrap();

        // It asks instead of connecting, and the request names the
        // connection but carries no password.
        assert_eq!(
            action_rx.recv().await,
            Some(Action::NeedPassword { id, name: "prompt".into() })
        );

        // The prompt leaves the password in the stash, not in the action.
        secrets::stash(Password::new("hunter2"));
        db.send(DbCommand::PasswordEntered).unwrap();
        assert_eq!(action_rx.recv().await, Some(Action::Connected(":memory:".into())));
        assert_eq!(secrets::take_stash(), None, "the stash is emptied by the worker");
    }

    #[tokio::test]
    async fn a_failed_connection_is_reported_for_the_modal_and_the_status_bar() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        // A directory is not a database file.
        let mut record = sqlite("", &std::env::temp_dir().display().to_string());
        record.mode = Some("ro".into());
        db.send(DbCommand::Connect(Box::new(record))).unwrap();

        let Some(Action::ConnectFailed(message)) = action_rx.recv().await else {
            panic!("expected ConnectFailed first, for the modal");
        };
        assert!(!message.is_empty());
        assert!(matches!(
            action_rx.recv().await,
            Some(Action::Status(StatusCode::Error(_)))
        ));
    }

    #[tokio::test]
    async fn query_result_comes_back_as_actions() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        db.send(in_memory()).unwrap();
        db.send(DbCommand::Query("SELECT 1 AS one".into())).unwrap();

        assert_eq!(action_rx.recv().await, Some(Action::Connected(":memory:".into())));
        assert_eq!(action_rx.recv().await, Some(success("Connected to :memory:".into())));
        let Some(Action::QueryDone(result)) = action_rx.recv().await else {
            panic!("expected QueryDone");
        };
        assert_eq!(result.columns, ["one"]);
        let Some(Action::Status(StatusCode::Success(message))) = action_rx.recv().await else {
            panic!("expected a status message");
        };
        assert!(message.starts_with("Fetched 1 row in "), "{message}");
        assert!(message.ends_with("ms") || message.ends_with('s'), "{message}");
    }

    #[tokio::test]
    async fn failures_become_error_status() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        db.send(DbCommand::Query("SELECT 1".into())).unwrap();
        assert_eq!(
            action_rx.recv().await,
            Some(error("not connected to a database".into()))
        );

        db.send(in_memory()).unwrap();
        action_rx.recv().await; // Connected
        action_rx.recv().await; // its status message
        db.send(DbCommand::Query("SELEC nonsense".into())).unwrap();
        assert!(matches!(
            action_rx.recv().await,
            Some(Action::Status(StatusCode::Error(_)))
        ));
    }

    fn temp_csv() -> String {
        let name = format!("qry-worker-{}.csv", std::process::id());
        std::env::temp_dir().join(name).display().to_string()
    }

    #[tokio::test]
    async fn export_writes_the_last_result() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        let path = temp_csv();

        // Nothing has run yet, so there is nothing to export.
        db.send(DbCommand::Export(ExportConfig {
            path: path.clone(),
            separator: ",".into(),
            etype: ExportType::Csv,
        }))
        .unwrap();
        assert_eq!(
            action_rx.recv().await,
            Some(Action::ExportFailed("Nothing to export yet: run a query first".into()))
        );
        assert_eq!(
            action_rx.recv().await,
            Some(error("Nothing to export yet: run a query first".into()))
        );

        db.send(in_memory()).unwrap();
        db.send(DbCommand::Query(
            "WITH t(id, name) AS (VALUES (1, 'alice'), (2, NULL)) SELECT * FROM t".into(),
        ))
        .unwrap();
        db.send(DbCommand::Export(ExportConfig {
            path: path.clone(),
            separator: ",".into(),
            etype: ExportType::Csv,
        }))
        .unwrap();

        // Connected, its status, QueryDone, its status, Exported, its status.
        let mut messages = Vec::new();
        while messages.len() < 6 {
            messages.push(action_rx.recv().await.unwrap());
        }
        assert_eq!(
            messages.last(),
            Some(&success(format!("Exported 2 rows to {path}")))
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "id,name\n1,alice\n2,\n"
        );
        let _ = std::fs::remove_file(&path);
    }
}
