//! Unit tests for `db.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{
    DatabaseType, ExportType,
    testing::{FakeDatabase, Reply, grid, result},
};

use super::*;
use crate::connections::Driver as StoredDriver;
use crate::notice::Level;

/// A SQLite connection that needs no password.
fn sqlite(name: &str, path: &str) -> StoredConnection {
    let mut record = StoredConnection::new(name, StoredDriver::Sqlite);
    record.path = Some(path.to_string());
    record.secret = Secret::None;
    record
}

/// A worker already connected to `db`, running on its own task.
fn spawn_fake(db: &FakeDatabase, answers: Answers) -> UnboundedSender<DbCommand> {
    let worker = Worker {
        driver: Driver::with_database(Box::new(db.clone())),
        ..Worker::default()
    };
    spawn_worker(worker, SessionId::FIRST, answers)
}

/// A worker for the first session, and where its answers arrive.
fn spawn_first() -> (UnboundedSender<DbCommand>, Receiver) {
    let (answers, rx) = mpsc::unbounded_channel();
    (spawn(SessionId::FIRST, answers, 0).0, Receiver(rx))
}

/// The answers of the first session's worker, without the session's id.
struct Receiver(mpsc::UnboundedReceiver<(SessionId, DbAnswer)>);

impl Receiver {
    async fn recv(&mut self) -> Option<DbAnswer> {
        let (id, answer) = self.0.recv().await?;
        assert_eq!(id, SessionId::FIRST, "answered for another session");
        Some(answer)
    }
}

fn in_memory() -> DbCommand {
    DbCommand::Connect(Box::new(sqlite("", ":memory:")))
}

#[tokio::test]
async fn named_connection_shows_its_name() {
    let (db, mut answers) = spawn_first();
    db.send(DbCommand::Connect(Box::new(sqlite("scratch", ":memory:"))))
        .unwrap();
    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::Connected(":memory:".into()))
    );
    assert_eq!(
        answers.recv().await,
        Some(success("Connected to scratch (:memory:)".into()))
    );
}

#[tokio::test]
// The guard only serialises tests against each other; nothing in the
// worker waits on it, so holding it across an await cannot deadlock.
#[allow(clippy::await_holding_lock)]
async fn a_password_in_the_environment_is_used_without_asking() {
    let _guard = secrets::test_lock();
    secrets::set_test_var("QRY_TEST_DB_PASSWORD", "hunter2");

    let (db, mut answers) = spawn_first();
    let mut record = sqlite("env", ":memory:");
    record.secret = Secret::Env {
        var: "QRY_TEST_DB_PASSWORD".into(),
    };
    db.send(DbCommand::Connect(Box::new(record))).unwrap();

    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::Connected(":memory:".into()))
    );
}

#[tokio::test]
// The guard only serialises tests against each other; nothing in the
// worker waits on it, so holding it across an await cannot deadlock.
#[allow(clippy::await_holding_lock)]
async fn a_password_that_is_not_stored_is_asked_for_then_used() {
    let _guard = secrets::test_lock();

    let (db, mut answers) = spawn_first();
    let mut record = sqlite("prompt", ":memory:");
    record.secret = Secret::Prompt;
    let id = record.id.clone();
    db.send(DbCommand::Connect(Box::new(record))).unwrap();

    // It asks instead of connecting, and the request names the
    // connection but carries no password.
    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::NeedPassword {
            id: id.clone(),
            name: "prompt".into()
        })
    );

    // The prompt leaves the password in the stash, not in the command.
    secrets::stash(&id, Password::new("hunter2"));
    db.send(DbCommand::PasswordEntered).unwrap();
    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::Connected(":memory:".into()))
    );
    assert_eq!(
        secrets::take_stash(&id),
        None,
        "the stash is emptied by the worker"
    );
}

#[tokio::test]
async fn a_failed_connection_is_reported_for_the_modal_and_the_status_bar() {
    let (db, mut answers) = spawn_first();
    // A directory is not a database file.
    let mut record = sqlite("", &std::env::temp_dir().display().to_string());
    record.mode = Some("ro".into());
    db.send(DbCommand::Connect(Box::new(record))).unwrap();

    let Some(DbAnswer::ConnectFailed(message)) = answers.recv().await else {
        panic!("expected ConnectFailed first, for the modal");
    };
    assert!(!message.is_empty());
    assert!(matches!(
        answers.recv().await,
        Some(DbAnswer::Notice(Notice {
            level: Level::Error,
            ..
        }))
    ));
}

#[tokio::test]
async fn query_result_comes_back_as_db_answers() {
    let (db, mut answers) = spawn_first();
    db.send(in_memory()).unwrap();
    db.send(DbCommand::Query("SELECT 1 AS one".into())).unwrap();

    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::Connected(":memory:".into()))
    );
    assert_eq!(
        answers.recv().await,
        Some(success("Connected to :memory:".into()))
    );
    let Some(DbAnswer::QueryDone(result)) = answers.recv().await else {
        panic!("expected QueryDone");
    };
    assert_eq!(result.column_names(), ["one"]);
    let Some(DbAnswer::Notice(Notice {
        level: Level::Success,
        text: message,
    })) = answers.recv().await
    else {
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
    let (db, mut answers) = spawn_first();
    db.send(DbCommand::Query("SELECT 1".into())).unwrap();
    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::QueryFailed {
            message: "not connected to a database".into(),
            position: None
        })
    );
    assert_eq!(
        answers.recv().await,
        Some(error("not connected to a database".into()))
    );

    db.send(in_memory()).unwrap();
    answers.recv().await; // Connected
    answers.recv().await; // its status message
    db.send(DbCommand::Query("SELEC nonsense".into())).unwrap();
    assert!(matches!(
        answers.recv().await,
        Some(DbAnswer::QueryFailed { .. })
    ));
    assert!(matches!(
        answers.recv().await,
        Some(DbAnswer::Notice(Notice {
            level: Level::Error,
            ..
        }))
    ));
}

