//! [`UiEvent`]: everything the reducer reacts to.
//!
//! Time never comes from the clock inside the reducer: an event that needs
//! it carries the moment it happened at.

use std::{sync::Arc, time::Instant};

use crossterm::event::{KeyEvent, MouseEvent};
use qry_core::{
    QueryResult,
    jobs::JobId,
    schema::{Children, SchemaPath},
    snippets::Snippet,
};

use super::state::SessionId;
use crate::{
    connections::StoredConnection,
    history_store::Loaded,
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
    /// The wall-clock time, sent with every `Clock`, for the query
    /// history's timestamps.
    WallClock(WallClock),
    /// Something the runtime has to tell the user, such as a keychain that
    /// would not forget a password. It is logged too.
    Notice(Notice),
    /// A timer the reducer asked for with `Effect::ScheduleTimer` ran out.
    Timer(Timer),
    /// A session's database worker answered.
    Db(SessionId, DbAnswer),
    /// A file the runtime read or wrote off the event loop.
    File(FileAnswer),
    /// The outcome of `Effect::TestConnection` number `test`: how long the
    /// connection took and what the server said it is, or why it failed.
    ConnectionTested {
        test: u64,
        outcome: Result<Tested, String>,
    },
}

/// A connection test that worked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tested {
    pub millis: u64,
    pub version: String,
}

/// The date and time, and the local zone, as the runtime saw it last.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WallClock {
    /// Seconds since the Unix epoch, UTC.
    pub unix: i64,
    /// The local zone's offset, in seconds east of UTC.
    pub offset: i32,
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
    /// The backup's file is already there (the path as written), so it was
    /// left alone until the user says to replace it.
    ConnectionsExportExists(String),
    /// Every connection's query history, as read at startup.
    HistoryLoaded(Vec<Loaded>),
    /// A history file could not be written; the runtime has logged why.
    HistoryWriteFailed(String),
    /// The saved snippets, as read at startup.
    SnippetsLoaded(Vec<Snippet>),
    /// What is at the path the Export dialog names.
    ExportProbed(ExportProbe),
}

/// What the runtime found at the path typed into the Export dialog, for its
/// preview: the file it names in full, and whether it is there already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportProbe {
    /// The path as typed and the format, so a late answer about an older
    /// path is told apart.
    pub input: String,
    pub etype: qry_core::ExportType,
    /// `~` expanded and the extension added.
    pub resolved: String,
    pub exists: bool,
    pub is_dir: bool,
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
    /// The query read these rows, then failed: the rows are kept, and
    /// the message says why it stopped.
    QueryStopped {
        result: Arc<QueryResult>,
        message: String,
    },
    /// The query's stream stopped at the row limit and is kept open, for
    /// a `FetchMore` to read on: sent just before the `QueryDone` of the
    /// truncated result it belongs to.
    CursorKept,
    /// The kept stream was closed, as the next command needed the
    /// connection: its result can no longer load more.
    CursorClosed,
    /// The answer to a `FetchMore`: the next rows of the kept stream.
    MoreRows(Box<MoreRows>),
    /// The running statement has read this many rows so far.
    Progress {
        rows: u64,
    },
    /// Autocommit is now on or off, or why it could not be changed.
    Autocommit(Result<bool, String>),
    /// The query did not run; the status line that follows says why in
    /// full. Either this, `QueryStopped` or `QueryDone` ends every query.
    QueryFailed {
        message: String,
        /// Where in the statement, as a byte offset, when the server says.
        position: Option<usize>,
    },
    /// The answer to an `Export`: how many rows went to the file (its path
    /// as shown, `~/…`), with anything worth knowing about it, or why it
    /// could not be written and how far it got. `job` is the export's job,
    /// for an export of every row.
    Exported {
        job: Option<JobId>,
        rows: u64,
        path: String,
        warning: Option<String>,
    },
    ExportFailed {
        job: Option<JobId>,
        rows: u64,
        path: String,
        message: String,
    },
    /// The file is already there and was left alone, until the user says
    /// to replace it.
    ExportExists {
        job: Option<JobId>,
        path: String,
    },
    /// An export of every row has written this many so far.
    ExportProgress {
        job: JobId,
        rows: u64,
    },
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

/// What a `FetchMore` read on from a kept stream: its next rows, whether
/// more wait behind them, how long it took, and the error that stopped it,
/// if one did.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct MoreRows {
    pub rows: Vec<Vec<Option<String>>>,
    pub more: bool,
    pub elapsed: std::time::Duration,
    pub error: Option<String>,
}

/// The rows' count only: rows must never reach the log.
impl std::fmt::Debug for MoreRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoreRows")
            .field("rows", &self.rows.len())
            .field("more", &self.more)
            .field("elapsed", &self.elapsed)
            .field("error", &self.error)
            .finish()
    }
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
            Self::QueryStopped { result, message } => f
                .debug_struct("QueryStopped")
                .field("rows", &result.rows.len())
                .field("message", message)
                .finish(),
            Self::CursorKept => f.write_str("CursorKept"),
            Self::CursorClosed => f.write_str("CursorClosed"),
            Self::MoreRows(more) => f.debug_tuple("MoreRows").field(more).finish(),
            Self::Progress { rows } => f.debug_struct("Progress").field("rows", rows).finish(),
            Self::Autocommit(outcome) => f.debug_tuple("Autocommit").field(outcome).finish(),
            Self::QueryFailed { message, position } => f
                .debug_struct("QueryFailed")
                .field("message", message)
                .field("position", position)
                .finish(),
            Self::Exported {
                job,
                rows,
                path,
                warning,
            } => f
                .debug_struct("Exported")
                .field("job", job)
                .field("rows", rows)
                .field("path", path)
                .field("warning", warning)
                .finish(),
            Self::ExportFailed {
                job,
                rows,
                path,
                message,
            } => f
                .debug_struct("ExportFailed")
                .field("job", job)
                .field("rows", rows)
                .field("path", path)
                .field("message", message)
                .finish(),
            Self::ExportExists { job, path } => f
                .debug_struct("ExportExists")
                .field("job", job)
                .field("path", path)
                .finish(),
            Self::ExportProgress { job, rows } => f
                .debug_struct("ExportProgress")
                .field("job", job)
                .field("rows", rows)
                .finish(),
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
