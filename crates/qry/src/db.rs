//! Background task that owns the database connection.
//!
//! The event loop never awaits a query. It sends a [`DbCommand`] here, and the
//! outcome comes back later as [`DbAnswer`]s, tagged with the session's id.

use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use qry_core::{
    DatabaseType, Driver, ExportOptions, ExportType, QueryResult, exporter,
    jobs::JobId,
    schema::{Children, SchemaPath},
    shared::{CancelHandle, StreamItem},
    sql::{Dialect, TokenKind, is_read_only, lex},
};
use tokio::{
    sync::mpsc::{self, UnboundedSender},
    task::JoinHandle,
};

use crate::{
    app::{
        event::{DbAnswer, MoreRows},
        state::SessionId,
    },
    connections::{Secret, StoredConnection},
    notice::Notice,
    secrets::{self, Password},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbCommand {
    /// Open this connection. The record holds no password: the worker
    /// resolves the secret from the keychain, the environment, or a prompt.
    Connect(Box<StoredConnection>),
    /// A prompted password is waiting in the stash, under the id of the
    /// connection the worker last asked about.
    PasswordEntered,
    /// Runs a statement, keeping the first `max_rows` rows.
    Query(String),
    /// Runs a statement and keeps every row.
    QueryAll(String),
    /// Reads the next `max_rows` rows of the last query's kept stream,
    /// without running it again.
    FetchMore,
    /// Turns the connection's autocommit on or off.
    SetAutocommit(bool),
    /// Writes rows to a file: the rows in hand, or every row of a
    /// statement run again.
    Export(Box<ExportJob>),
    /// One node of the schema browser: the catalogs, a catalog's objects, or
    /// an object's columns and indexes. The answer carries `generation`
    /// back, so the tree can tell it from a later load of the same node.
    LoadSchema { path: SchemaPath, generation: u64 },
}

/// Rows an export of every row reads at a time; no more than a few such
/// batches are held at once.
pub(crate) const EXPORT_FETCH_SIZE: usize = 1000;

/// Rows a streamed query reads at a time.
pub(crate) const QUERY_FETCH_SIZE: usize = 1000;

/// A running statement says how far it has got once it has read this many
/// more rows, or once this long has passed since it last said.
pub(crate) const PROGRESS_ROWS: u64 = 1000;
pub(crate) const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// What an export writes, and where.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportJob {
    /// The job an export of every row runs as. An export of the rows in
    /// hand has none: its dialog waits for the answer instead.
    pub job: Option<JobId>,
    pub source: ExportSource,
    /// As typed: the worker expands `~` and adds the extension.
    pub path: String,
    pub etype: ExportType,
    pub options: ExportOptions,
    /// Replace a file already there; without it the answer is
    /// `ExportExists`.
    pub overwrite: bool,
}

/// The rows an export writes.
#[derive(Clone, PartialEq, Eq)]
pub enum ExportSource {
    /// A result already fetched.
    Fetched(Arc<QueryResult>),
    /// Every row of this statement, run again and streamed to the file.
    Rerun(String),
}

/// Like the derived `Debug`, but a result shows only its size, so no rows
/// reach the log.
impl std::fmt::Debug for ExportSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetched(result) => write!(f, "Fetched({} rows)", result.rows.len()),
            Self::Rerun(sql) => f.debug_tuple("Rerun").field(sql).finish(),
        }
    }
}

impl std::fmt::Debug for ExportJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportJob")
            .field("job", &self.job)
            .field("source", &self.source)
            .field("path", &self.path)
            .field("etype", &self.etype)
            .field("options", &self.options)
            .field("overwrite", &self.overwrite)
            .finish()
    }
}

