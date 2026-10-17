//! Unit tests for `runtime.rs`: the executor really does the I/O the
//! reducer asks for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use super::*;
use crate::notice::Level;
use crate::{
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
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
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
    let (mut executor, _answers, mut events) = Executor::new(blocked, 0);
    finished(executor.execute(Effect::SaveConnections(Vec::new()))).await;
    let Ok(UiEvent::File(FileAnswer::SaveFailed(message))) = events.try_recv() else {
        panic!("expected SaveFailed");
    };
    assert!(message.starts_with("cannot save connections"), "{message}");
}

#[tokio::test]
async fn history_and_snippets_are_written_and_read_off_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::history_store::MemoryStore::default();
    let (executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
    let mut executor = executor
        .with_config_dir(dir.path().join("config"))
        .with_history(Arc::new(store.clone()));
    let line = "{\"sql\":\"SELECT 1\",\"ok\":true,\"ms\":1}\n".to_string();
    finished(executor.execute(Effect::History(HistoryWrite::Append {
        connection: "c1".into(),
        line: line.clone(),
    })))
    .await;
    assert_eq!(store.file("c1"), Some(line));
    finished(executor.execute(Effect::LoadHistory)).await;
    let Ok(UiEvent::File(FileAnswer::HistoryLoaded(loaded))) = events.try_recv() else {
        panic!("expected HistoryLoaded");
    };
    assert_eq!(loaded[0].connection, "c1");
    assert_eq!(loaded[0].history.len(), 1);

    let snippet = qry_core::snippets::Snippet {
        name: "one".into(),
        sql: "SELECT 1".into(),
        connection: None,
        tags: Vec::new(),
    };
    finished(executor.execute(Effect::SaveSnippets(vec![snippet.clone()]))).await;
    assert!(dir.path().join("config/snippets.json").exists());
    finished(executor.execute(Effect::LoadSnippets)).await;
    assert_eq!(
        events.try_recv().unwrap(),
        UiEvent::File(FileAnswer::SnippetsLoaded(vec![snippet]))
    );
}

#[tokio::test]
async fn loading_reads_the_saved_connections() {
    let dir = tempfile::tempdir().unwrap();
    let saved = sqlite("app", "app.db");
    connections::save_to(dir.path(), std::slice::from_ref(&saved)).unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);

    finished(executor.execute(Effect::LoadConnections)).await;
    assert_eq!(
        events.try_recv().unwrap(),
        UiEvent::File(FileAnswer::ConnectionsLoaded(vec![saved]))
    );
}

#[tokio::test]
async fn db_worker_answers_with_db_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, mut answers, _events) = Executor::new(dir.path().to_path_buf(), 0);
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
        Some((
            _,
            DbAnswer::Notice(Notice {
                level: Level::Success,
                ..
            })
        ))
    ));
    let Some((_, DbAnswer::QueryDone(result))) = answers.recv().await else {
        panic!("expected QueryDone");
    };
    assert_eq!(result.column_names(), ["one"]);
}

#[tokio::test]
async fn a_closed_session_reports_its_worker_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
    let id = SessionId(2);
    executor.execute(Effect::OpenSession(id, Box::new(sqlite("", ":memory:"))));
    assert!(executor.sessions.contains(id));

    executor.execute(Effect::CloseSession(id));
    assert!(!executor.sessions.contains(id));
    executor.execute(Effect::Db(id, DbCommand::Query("SELECT 1".into())));
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
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
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

#[tokio::test]
async fn test_connection_reports_server_version() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
    let record = sqlite("mem", ":memory:");
    finished(executor.execute(Effect::TestConnection {
        test: 7,
        record: Box::new(record),
    }))
    .await;
    let Ok(UiEvent::ConnectionTested { test, outcome }) = events.try_recv() else {
        panic!("expected a test answer");
    };
    assert_eq!(test, 7);
    let tested = outcome.unwrap();
    assert!(tested.version.starts_with("SQLite 3."), "{tested:?}");
    assert!(
        !executor.sessions.contains(SessionId(1)),
        "no session was opened"
    );
}

#[tokio::test]
async fn a_test_takes_its_password_from_the_stash_and_leaves_none_behind() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
    let mut record = StoredConnection::new("pg", Driver::Postgres);
    record.host = Some("127.0.0.1".into());
    record.port = Some(1);
    record.user = Some("alice".into());
    record.database = Some("app".into());
    record.secret = Secret::Prompt;
    let key = crate::app::effects::test_stash_key(&record.id);
    let id = record.id.clone();
    executor.execute(Effect::StashPassword {
        connection: key.clone(),
        password: crate::secrets::Password::new("CANARY-PW-test"),
    });
    finished(executor.execute(Effect::TestConnection {
        test: 1,
        record: Box::new(record),
    }))
    .await;
    assert!(
        crate::secrets::take_stash(&key).is_none(),
        "taken and dropped"
    );
    assert!(crate::secrets::take_stash(&id).is_none());
    let Ok(UiEvent::ConnectionTested { outcome, .. }) = events.try_recv() else {
        panic!("expected a test answer");
    };
    let message = outcome.unwrap_err();
    assert!(!message.contains("CANARY"), "{message}");
}

#[tokio::test]
async fn the_export_preview_looks_at_the_file_off_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
    std::fs::write(dir.path().join("orders.csv"), "old").unwrap();
    let input = dir.path().join("orders").display().to_string();

    finished(executor.execute(Effect::ProbeExportPath {
        input: input.clone(),
        etype: qry_core::ExportType::Csv,
    }))
    .await;
    let UiEvent::File(FileAnswer::ExportProbed(probe)) = events.try_recv().unwrap() else {
        panic!("expected a probe");
    };
    assert_eq!(probe.input, input);
    assert_eq!(probe.resolved, format!("{input}.csv"));
    assert!(probe.exists && !probe.is_dir);

    // `~/` is the home directory; a folder says so.
    let home = dir.path();
    let probe = probe_export_path("~/".into(), qry_core::ExportType::Json, home);
    assert_eq!(probe.resolved, home.join(".json").display().to_string());
    std::fs::create_dir(dir.path().join("sub.csv")).unwrap();
    let probe = probe_export_path("~/sub".into(), qry_core::ExportType::Csv, home);
    assert!(probe.exists && probe.is_dir);
}

#[tokio::test]
async fn an_export_over_an_existing_file_waits_to_be_told() {
    let dir = tempfile::tempdir().unwrap();
    let (mut executor, _answers, mut events) = Executor::new(dir.path().to_path_buf(), 0);
    let target = dir.path().join("backup.json");
    connections::export(&target, &[]).unwrap();
    let path = target.display().to_string();
    let saved = sqlite("app", "app.db");

    finished(executor.execute(Effect::ExportConnections {
        path: path.clone(),
        connections: vec![saved.clone()],
        overwrite: false,
    }))
    .await;
    assert_eq!(
        events.try_recv().unwrap(),
        UiEvent::File(FileAnswer::ConnectionsExportExists(path.clone()))
    );
    assert!(
        connections::parse(&std::fs::read_to_string(&target).unwrap(), &target)
            .unwrap()
            .is_empty()
    );

    finished(executor.execute(Effect::ExportConnections {
        path: path.clone(),
        connections: vec![saved.clone()],
        overwrite: true,
    }))
    .await;
    assert!(matches!(
        events.try_recv().unwrap(),
        UiEvent::File(FileAnswer::ConnectionsExported { count: 1, .. })
    ));
    assert_eq!(
        connections::parse(&std::fs::read_to_string(&target).unwrap(), &target).unwrap(),
        [saved]
    );
}
