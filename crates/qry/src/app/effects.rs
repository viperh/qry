//! [`Effect`]: what the reducer asks the outside world to do.
//!
//! The reducer never performs I/O. It describes what should happen, and the
//! [runtime](crate::runtime) carries it out; anything that has an outcome
//! comes back later as a [`UiEvent`](super::event::UiEvent).

use std::time::Duration;

use qry_core::snippets::Snippet;

use super::{event::Timer, state::SessionId};
use crate::{
    connections::StoredConnection, db::DbCommand, history_store::HistoryWrite, import::Format,
    secrets::Password,
};

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
    /// `FileAnswer::ConnectionsExported` or `ConnectionsExportFailed`.
    ExportConnections {
        path: String,
        connections: Vec<StoredConnection>,
    },
    /// Remove this connection's password from the keychain.
    ForgetSecret(String),
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
    /// Read every connection's query history, answered with
    /// `FileAnswer::HistoryLoaded`.
    LoadHistory,
    /// Write to a connection's history file; a failure is answered with
    /// `FileAnswer::HistoryWriteFailed`. It holds SQL text only, already
    /// redacted.
    History(HistoryWrite),
    /// Read the saved snippets, answered with `FileAnswer::SnippetsLoaded`.
    LoadSnippets,
    /// Write the snippets file; a failure is answered with
    /// `FileAnswer::SaveFailed`.
    SaveSnippets(Vec<Snippet>),
    /// Put this text on the system clipboard, through the terminal (OSC 52).
    CopyToClipboard(String),
    /// Redraw the whole terminal.
    ClearScreen,
    Suspend,
    Quit,
}