/// The connection, and where to say how far an export has got.
#[derive(Default)]
struct Worker {
    /// Shared with the task a streamed query reads on.
    driver: Arc<Driver>,
    /// The session's id and its answers, for progress sent while a
    /// command still runs.
    progress: Option<(SessionId, Answers)>,
    /// The connection waiting for a password to be typed.
    asked: Option<StoredConnection>,
    /// The most rows a query keeps; 0 keeps all.
    max_rows: usize,
    /// Where the open connection's cancel handle is left, for the runtime
    /// to stop a statement while this worker waits on it.
    cancel: CancelSlot,
    /// The last query's stream, kept open after it stopped at the row
    /// limit, until a command needs the connection.
    cursor: Option<Cursor>,
    /// Autocommit was turned off: on Postgres and SQLite each COMMIT or
    /// ROLLBACK is followed by a BEGIN.
    manual: bool,
}

/// A query's rows read on a task of their own, through a channel of one
/// batch: when nobody reads, the stream waits, and with it the cursor on
/// the server, so `FetchMore` goes on where the last read stopped.
struct Cursor {
    rows: mpsc::Receiver<StreamItem>,
    task: JoinHandle<anyhow::Result<u64>>,
    /// Rows read off the stream past what the last answer held.
    carry: Vec<Vec<Option<String>>>,
}

/// What one read of a [`Cursor`] gave.
#[derive(Default)]
struct Read {
    columns: Option<Vec<qry_core::ColumnMeta>>,
    rows: Vec<Vec<Option<String>>>,
    /// More rows wait in the stream.
    more: bool,
    /// What stopped the stream, if it failed.
    error: Option<anyhow::Error>,
}

/// Dropping the stream's reader stops it at its next batch; the task goes
/// too, in case it never gets that far.
impl Drop for Cursor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Cursor {
    /// Starts streaming `sql` on `driver`.
    fn start(driver: Arc<Driver>, sql: String) -> Self {
        let (tx, rows) = mpsc::channel(1);
        let task =
            tokio::spawn(async move { driver.query_stream(&sql, QUERY_FETCH_SIZE, tx).await });
        Self {
            rows,
            task,
            carry: Vec::new(),
        }
    }

    /// Reads up to `limit` rows (all for 0), telling `progress` how many
    /// it has so far, then looks one batch further to tell whether more
    /// wait.
    async fn read(&mut self, limit: usize, progress: &mut (dyn FnMut(u64) + Send)) -> Read {
        let mut read = Read {
            rows: std::mem::take(&mut self.carry),
            ..Read::default()
        };
        let mut ended = false;
        while limit == 0 || read.rows.len() < limit {
            match self.rows.recv().await {
                Some(StreamItem::Columns(columns)) => read.columns = Some(columns),
                Some(StreamItem::Rows(batch)) => {
                    read.rows.extend(batch);
                    progress(read.rows.len() as u64);
                }
                None => {
                    ended = true;
                    read.error = self.finish().await;
                    break;
                }
            }
        }
        if limit > 0 && read.rows.len() > limit {
            self.carry = read.rows.split_off(limit);
        }
        if ended {
            return read;
        }
        if !self.carry.is_empty() {
            read.more = true;
            return read;
        }
        loop {
            match self.rows.recv().await {
                Some(StreamItem::Rows(batch)) if !batch.is_empty() => {
                    self.carry = batch;
                    read.more = true;
                    return read;
                }
                Some(StreamItem::Columns(columns)) => read.columns = Some(columns),
                Some(StreamItem::Rows(_)) => {}
                None => {
                    read.error = self.finish().await;
                    return read;
                }
            }
        }
    }

    /// The stream's own outcome, once it has ended.
    async fn finish(&mut self) -> Option<anyhow::Error> {
        match (&mut self.task).await {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(e),
            Err(e) => Some(anyhow::anyhow!("the query's reader stopped: {e}")),
        }
    }
}

/// When a running statement says next how far it has got: every
/// [`PROGRESS_ROWS`] rows, or after [`PROGRESS_EVERY`] of the worker's
/// time, whichever comes first.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Throttle {
    rows: u64,
    at: Instant,
}

impl Throttle {
    pub(crate) fn new(now: Instant) -> Self {
        Self { rows: 0, at: now }
    }

