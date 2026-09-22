//! Background task that owns the database connection.
//!
//! The event loop never awaits a query. It sends a [`DbCommand`] here, and the
//! outcome comes back later as an [`Action`] on the normal action channel.

use std::sync::Arc;

use qry_core::{ConnectionConfig, Driver};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::action::{Action, StatusCode};

pub enum DbCommand {
    /// A display name (may be empty) and how to reach the database.
    Connect(String, ConnectionConfig),
    Query(String),
}

/// Starts the worker and returns the sender used to reach it. Commands run one
/// at a time, in order. The worker stops once every sender has been dropped.
pub fn spawn(action_tx: UnboundedSender<Action>) -> UnboundedSender<DbCommand> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut driver = Driver::new();
        while let Some(command) = rx.recv().await {
            for action in run(&mut driver, command).await {
                if action_tx.send(action).is_err() {
                    return;
                }
            }
        }
    });
    tx
}

async fn run(driver: &mut Driver, command: DbCommand) -> Vec<Action> {
    match command {
        DbCommand::Connect(name, config) => match driver.connect(config).await {
            Ok(label) if name.is_empty() => vec![success(format!("Connected to {label}"))],
            Ok(label) => vec![success(format!("Connected to {name} ({label})"))],
            Err(e) => vec![error(format!("Connection failed: {e:#}"))],
        },
        DbCommand::Query(sql) => match driver.query(&sql).await {
            Ok(result) => {
                let summary = match result.rows.len() {
                    _ if result.columns.is_empty() => "Statement executed".to_string(),
                    1 => "1 row".to_string(),
                    n => format!("{n} rows"),
                };
                vec![Action::QueryDone(Arc::new(result)), success(summary)]
            }
            Err(e) => vec![error(format!("{e:#}"))],
        },
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
    use qry_core::sqlite::SqliteConfig;

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
        assert_eq!(
            action_rx.recv().await,
            Some(success("Connected to scratch (:memory:)".into()))
        );
    }

    #[tokio::test]
    async fn query_result_comes_back_as_actions() {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();
        let db = spawn(action_tx);
        db.send(in_memory()).unwrap();
        db.send(DbCommand::Query("SELECT 1 AS one".into())).unwrap();

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
        action_rx.recv().await;
        db.send(DbCommand::Query("SELEC nonsense".into())).unwrap();
        assert!(matches!(
            action_rx.recv().await,
            Some(Action::Status(StatusCode::Error(_)))
        ));
    }
}
