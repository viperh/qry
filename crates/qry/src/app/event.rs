//! [`UiEvent`]: everything the reducer reacts to.
//!
//! Time never comes from the clock inside the reducer: an event that needs
//! it carries the moment it happened at.

use std::{sync::Arc, time::Instant};

use crossterm::event::KeyEvent;
use qry_core::QueryResult;

use super::state::{SessionId, StatusCode};
use crate::{connections::StoredConnection, import::Imported};

/// An event delivered to [`App::handle`](super::App::handle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// Delivered once, before anything else: the app opens its scratch
    /// connection and asks for the saved ones.
    Started,
    Key(KeyEvent),
    Resize(u16, u16),
    /// The tick-rate timer, at the moment the runtime saw it.
    Tick(Instant),
    /// A session's database worker answered.
    Db(SessionId, DbAnswer),
    /// A file the runtime read or wrote off the event loop.
    File(FileAnswer),
}

/// The outcome of an effect that reads or writes a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAnswer {
    /// The connections read from disk at startup.
    ConnectionsLoaded(Vec<StoredConnection>),
    /// They could not be read; the runtime has logged why.
    LoadFailed(String),
    /// The connections could not be written; the runtime has logged why.
    SaveFailed(String),
    /// What an imported file held. The tree adds what it does not have
    /// yet, and saves.
    Imported(Box<Imported>),
    ImportFailed(String),
    /// Where the backup of `count` connections went, or why it could not be
    /// written.
    ConnectionsExported {
        path: String,
        count: usize,
    },
    ConnectionsExportFailed(String),
}

/// The database worker's answer to a [`DbCommand`](crate::db::DbCommand).
#[derive(Clone, PartialEq, Eq)]
pub enum DbAnswer {
    /// The answer to a `Connect`: the label it connected to, or why it could
    /// not. The New Connection modal stays open until one of these arrives,
    /// so a typo does not cost the whole form.
    Connected(String),
    ConnectFailed(String),
    /// The worker needs a password typed before it can open this connection.
    /// The prompt leaves it in `secrets::stash`, never in an event.
    NeedPassword {
        id: String,
        name: String,
    },
    /// The keychain refused a password, so the connection must ask for one
    /// every time instead.
    SecretNotStored(String),
    /// Wrapped in `Arc` so the state and the export keep the same rows.
    QueryDone(Arc<QueryResult>),
    /// The answer to an `Export`, the same way round: the path it wrote, or
    /// why it could not.
    Exported(String),
    ExportFailed(String),
    /// The tables of whatever is connected, for the tree.
    TablesLoaded(Vec<String>),
    TablesFailed(String),
    /// A line for the status bar.
    Status(StatusCode),
    /// The worker has gone, so nothing sent to it will be answered.
    WorkerStopped,
}

/// Like the derived `Debug`, except that a result shows only its size: a
/// whole result set must never reach the log.
impl std::fmt::Debug for DbAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected(label) => f.debug_tuple("Connected").field(label).finish(),
            Self::ConnectFailed(message) => f.debug_tuple("ConnectFailed").field(message).finish(),
            Self::NeedPassword { id, name } => f
                .debug_struct("NeedPassword")
                .field("id", id)
                .field("name", name)
                .finish(),
            Self::SecretNotStored(id) => f.debug_tuple("SecretNotStored").field(id).finish(),
            Self::QueryDone(result) => write!(
                f,
                "QueryDone({} rows × {} columns)",
                result.rows.len(),
                result.columns.len()
            ),
            Self::Exported(path) => f.debug_tuple("Exported").field(path).finish(),
            Self::ExportFailed(message) => f.debug_tuple("ExportFailed").field(message).finish(),
            Self::TablesLoaded(tables) => f.debug_tuple("TablesLoaded").field(tables).finish(),
            Self::TablesFailed(message) => f.debug_tuple("TablesFailed").field(message).finish(),
            Self::Status(status) => f.debug_tuple("Status").field(status).finish(),
            Self::WorkerStopped => f.write_str("WorkerStopped"),
        }
    }
}