    /// Whether `rows` read by `now` is worth saying, and if so, notes it.
    pub(crate) fn due(&mut self, rows: u64, now: Instant) -> bool {
        let due = rows >= self.rows + PROGRESS_ROWS
            || (rows > self.rows && now.duration_since(self.at) >= PROGRESS_EVERY);
        if due {
            (self.rows, self.at) = (rows, now);
        }
        due
    }
}

/// Whether the worker streams `sql`, keeping the stream open when it stops
/// at the row limit: a statement that returns rows and changes nothing.
/// Transaction statements and anything that writes go the plain way, which
/// reports the rows they changed.
pub(crate) fn streams(dialect: Dialect, sql: &str) -> bool {
    let first = lex(dialect, sql)
        .into_iter()
        .find(|t| matches!(t.kind, TokenKind::Keyword | TokenKind::Ident))
        .map(|t| sql[t.span].to_ascii_uppercase());
    matches!(
        first.as_deref(),
        Some("SELECT" | "WITH" | "VALUES" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC")
    ) && is_read_only(dialect, sql)
}

/// Whether autocommit off is a transaction qry begins itself, again after
/// each COMMIT or ROLLBACK: the backends without the setting.
fn begins_again(kind: Option<DatabaseType>) -> bool {
    matches!(kind, Some(DatabaseType::Postgres | DatabaseType::Sqlite))
}

/// The cancel handle of a session's open connection, if it has one.
pub(crate) type CancelSlot = Arc<Mutex<Option<Arc<dyn CancelHandle>>>>;

/// Answers as they arrive from the workers, each tagged with its session.
pub(crate) type Answers = UnboundedSender<(SessionId, DbAnswer)>;

/// Starts the worker for session `id`, keeping at most `max_rows` rows of
/// a query, and returns the sender used to reach it and where it leaves
/// its connection's cancel handle. Commands run one at a time, in order.
/// The worker stops once every sender has been dropped, or once nobody
/// listens to its answers.
pub(crate) fn spawn(
    id: SessionId,
    answers: Answers,
    max_rows: usize,
) -> (UnboundedSender<DbCommand>, CancelSlot) {
    let cancel = CancelSlot::default();
    let worker = Worker {
        max_rows,
        cancel: cancel.clone(),
        ..Worker::default()
    };
    (spawn_worker(worker, id, answers), cancel)
}

/// Locks the cancel slot, even if a panic poisoned it: it only ever holds
/// a handle or nothing.
pub(crate) fn lock(slot: &CancelSlot) -> MutexGuard<'_, Option<Arc<dyn CancelHandle>>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `worker` on its own task; tests hand in one whose driver is a fake.
fn spawn_worker(mut worker: Worker, id: SessionId, answers: Answers) -> UnboundedSender<DbCommand> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    worker.progress = Some((id, answers.clone()));
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            for answer in worker.run(command).await {
                if answers.send((id, answer)).is_err() {
                    return;
                }
            }
        }
    });
    tx
}

impl Worker {
    async fn run(&mut self, command: DbCommand) -> Vec<DbAnswer> {
        // Anything that needs the connection closes the kept stream first:
        // it holds the connection until it is read to the end or dropped.
        let keeps = match &command {
            DbCommand::FetchMore => true,
            DbCommand::Export(job) => matches!(job.source, ExportSource::Fetched(_)),
            _ => false,
        };
        let mut answers = Vec::new();
        if !keeps && self.cursor.take().is_some() {
            answers.push(DbAnswer::CursorClosed);
        }
        answers.extend(self.run_command(command).await);
        answers
    }

