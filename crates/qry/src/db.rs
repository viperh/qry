//! Background task that owns the database connection.
//!
//! The event loop never awaits a query. It sends a [`DbCommand`] here, and the
//! outcome comes back later as an [`Action`] on the normal action channel.

use std::sync::Arc;

use qry_core::{ConnectionConfig, Driver, ExportConfig, Exporter, QueryResult};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::action::{Action, StatusCode};

pub enum DbCommand {
    /// A display name (may be empty) and how to reach the database.
    Connect(String, ConnectionConfig),
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
            DbCommand::Connect(name, config) => match self.driver.connect(config).await {
                Ok(label) => {
                    let shown = if name.is_empty() {
                        label.clone()
                    } else {
                        format!("{name} ({label})")
                    };
                    vec![Action::Connected(label), success(format!("Connected to {shown}"))]
                }
                Err(e) => vec![
                    // The modal shows the top of the chain, which fits its
                    // bottom border; the status bar gets the whole of it.
                    Action::ConnectFailed(e.to_string()),
                    error(format!("Connection failed: {e:#}")),
                ],
            },
            DbCommand::Query(sql) => match self.driver.query(&sql).await {
                Ok(result) => {
                    let summary = match result.rows.len() {
                        _ if result.columns.is_empty() => "Statement executed".to_string(),
                        1 => "1 row".to_string(),
                        n => format!("{n} rows"),
                    };
                    let result = Arc::new(result);
                    self.last = Some(result.clone());
                    vec![Action::QueryDone(result), success(summary)]
                }
                Err(e) => vec![error(format!("{e:#}"))],
            },
            DbCommand::Export(config) => self.export(config).await,
            DbCommand::ListTables => match self.driver.tables().await {
                Ok(tables) => vec![Action::TablesLoaded(tables)],
                Err(e) => vec![Action::TablesFailed(e.to_string()), error(format!("{e:#}"))],
            },
        }
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

fn success(message: String) -> Action {
    Action::Status(StatusCode::Success(message))
}

fn error(message: String) -> Action {
    Action::Status(StatusCode::Error(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qry_core::{ExportType, sqlite::SqliteConfig};

    fn in_memory() -> DbCommand {
        DbCommand::Connect(
            String::new(),
            ConnectionConfig::Sqlite(SqliteConfig::new(":memory:", false)),
        )
    }

    #[tokio::test]
    async fn named_connection_shows_its_name() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        db.send(DbCommand::Connect(
            "scratch".into(),
            ConnectionConfig::Sqlite(SqliteConfig::new(":memory:", false)),
        ))
        .unwrap();
        assert_eq!(action_rx.recv().await, Some(Action::Connected(":memory:".into())));
        assert_eq!(
            action_rx.recv().await,
            Some(success("Connected to scratch (:memory:)".into()))
        );
    }

    #[tokio::test]
    async fn a_failed_connection_is_reported_for_the_modal_and_the_status_bar() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        db.send(DbCommand::Connect(
            String::new(),
            // A directory is not a database file.
            ConnectionConfig::Sqlite(SqliteConfig::new(std::env::temp_dir(), true)),
        ))
        .unwrap();

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
        assert_eq!(action_rx.recv().await, Some(success("1 row".into())));
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
