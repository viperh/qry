//! The open sessions' database workers, on the runtime's side.
//!
//! [`SessionManager`] is the only place that holds the workers' senders.
//! The reducer names a session by its [`SessionId`] and the
//! [executor](crate::runtime) looks the worker up here. Each session has a
//! worker of its own, so a query in one never waits for, or touches, the
//! driver of another.

use std::collections::HashMap;

use tokio::sync::mpsc::UnboundedSender;

use crate::{
    app::SessionId,
    connections::StoredConnection,
    db::{self, Answers, CancelSlot, DbCommand},
};

/// The session has no worker: it was closed, never opened, or its worker
/// has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkerGone;

/// The workers of the open sessions, by session.
#[derive(Default)]
pub(crate) struct SessionManager {
    workers: HashMap<SessionId, (UnboundedSender<DbCommand>, CancelSlot)>,
    /// The most rows a query keeps; 0 keeps all.
    max_rows: usize,
}

/// What [`SessionManager::cancel`] could do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelOutcome {
    /// The connection was asked to stop its statement.
    Asked,
    /// Its driver cannot stop a statement; it runs on.
    Unsupported,
    Gone,
}

/// The handles inside have no `Debug`; the sessions' ids say enough.
impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("sessions", &self.workers.keys().collect::<Vec<_>>())
            .field("max_rows", &self.max_rows)
            .finish()
    }
}

impl SessionManager {
    /// No session open yet; each query will keep at most `max_rows` rows.
    pub(crate) fn new(max_rows: usize) -> Self {
        Self {
            workers: HashMap::new(),
            max_rows,
        }
    }

    /// Starts a worker for session `id` and has it open `record`. Its
    /// answers arrive on `answers`, tagged with `id`. A session that is
    /// already open gets a fresh worker; the old one ends.
    pub(crate) fn open(
        &mut self,
        id: SessionId,
        record: StoredConnection,
        answers: Answers,
    ) -> Result<(), WorkerGone> {
        let (worker, cancel) = db::spawn(id, answers, self.max_rows);
        let sent = worker.send(DbCommand::Connect(Box::new(record)));
        self.workers.insert(id, (worker, cancel));
        sent.map_err(|_| WorkerGone)
    }

    /// Hands `command` to session `id`'s worker.
    pub(crate) fn send(&self, id: SessionId, command: DbCommand) -> Result<(), WorkerGone> {
        let (worker, _) = self.workers.get(&id).ok_or(WorkerGone)?;
        worker.send(command).map_err(|_| WorkerGone)
    }

    /// Asks session `id`'s connection to stop the statement it runs, on a
    /// task of its own: the worker is busy with that statement.
    pub(crate) fn cancel(&self, id: SessionId) -> CancelOutcome {
        let Some((_, slot)) = self.workers.get(&id) else {
            return CancelOutcome::Gone;
        };
        let Some(handle) = db::lock(slot).clone() else {
            return CancelOutcome::Unsupported;
        };
        tokio::spawn(async move {
            // A statement that already finished has nothing to stop.
            let _ = handle.cancel().await;
        });
        CancelOutcome::Asked
    }

    /// Drops the session's sender. The worker finishes the command it is
    /// running, if any, then ends, which closes its connection. Anything it
    /// still answers carries an id the app no longer knows, so it is
    /// dropped.
    pub(crate) fn close(&mut self, id: SessionId) {
        self.workers.remove(&id);
    }

    /// Whether session `id` has a worker.
    pub(crate) fn contains(&self, id: SessionId) -> bool {
        self.workers.contains_key(&id)
    }
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod sessions_tests;