    async fn run_command(&mut self, command: DbCommand) -> Vec<DbAnswer> {
        match command {
            DbCommand::Connect(record) => self.connect(*record).await,
            DbCommand::PasswordEntered => match self.asked.take() {
                Some(record) => {
                    let typed = secrets::take_stash(&record.id).unwrap_or_default();
                    self.open(record, typed, true).await
                }
                None => Vec::new(),
            },
            DbCommand::Query(sql) => self.query(&sql, self.max_rows).await,
            DbCommand::QueryAll(sql) => self.query(&sql, 0).await,
            DbCommand::FetchMore => self.fetch_more().await,
            DbCommand::SetAutocommit(on) => self.set_autocommit(on).await,
            DbCommand::Export(job) => self.export(*job).await,
            DbCommand::LoadSchema { path, generation } => {
                let outcome = self.driver.load_schema(&path).await;
                vec![schema_answer(path, generation, outcome)]
            }
        }
    }

    /// Runs `sql`, keeping at most `max_rows` rows (0 keeps all). A read
    /// on a backend that streams is streamed, saying how far it has got
    /// as it goes, and its stream is kept when it stops at the limit.
    async fn query(&mut self, sql: &str, max_rows: usize) -> Vec<DbAnswer> {
        let started = Instant::now();
        let kind = self.driver.kind();
        let streamed =
            self.driver.streams_rows() && kind.as_ref().is_some_and(|k| streams(k.dialect(), sql));
        let mut answers = if streamed {
            self.stream(sql, max_rows, started).await
        } else {
            let outcome = self.driver.query_limited(sql, max_rows).await;
            query_answer(outcome.map_err(Failure), started.elapsed(), None)
        };
        // With autocommit off, a transaction ended is followed by the next.
        let ok = !answers
            .iter()
            .any(|a| matches!(a, DbAnswer::QueryFailed { .. }));
        if ok && self.manual && begins_again(kind) && crate::app::tx_change(sql) == Some(false) {
            if let Err(e) = self.driver.set_autocommit(false).await {
                answers.push(error(format!(
                    "could not begin the next transaction: {}",
                    first_line(&format!("{e:#}"))
                )));
            }
        }
        answers
    }

    /// Streams `sql`, keeping the stream open if it stops at `max_rows`.
    async fn stream(&mut self, sql: &str, max_rows: usize, started: Instant) -> Vec<DbAnswer> {
        let mut cursor = Cursor::start(self.driver.clone(), sql.to_string());
        let mut progress = self.reporter();
        let read = cursor.read(max_rows, &mut progress).await;
        let elapsed = started.elapsed();
        let rows = read.rows.len();
        let result = QueryResult {
            columns: read.columns.unwrap_or_default(),
            rows: read.rows,
            affected: Some(rows as u64),
            elapsed,
            truncated: read.more,
        };
        match read.error {
            // Some rows came before it failed: they are kept.
            Some(e) if rows > 0 => {
                let message = first_line(&format!("{e}"));
                vec![
                    DbAnswer::QueryStopped {
                        result: Arc::new(result),
                        message: message.clone(),
                    },
                    error(format!(
                        "stopped after {} rows: {message}",
                        qry_core::jobs::format_rows(rows as u64)
                    )),
                ]
            }
            Some(e) => query_answer(Err(Failure(e)), elapsed, None),
            None => {
                let mut answers = Vec::new();
                if read.more {
                    self.cursor = Some(cursor);
                    answers.push(DbAnswer::CursorKept);
                }
                answers.extend(query_answer::<Failure>(Ok(result), elapsed, None));
                answers
            }
        }
    }

    /// Reads the next `max_rows` rows of the kept stream.
    async fn fetch_more(&mut self) -> Vec<DbAnswer> {
        let Some(mut cursor) = self.cursor.take() else {
            return vec![DbAnswer::MoreRows(Box::new(MoreRows {
                error: Some(CURSOR_CLOSED.into()),
                ..MoreRows::default()
            }))];
        };
        let started = Instant::now();
        let mut progress = self.reporter();
        let read = cursor.read(self.max_rows, &mut progress).await;
        let more = read.more && read.error.is_none();
        if more {
            self.cursor = Some(cursor);
        }
        vec![DbAnswer::MoreRows(Box::new(MoreRows {
            rows: read.rows,
            more,
            elapsed: started.elapsed(),
            error: read.error.map(|e| first_line(&format!("{e}"))),
        }))]
    }

