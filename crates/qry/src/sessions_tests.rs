//! Unit tests for `sessions.rs`: real workers on in-memory SQLite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::*;
use crate::{
    app::DbAnswer,
    connections::{Driver, Secret},
};

fn memory(name: &str) -> StoredConnection {
    let mut record = StoredConnection::new(name, Driver::Sqlite);
    record.path = Some(":memory:".into());
    record.secret = Secret::None;
    record
}

/// The next answer, which must come from `id`, skipping the status lines
/// (from any session).
async fn next(answers: &mut UnboundedReceiver<(SessionId, DbAnswer)>, id: SessionId) -> DbAnswer {
    loop {
        let (from, answer) = answers.recv().await.expect("a worker answers");
        if !matches!(answer, DbAnswer::Notice(_)) {
            assert_eq!(from, id, "an answer from the wrong session: {answer:?}");
            return answer;
        }
    }
}

#[tokio::test]
async fn two_sqlite_sessions_are_independent() {
    let (tx, mut answers) = mpsc::unbounded_channel();
    let mut sessions = SessionManager::default();
    let (a, b) = (SessionId(1), SessionId(2));
    sessions.open(a, memory("a"), tx.clone()).unwrap();
    assert!(matches!(
        next(&mut answers, a).await,
        DbAnswer::Connected(_)
    ));
    sessions.open(b, memory("b"), tx).unwrap();
    assert!(matches!(
        next(&mut answers, b).await,
        DbAnswer::Connected(_)
    ));

    sessions
        .send(
            a,
            DbCommand::Query("CREATE TABLE only_in_a (x INTEGER)".into()),
        )
        .unwrap();
    assert!(matches!(
        next(&mut answers, a).await,
        DbAnswer::QueryDone(_)
    ));
    sessions.send(a, DbCommand::ListTables).unwrap();
    assert_eq!(
        next(&mut answers, a).await,
        DbAnswer::TablesLoaded(vec!["only_in_a".into()])
    );

    // The other :memory: database is another database.
    sessions.send(b, DbCommand::ListTables).unwrap();
    assert_eq!(
        next(&mut answers, b).await,
        DbAnswer::TablesLoaded(Vec::new())
    );
    sessions
        .send(b, DbCommand::Query("SELECT * FROM only_in_a".into()))
        .unwrap();
    assert!(matches!(
        next(&mut answers, b).await,
        DbAnswer::QueryFailed(_)
    ));
}

#[tokio::test]
async fn close_drops_worker() {
    let (tx, mut answers) = mpsc::unbounded_channel();
    let mut sessions = SessionManager::default();
    let id = SessionId(1);
    sessions.open(id, memory("a"), tx).unwrap();
    assert!(matches!(
        next(&mut answers, id).await,
        DbAnswer::Connected(_)
    ));
    assert!(sessions.contains(id));

    sessions.close(id);
    assert!(!sessions.contains(id));
    assert_eq!(sessions.send(id, DbCommand::ListTables), Err(WorkerGone));
    // The worker ends once its sender is gone, and with it the last sender
    // of the answers.
    while let Some((from, answer)) = answers.recv().await {
        assert_eq!(from, id, "{answer:?}");
    }
    assert_eq!(
        SessionManager::default().send(SessionId(9), DbCommand::ListTables),
        Err(WorkerGone),
        "never opened"
    );
}

#[tokio::test]
async fn answers_are_tagged_with_session_id() {
    let (tx, mut answers) = mpsc::unbounded_channel();
    let mut sessions = SessionManager::default();
    for n in 1..=3 {
        sessions
            .open(SessionId(n), memory("m"), tx.clone())
            .unwrap();
    }
    drop(tx);
    for n in 1..=3 {
        sessions
            .send(SessionId(n), DbCommand::Query(format!("SELECT {n} AS n")))
            .unwrap();
    }

    // Each session's result carries its own number and its own id.
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let (id, answer) = answers.recv().await.unwrap();
        if let DbAnswer::QueryDone(result) = answer {
            assert_eq!(
                result.rows[0][0].as_deref(),
                Some(id.0.to_string().as_str())
            );
            seen.push(id);
        }
    }
    seen.sort();
    assert_eq!(seen, [SessionId(1), SessionId(2), SessionId(3)]);
}