/// An export of `source` to `path`, as the dialog sends it.
fn export(source: ExportSource, path: &str, overwrite: bool) -> DbCommand {
    DbCommand::Export(Box::new(ExportJob {
        job: None,
        source,
        path: path.into(),
        etype: ExportType::Csv,
        options: ExportOptions::default(),
        overwrite,
    }))
}

#[tokio::test]
async fn export_writes_the_fetched_rows() {
    let (db, mut answers) = spawn_first();
    let dir = tempfile::tempdir().unwrap();
    // The extension is added.
    let path = dir.path().join("rows").display().to_string();
    let rows = Arc::new(result(
        &["id", "name"],
        &[&[Some("1"), Some("alice")], &[Some("2"), None]],
    ));

    db.send(export(ExportSource::Fetched(rows.clone()), &path, false))
        .unwrap();
    let written = format!("{path}.csv");
    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::Exported {
            job: None,
            rows: 2,
            path: written.clone(),
            warning: None,
        })
    );
    assert_eq!(
        std::fs::read_to_string(&written).unwrap(),
        "id,name\n1,alice\n2,\n"
    );

    // It is there now: left alone until the user says to replace it.
    db.send(export(ExportSource::Fetched(rows.clone()), &path, false))
        .unwrap();
    assert_eq!(
        answers.recv().await,
        Some(DbAnswer::ExportExists {
            job: None,
            path: written.clone(),
        })
    );
    db.send(export(ExportSource::Fetched(rows), &path, true))
        .unwrap();
    assert!(matches!(
        answers.recv().await,
        Some(DbAnswer::Exported { rows: 2, .. })
    ));
}

#[tokio::test]
async fn a_failed_export_names_the_path() {
    let (db, mut answers) = spawn_first();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no/such/rows.csv").display().to_string();
    let rows = Arc::new(grid(3, 1));
    db.send(export(ExportSource::Fetched(rows), &path, false))
        .unwrap();
    let Some(DbAnswer::ExportFailed {
        job: None,
        rows: 3,
        path: shown,
        message,
    }) = answers.recv().await
    else {
        panic!("expected a failure");
    };
    assert_eq!(shown, path);
    assert!(
        message.contains("could not write") && message.contains("does not exist"),
        "{message}"
    );
}

#[tokio::test]
async fn an_export_of_every_row_streams_with_progress() {
    let (db, mut answers) = spawn_first();
    db.send(in_memory()).unwrap();
    answers.recv().await; // Connected
    answers.recv().await; // its status message
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("all.csv").display().to_string();
    let sql = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 2500) \
               SELECT n FROM s";
    db.send(DbCommand::Export(Box::new(ExportJob {
        job: Some(JobId(7)),
        source: ExportSource::Rerun(sql.into()),
        path: path.clone(),
        etype: ExportType::Csv,
        options: ExportOptions::default(),
        overwrite: false,
    })))
    .unwrap();
    let mut progress = Vec::new();
    loop {
        match answers.recv().await.unwrap() {
            DbAnswer::ExportProgress { job, rows } => {
                assert_eq!(job, JobId(7));
                progress.push(rows);
            }
            answer => {
                assert_eq!(
                    answer,
                    DbAnswer::Exported {
                        job: Some(JobId(7)),
                        rows: 2500,
                        path: path.clone(),
                        warning: None,
                    }
                );
                break;
            }
        }
    }
    assert_eq!(progress, [1000, 2000, 2500]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2501);
}

#[tokio::test]
async fn a_driver_error_comes_back_as_a_status_error() {
    let db = FakeDatabase::new(DatabaseType::Postgres)
        .reply(Reply::Error("relation \"nope\" does not exist".into()));
    let mut worker = Worker {
        driver: Driver::with_database(Box::new(db.clone())),
        ..Worker::default()
    };

    let answers = worker
        .run(DbCommand::Query("SELECT * FROM nope".into()))
        .await;
    assert_eq!(
        answers,
        [
            DbAnswer::QueryFailed {
                message: "relation \"nope\" does not exist".into(),
                position: None
            },
            error("relation \"nope\" does not exist".into())
        ]
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
    let (tx, rx) = mpsc::unbounded_channel();
    let mut answers = Receiver(rx);
    let worker = spawn_fake(&db, tx);

    worker.send(DbCommand::Query("SELECT slow".into())).unwrap();
    worker.send(DbCommand::Query("SELECT fast".into())).unwrap();

    // The paused clock jumps over the delay as soon as every task is idle,
    // so this waits on virtual time only.
    let mut done = Vec::new();
    while done.len() < 2 {
        if let Some(DbAnswer::QueryDone(result)) = answers.recv().await {
            let mut result = (*result).clone();
            // How long it took is measured on the real clock.
            result.elapsed = Duration::ZERO;
            done.push(result);
        }
    }
    assert_eq!(done, [slow, fast]);
    assert_eq!(db.calls(), ["SELECT slow", "SELECT fast"]);
}

#[test]
fn a_finished_query_carries_how_long_it_took() {
    let answers = query_answer::<String>(Ok(grid(1, 1)), Duration::from_millis(12), None);
    let DbAnswer::QueryDone(result) = &answers[0] else {
        panic!("expected QueryDone first");
    };
    assert_eq!(result.elapsed, Duration::from_millis(12));
}