    /// Turns autocommit on or off, as [`Driver::set_autocommit`] does.
    async fn set_autocommit(&mut self, on: bool) -> Vec<DbAnswer> {
        match self.driver.set_autocommit(on).await {
            Ok(()) => {
                self.manual = !on;
                vec![DbAnswer::Autocommit(Ok(on))]
            }
            Err(e) => vec![DbAnswer::Autocommit(Err(first_line(&format!("{e:#}"))))],
        }
    }

    /// What says how far a statement has got: a `Progress` answer, sent
    /// at once, as often as [`Throttle`] allows.
    fn reporter(&self) -> impl FnMut(u64) + Send + use<> {
        let progress = self.progress.clone();
        let mut throttle = Throttle::new(Instant::now());
        move |rows| {
            if let Some((id, answers)) = &progress
                && throttle.due(rows, Instant::now())
            {
                let _ = answers.send((*id, DbAnswer::Progress { rows }));
            }
        }
    }

    /// Finds the password this record says it uses, and opens the
    /// connection. Anything it cannot find without the user is asked for,
    /// and the stored mode is never rewritten behind their back.
    async fn connect(&mut self, record: StoredConnection) -> Vec<DbAnswer> {
        // A password typed into the New Connection form is waiting here.
        if let Some(typed) = secrets::take_stash(&record.id) {
            return self.open(record, typed, true).await;
        }
        match record.secret.clone() {
            Secret::None => self.open(record, Password::default(), false).await,
            Secret::Env { var } => match secrets::from_env(&var) {
                Some(password) => self.open(record, password, false).await,
                None => self.ask(record),
            },
            Secret::Keyring => {
                let id = record.id.clone();
                match tokio::task::spawn_blocking(move || secrets::fetch(&id)).await {
                    Ok(Ok(Some(password))) => self.open(record, password, false).await,
                    // No entry, or no keychain to ask: prompt for this
                    // connect only.
                    _ => self.ask(record),
                }
            }
            Secret::Prompt => self.ask(record),
        }
    }

    fn ask(&mut self, record: StoredConnection) -> Vec<DbAnswer> {
        let answer = DbAnswer::NeedPassword {
            id: record.id.clone(),
            name: record.title().to_string(),
        };
        self.asked = Some(record);
        vec![answer]
    }

    /// Opens the connection with the password in hand. `typed` says the user
    /// just entered it, which is what makes it worth keeping.
    async fn open(
        &mut self,
        record: StoredConnection,
        password: Password,
        typed: bool,
    ) -> Vec<DbAnswer> {
        let config = match record.to_config(password.as_str()) {
            Ok(config) => config,
            Err(e) => return failed(&format!("{e:#}")),
        };
        let mut driver = Driver::new();
        let label = match driver.connect(config).await {
            Ok(label) => label,
            // Several drivers put the whole DSN, password included, in their
            // error text, so nothing goes out without passing through here.
            Err(e) => return failed(&format!("{e:#}")),
        };

        // A new connection starts in autocommit.
        self.driver = Arc::new(driver);
        self.manual = false;
        *lock(&self.cancel) = self.driver.cancel_handle();
        let mut answers = connected_answer(&record, label);

        // Only a freshly typed password is worth storing, and only once it is
        // known to work.
        if typed && record.secret == Secret::Keyring && !password.is_empty() {
            let id = record.id.clone();
            let stored = tokio::task::spawn_blocking(move || secrets::store(&id, &password)).await;
            // The reducer says so in a toast, and the record asks from now on.
            if !matches!(stored, Ok(Ok(()))) {
                answers.push(DbAnswer::SecretNotStored(record.id.clone()));
            }
        }
        answers
    }

