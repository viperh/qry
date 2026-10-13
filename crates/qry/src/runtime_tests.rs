//! Unit tests for `runtime.rs`: the executor really does the I/O the
//! reducer asks for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use super::*;
use crate::{
    app::StatusCode,
    connections::{Driver, Secret},
    db::DbCommand,
};

fn sqlite(name: &str, path: &str) -> StoredConnection {
    let mut record = StoredConnection::new(name, Driver::Sqlite);
    record.path = Some(path.to_string());
    record.secret = Secret::None;
    record
}

/// Waits for a task the executor spawned.
async fn finished(executed: Executed) {
    let Executed::Spawned(task) = executed else {
        panic!("expected a task off the loop, got {executed:?}");
    };
    task.await.unwrap();
}

#[tokio::test]
async fn runtime_executes_save_off_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf());
    let saved = sqlite("app", "app.db");

    finished(executor.execute(Effect::SaveConnections(vec![saved.clone()]))).await;
    assert_eq!(connections::load_from(dir.path()).unwrap(), [saved]);
    assert!(
        events.try_recv().is_err(),
        "a save that worked says nothing"
    );

    // A file where the directory should be makes the save fail.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "").unwrap();
    let (mut executor, _answers, mut events) = Executor::new(blocked);
    finished(executor.execute(Effect::SaveConnections(Vec::new()))).await;
    let Ok(UiEvent::File(FileAnswer::SaveFailed(message))) = events.try_recv() else {
        panic!("expected SaveFailed");
    };
    assert!(message.starts_with("cannot save connections"), "{message}");
}

#[tokio::test]
async fn loading_reads_the_saved_connections() {
    let dir = tempfile::tempdir().unwrap();
    let saved = sqlite("app", "app.db");
    connections::save_to(dir.path(), std::slice::from_ref(&saved)).unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf());

    finished(executor.execute(Effect::LoadConnections)).await;
    assert_eq!(
        events.try_recv().unwrap(),
        UiEvent::File(FileAnswer::ConnectionsLoaded(vec![saved]))
    );
}

#[tokio::test]
async fn db_worker_answers_with_db_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, mut answers, _events) = Executor::new(dir.path().to_path_buf());
    let mut scratch = sqlite("", ":memory:");
    scratch.ephemeral = true;

    let id = SessionId::FIRST;
    assert!(matches!(
        executor.execute(Effect::OpenSession(id, Box::new(scratch))),
        Executed::Done
    ));
    executor.execute(Effect::Db(id, DbCommand::Query("SELECT 1 AS one".into())));

    assert_eq!(
        answers.recv().await,
        Some((id, DbAnswer::Connected(":memory:".into())))
    );
    assert!(matches!(
        answers.recv().await,
        Some((_, DbAnswer::Status(StatusCode::Success(_))))
    ));
    let Some((_, DbAnswer::QueryDone(result))) = answers.recv().await else {
        panic!("expected QueryDone");
    };
    assert_eq!(result.columns, ["one"]);
}

#[tokio::test]
async fn a_closed_session_reports_its_worker_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf());
    let id = SessionId(2);
    executor.execute(Effect::OpenSession(id, Box::new(sqlite("", ":memory:"))));
    assert!(executor.sessions.contains(id));

    executor.execute(Effect::CloseSession(id));
    assert!(!executor.sessions.contains(id));
    executor.execute(Effect::Db(id, DbCommand::ListTables));
    assert_eq!(
        events.try_recv().unwrap(),
        UiEvent::Db(id, DbAnswer::WorkerStopped)
    );
}

#[test]
fn query_results_are_never_logged() {
    let result = crate::testing::grid(3, 2);
    let done = UiEvent::Db(SessionId::FIRST, DbAnswer::QueryDone(Arc::new(result)));
    assert_eq!(log_line(&done), None);
    // Even printed, a result shows only its size.
    let shown = format!("{done:?}");
    assert!(!shown.contains("r0c0"), "{shown}");
    assert!(shown.contains("3 rows × 2 columns"), "{shown}");

    let connected = UiEvent::Db(SessionId::FIRST, DbAnswer::Connected("x".into()));
    assert!(log_line(&connected).is_some());
}

#[tokio::test(start_paused = true)]
async fn a_scheduled_timer_comes_back_as_an_event_once_it_runs_out() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf());
    let timer = crate::app::Timer::KeySequence(7);
    let start = tokio::time::Instant::now();

    finished(executor.execute(Effect::ScheduleTimer {
        timer,
        after: std::time::Duration::from_secs(1),
    }))
    .await;
    assert_eq!(start.elapsed(), std::time::Duration::from_secs(1));
    assert_eq!(events.try_recv().unwrap(), UiEvent::Timer(timer));
}
