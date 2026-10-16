//! [`Effect`]: what the reducer asks the outside world to do.
//!
//! The reducer never performs I/O. It describes what should happen, and the
//! [runtime](crate::runtime) carries it out; anything that has an outcome
//! comes back later as a [`UiEvent`](super::event::UiEvent).

use std::time::Duration;

use super::{event::Timer, state::SessionId};
use crate::{connections::StoredConnection, db::DbCommand, import::Format, secrets::Password};

/// A side-effect request. Not `Clone`: [`Effect::StashPassword`] holds a
/// [`Password`], whose `Debug` is redacted, so no effect's `Debug` shows one.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    /// Hand this command to the session's database worker.
    Db(SessionId, DbCommand),
    /// Open a session on this connection: start a worker of its own and
    /// connect it. Its answers come back as `UiEvent::Db` with the same id.
    OpenSession(SessionId, Box<StoredConnection>),
    /// Drop the session's worker, which closes its connection once the
    /// command it is running, if any, is done.
    CloseSession(SessionId),
    /// Write these to disk. Ephemeral records (the scratch database) are
    /// never in the list.
    SaveConnections(Vec<StoredConnection>),
    /// Read the saved connections, answered with `FileAnswer::ConnectionsLoaded`.
    LoadConnections,
    /// Read connections from another file, answered with
    /// `FileAnswer::Imported` or `ImportFailed`.
    Import {
        path: String,
        format: Format,
    },
    /// Write a backup of these connections, answered with
    /// `FileAnswer::ConnectionsExported` or `ConnectionsExportFailed`. A
    /// file already there is only replaced with `overwrite`; without it the
    /// answer is `ConnectionsExportExists`.
    ExportConnections {
        path: String,
        connections: Vec<StoredConnection>,
        overwrite: bool,
    },
    /// Remove this connection's password from the keychain.
    ForgetSecret(String),
    /// Remove this connection's query history (T306 keeps one; until then
    /// there is nothing to remove).
    DeleteHistory(String),
    /// Try `record` on a throw-away connection that is neither saved nor
    /// added to the tree, then drop it. A password typed for it waits in
    /// the stash under [`test_stash_key`]
    /// and is cleared once used. Answered with `UiEvent::ConnectionTested`
    /// carrying the same `test` number.
    TestConnection {
        test: u64,
        record: Box<StoredConnection>,
    },
    /// Leave a typed password in `secrets::stash` under the connection's id,
    /// where that connection's worker takes it from. The `Db` effect that
    /// follows carries no password, only the connection's record or nothing
    /// at all.
    StashPassword {
        connection: String,
        password: Password,
    },
    /// Deliver `UiEvent::Timer(timer)` once `after` has passed. Nothing
    /// cancels one: the reducer ignores a timer it no longer waits for.
    ScheduleTimer {
        timer: Timer,
        after: Duration,
    },
    /// Put this text on the system clipboard, through the terminal (OSC 52).
    CopyToClipboard(String),
    /// Redraw the whole terminal.
    ClearScreen,
    Suspend,
    Quit,
}

/// Where a password typed for a connection test of `id` waits in the
/// stash, apart from one typed to connect it, so neither uses up the other.
pub(crate) fn test_stash_key(id: &str) -> String {
    format!("test:{id}")
}