    /// Writes `job`'s rows, unless the file is there and may not be
    /// replaced. The rows in hand are written on a thread that may block;
    /// every row of a statement is streamed from the connection.
    async fn export(&self, job: ExportJob) -> Vec<DbAnswer> {
        let home = crate::import::home_dir();
        let target = exporter::resolve_path(&job.path, job.etype, &home);
        let path = exporter::display_path(&target, &home);
        let tag = job.job;
        if !job.overwrite && target.exists() {
            return vec![DbAnswer::ExportExists { job: tag, path }];
        }
        let (etype, options) = (job.etype, job.options);
        let (outcome, rows) = match job.source {
            ExportSource::Fetched(result) => {
                let rows = result.rows.len() as u64;
                let written = tokio::task::spawn_blocking(move || {
                    exporter::write_result(&result, etype, &target, &options)
                })
                .await;
                let outcome = match written {
                    Ok(written) => written.map_err(|e| format!("{e:#}")),
                    Err(e) => Err(e.to_string()),
                };
                (outcome, rows)
            }
            ExportSource::Rerun(sql) => {
                let seen = Arc::new(AtomicU64::new(0));
                let progress = self.progress.clone();
                let heard = seen.clone();
                let report = move |rows: u64| {
                    heard.store(rows, Ordering::Relaxed);
                    if let (Some((id, answers)), Some(job)) = (&progress, tag) {
                        let _ = answers.send((*id, DbAnswer::ExportProgress { job, rows }));
                    }
                };
                let outcome = exporter::export_query(
                    &self.driver,
                    &sql,
                    EXPORT_FETCH_SIZE,
                    etype,
                    &target,
                    &options,
                    report,
                )
                .await
                .map_err(|e| format!("{e:#}"));
                (outcome, seen.load(Ordering::Relaxed))
            }
        };
        match outcome {
            Ok(written) => vec![DbAnswer::Exported {
                job: tag,
                rows: written.rows,
                path,
                warning: written.warning,
            }],
            Err(e) => {
                let message = secrets::redact(&e);
                vec![DbAnswer::ExportFailed {
                    job: tag,
                    rows,
                    path,
                    message: message.lines().next().unwrap_or_default().to_string(),
                }]
            }
        }
    }
}

/// What the worker reports for a finished query: the result and a summary
/// for the status bar, or the error. The app harness in [`crate::testing`]
/// answers with the same answers, so its stand-in worker cannot drift from
/// this one.
pub(crate) fn query_answer<E: std::fmt::Display + Positioned>(
    outcome: Result<QueryResult, E>,
    elapsed: Duration,
    position: Option<usize>,
) -> Vec<DbAnswer> {
    let position = position.or_else(|| outcome.as_ref().err().and_then(Positioned::position));
    match outcome {
        Ok(mut result) => {
            result.elapsed = elapsed;
            let took = took(elapsed);
            let summary = match result.rows.len() {
                // A CREATE or an INSERT returns no columns to fetch.
                _ if result.columns.is_empty() => format!("Statement executed in {took}"),
                1 => format!("Fetched 1 row in {took}"),
                n => format!("Fetched {n} rows in {took}"),
            };
            vec![DbAnswer::QueryDone(Arc::new(result)), success(summary)]
        }
        Err(e) => vec![
            DbAnswer::QueryFailed {
                message: e.to_string(),
                position,
            },
            error(format!("{e:#}")),
        ],
    }
}

/// What `FetchMore` says when there is no stream to read on.
pub(crate) const CURSOR_CLOSED: &str = "the result's cursor was closed; run the statement again";

/// An error that may say where in the statement the server stopped.
pub(crate) trait Positioned {
    fn position(&self) -> Option<usize> {
        None
    }
}

impl Positioned for &str {}
impl Positioned for String {}

/// A driver's error, which says where the statement failed when the
/// server did.
pub(crate) struct Failure(anyhow::Error);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            write!(f, "{:#}", self.0)
        } else {
            write!(f, "{}", self.0)
        }
    }
}

