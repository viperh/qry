//! [`Effect`]: what the reducer asks the outside world to do.
//!
//! The reducer never performs I/O. It describes what should happen, and the
//! [runtime](crate::runtime) carries it out; anything that has an outcome
//! comes back later as a [`UiEvent`](super::event::UiEvent).

use super::state::SessionId;
use crate::{connections::StoredConnection, db::DbCommand, import::Format, secrets::Password};

/// A side-effect request. Not `Clone`: [`Effect::StashPassword`] holds a
/// [`Password`], whose `Debug` is redacted, so no effect's `Debug` shows one.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    /// Hand this command to the session's database worker.
    Db(SessionId, DbCommand),
    /// Open a session on this connection. Its answers come back as
    /// `UiEvent::Db` with the same id.
    OpenSession(SessionId, Box<StoredConnection>),
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
    /// Leave a typed password in `secrets::stash`, where the worker takes it
    /// from. The `Db` effect that follows carries no password, only the
    /// connection's record or nothing at all.
    StashPassword(Password),
    /// Redraw the whole terminal.
    ClearScreen,
    Suspend,
    Quit,
}
