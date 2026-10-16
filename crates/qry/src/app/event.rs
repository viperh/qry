//! [`UiEvent`]: everything the reducer reacts to.
//!
//! Time never comes from the clock inside the reducer: an event that needs
//! it carries the moment it happened at.

use std::{sync::Arc, time::Instant};

use crossterm::event::{KeyEvent, MouseEvent};
use qry_core::{
    QueryResult,
    schema::{Children, SchemaPath},
};

use super::state::SessionId;
use crate::{
    connections::StoredConnection,
    import::Imported,
    notice::{Notice, TimeOfDay},
};

/// An event delivered to [`App::handle`](super::App::handle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// Delivered once, before anything else: the app opens its scratch
    /// connection and asks for the saved ones.
    Started,
    Key(KeyEvent),
    /// A click or the wheel, at a cell of the screen.
    Mouse(MouseEvent),
    /// Text pasted into the terminal, whole.
    Paste(String),
    Resize(u16, u16),
    /// The tick-rate timer, at the moment the runtime saw it.
    Tick(Instant),
    /// The wall-clock time of day, sent as it changes, for the timestamps
    /// of the message history.
    Clock(TimeOfDay),
    /// Something the runtime has to tell the user, such as a keychain that
    /// would not forget a password. It is logged too.
    Notice(Notice),
    /// A timer the reducer asked for with `Effect::ScheduleTimer` ran out.
    Timer(Timer),
    /// A session's database worker answered.
    Db(SessionId, DbAnswer),
    /// A file the runtime read or wrote off the event loop.
    File(FileAnswer),
}

/// Which timer ran out. The reducer tells a stale one from the live one by
/// what it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Timer {
    /// The wait for the rest of a multi-key binding, numbered as
    /// `State::key_timer` counts them.
    KeySequence(u64),
    /// The next frame of a waiting dialog's spinner, numbered as
    /// `State::spinner_timer` counts them.
    Spinner(u64),
    /// The visible toast's lifetime ran out, numbered as `Toasts` counts
    /// its timers.
    Toast(u64),
    /// The next frame of the tree's spinner, numbered as
    /// `TreeState::spinner_timer` counts them.
    Tree(u64),
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
    /// The query did not run; the status line that follows says why in
    /// full. Either this or `QueryDone` ends every query.
    QueryFailed {
        message: String,
        /// Where in the statement, as a byte offset, when the server says.
        position: Option<usize>,
    },
    /// The answer to an `Export`, the same way round: the path it wrote, or
    /// why it could not.
    Exported(String),
    ExportFailed(String),
    /// The answer to a `LoadSchema`: what the node holds, or why it could
    /// not be read.
    Schema {
        path: SchemaPath,
        generation: u64,
        outcome: Result<Children, String>,
    },
    /// Something to tell the user, as a toast and in the history.
    Notice(Notice),
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
            Self::QueryFailed { message, position } => f
                .debug_struct("QueryFailed")
                .field("message", message)
                .field("position", position)
                .finish(),
            Self::Exported(path) => f.debug_tuple("Exported").field(path).finish(),
            Self::ExportFailed(message) => f.debug_tuple("ExportFailed").field(message).finish(),
            // A catalog can hold thousands of tables: only the count.
            Self::Schema {
                path,
                generation,
                outcome,
            } => {
                let outcome = match outcome {
                    Ok(Children::Catalogs(c)) => format!("{} catalogs", c.len()),
                    Ok(Children::Objects(o)) => format!("{} objects", o.len()),
                    Ok(Children::Details(d)) => {
                        format!("{} columns, {} indexes", d.columns.len(), d.indexes.len())
                    }
                    Err(e) => format!("failed: {e}"),
                };
                f.debug_struct("Schema")
                    .field("path", path)
                    .field("generation", generation)
                    .field("outcome", &outcome)
                    .finish()
            }
            Self::Notice(notice) => f.debug_tuple("Notice").field(notice).finish(),
            Self::WorkerStopped => f.write_str("WorkerStopped"),
        }
    }
}