impl Positioned for Failure {
    fn position(&self) -> Option<usize> {
        self.0
            .downcast_ref::<qry_core::shared::QueryError>()
            .and_then(|e| e.position)
    }
}

/// The first line of `message`, with any password taken out.
fn first_line(message: &str) -> String {
    let message = secrets::redact(message);
    message.lines().next().unwrap_or_default().to_string()
}

/// What the worker reports for a schema load, for the tree. A failure is
/// not a toast: the tree shows it on the node that failed.
pub(crate) fn schema_answer<E: std::fmt::Display>(
    path: SchemaPath,
    generation: u64,
    outcome: Result<Children, E>,
) -> DbAnswer {
    DbAnswer::Schema {
        path,
        generation,
        outcome: outcome.map_err(|e| {
            let message = secrets::redact(&format!("{e:#}"));
            message.lines().next().unwrap_or_default().to_string()
        }),
    }
}

/// What the worker reports once `record` is open as `label`: the answer the
/// modal waits for, and the name the status bar shows.
pub(crate) fn connected_answer(record: &StoredConnection, label: String) -> Vec<DbAnswer> {
    let shown = if record.name.is_empty() {
        label.clone()
    } else {
        format!("{} ({label})", record.name)
    };
    vec![
        DbAnswer::Connected(label),
        success(format!("Connected to {shown}")),
    ]
}

/// Tries `record` on a connection of its own, which is dropped as soon as
/// the server has said what it is: how long connecting took and the
/// server's version, or why it failed (with any password taken out).
/// `typed` is a password typed into the dialog; without one the record's
/// own source is used, but never a prompt. The password is cleared when
/// this returns.
pub(crate) async fn test_connection(
    record: StoredConnection,
    typed: Option<Password>,
) -> Result<crate::app::event::Tested, String> {
    let password = match (typed, &record.secret) {
        (Some(typed), _) => typed,
        (None, Secret::None) => Password::default(),
        (None, Secret::Env { var }) => secrets::from_env(var)
            .ok_or_else(|| format!("the environment variable {var} is not set"))?,
        (None, Secret::Keyring) => {
            let id = record.id.clone();
            match tokio::task::spawn_blocking(move || secrets::fetch(&id)).await {
                Ok(Ok(Some(password))) => password,
                _ => return Err("no password in the keychain yet; type it to test".into()),
            }
        }
        (None, Secret::Prompt) => return Err("type the password to test the connection".into()),
    };
    let outcome = async {
        let config = record.to_config(password.as_str())?;
        let mut driver = Driver::new();
        let started = Instant::now();
        fn to_report(e: impl std::fmt::Display) -> color_eyre::Report {
            color_eyre::eyre::eyre!("{e:#}")
        }
        driver.connect(config).await.map_err(to_report)?;
        let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let version = driver.server_version().await.map_err(to_report)?;
        Ok::<_, color_eyre::Report>(crate::app::event::Tested { millis, version })
    }
    .await;
    outcome.map_err(|e| {
        let message = secrets::redact(&format!("{e:#}"));
        message.lines().next().unwrap_or_default().to_string()
    })
}

/// How long something took, in the unit that reads best: `0.4 ms`, `12 ms`,
/// `1.40 s`.
pub(crate) fn took(elapsed: Duration) -> String {
    qry_core::jobs::format_elapsed(elapsed)
}

fn success(message: String) -> DbAnswer {
    DbAnswer::Notice(Notice::success(message))
}

fn error(message: String) -> DbAnswer {
    DbAnswer::Notice(Notice::error(message))
}

/// A connection that did not open: the short message for the modal, the whole
/// chain for the status bar, both with any password taken out of them.
fn failed(message: &str) -> Vec<DbAnswer> {
    let message = secrets::redact(message);
    let short = message.lines().next().unwrap_or_default().to_string();
    vec![
        DbAnswer::ConnectFailed(short),
        error(format!("Connection failed: {message}")),
    ]
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod db_tests;
