//! Unit tests for `db.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{
    DatabaseType, ExportType,
    testing::{FakeDatabase, Reply, grid, result},
};

use super::*;
use crate::connections::Driver as StoredDriver;

/// A SQLite connection that needs no password.
fn sqlite(name: &str, path: &str) -> StoredConnection {
    let mut record = StoredConnection::new(name, StoredDriver::Sqlite);
    record.path = Some(path.to_string());
    record.secret = Secret::None;
    record
}

/// A worker already connected to `db`, running on its own task.
fn spawn_fake(db: &FakeDatabase, action_tx: UnboundedSender<Action>) -> UnboundedSender<DbCommand> {
    let worker = Worker {
        driver: Driver::with_database(Box::new(db.clone())),
        ..Worker::default()
    };
    spawn_worker(worker, action_tx)
}

fn in_memory() -> DbCommand {
    DbCommand::Connect(Box::new(sqlite("", ":memory:")))
}

#[tokio::test]
async fn named_connection_shows_its_name() {
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let db = spawn(action_tx);
    db.send(DbCommand::Connect(Box::new(sqlite("scratch", ":memory:"))))
        .unwrap();
    assert_eq!(
        action_rx.recv().await,
        Some(Action::Connected(":memory:".into()))
    );
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
    secrets::set_test_var("QRY_TEST_DB_PASSWORD", "hunter2");

    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let db = spawn(action_tx);
    let mut record = sqlite("env", ":memory:");
    record.secret = Secret::Env {
        var: "QRY_TEST_DB_PASSWORD".into(),
    };
    db.send(DbCommand::Connect(Box::new(record))).unwrap();

    assert_eq!(
        action_rx.recv().await,
        Some(Action::Connected(":memory:".into()))
    );
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
        Some(Action::NeedPassword {
            id,
            name: "prompt".into()
        })
    );

    // The prompt leaves the password in the stash, not in the action.
    secrets::stash(Password::new("hunter2"));
    db.send(DbCommand::PasswordEntered).unwrap();
    assert_eq!(
        action_rx.recv().await,
        Some(Action::Connected(":memory:".into()))
    );
    assert_eq!(
        secrets::take_stash(),
        None,
        "the stash is emptied by the worker"
    );
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

    assert_eq!(
        action_rx.recv().await,
        Some(Action::Connected(":memory:".into()))
    );
    assert_eq!(
        action_rx.recv().await,
        Some(success("Connected to :memory:".into()))
    );
    let Some(Action::QueryDone(result)) = action_rx.recv().await else {
        panic!("expected QueryDone");
    };
    assert_eq!(result.columns, ["one"]);
    let Some(Action::Status(StatusCode::Success(message))) = action_rx.recv().await else {
        panic!("expected a status message");
    };
    assert!(message.starts_with("Fetched 1 row in "), "{message}");
    assert!(
        message.ends_with("ms") || message.ends_with('s'),
        "{message}"
    );
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

#[tokio::test]
async fn export_writes_the_last_result() {
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let db = spawn(action_tx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.csv").display().to_string();

    // Nothing has run yet, so there is nothing to export.
    db.send(DbCommand::Export(ExportConfig {
        path: path.clone(),
        separator: ",".into(),
        etype: ExportType::Csv,
    }))
    .unwrap();
    assert_eq!(
        action_rx.recv().await,
        Some(Action::ExportFailed(
            "Nothing to export yet: run a query first".into()
        ))
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
}

#[tokio::test]
async fn a_driver_error_comes_back_as_a_status_error() {
    let db = FakeDatabase::new(DatabaseType::Postgres)
        .reply(Reply::Error("relation \"nope\" does not exist".into()));
    let mut worker = Worker {
        driver: Driver::with_database(Box::new(db.clone())),
        ..Worker::default()
    };

    let actions = worker
        .run(DbCommand::Query("SELECT * FROM nope".into()))
        .await;
    assert_eq!(actions, [error("relation \"nope\" does not exist".into())]);
    assert!(
        worker.last.is_none(),
        "a failed query leaves nothing to export"
    );
    assert_eq!(db.calls(), ["SELECT * FROM nope"]);
}

#[tokio::test(start_paused = true)]
async fn commands_run_in_order_even_when_the_first_is_slow() {
    let slow = result(&["slow"], &[&[Some("1")]]);
    let fast = grid(2, 2);
    let db = FakeDatabase::default()
        .reply(Reply::Delay(
            Duration::from_secs(5),
            Box::new(Reply::Rows(slow.clone())),
        ))
        .reply(Reply::Rows(fast.clone()));
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let worker = spawn_fake(&db, action_tx);

    worker.send(DbCommand::Query("SELECT slow".into())).unwrap();
    worker.send(DbCommand::Query("SELECT fast".into())).unwrap();

    // The paused clock jumps over the delay as soon as every task is idle,
    // so this waits on virtual time only.
    let mut done = Vec::new();
    while done.len() < 2 {
        if let Some(Action::QueryDone(result)) = action_rx.recv().await {
            done.push((*result).clone());
        }
    }
    assert_eq!(done, [slow, fast]);
    assert_eq!(db.calls(), ["SELECT slow", "SELECT fast"]);
}
