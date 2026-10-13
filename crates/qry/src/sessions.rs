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
    db::{self, Answers, DbCommand},
};

/// The session has no worker: it was closed, never opened, or its worker
/// has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkerGone;

/// The workers of the open sessions, by session.
#[derive(Debug, Default)]
pub(crate) struct SessionManager {
    workers: HashMap<SessionId, UnboundedSender<DbCommand>>,
}

impl SessionManager {
    /// Starts a worker for session `id` and has it open `record`. Its
    /// answers arrive on `answers`, tagged with `id`. A session that is
    /// already open gets a fresh worker; the old one ends.
    pub(crate) fn open(
        &mut self,
        id: SessionId,
        record: StoredConnection,
        answers: Answers,
    ) -> Result<(), WorkerGone> {
        let worker = db::spawn(id, answers);
        let sent = worker.send(DbCommand::Connect(Box::new(record)));
        self.workers.insert(id, worker);
        sent.map_err(|_| WorkerGone)
    }

    /// Hands `command` to session `id`'s worker.
    pub(crate) fn send(&self, id: SessionId, command: DbCommand) -> Result<(), WorkerGone> {
        let worker = self.workers.get(&id).ok_or(WorkerGone)?;
        worker.send(command).map_err(|_| WorkerGone)
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
