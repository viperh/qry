//! The app's state, as plain data.
//!
//! Everything the views draw is here, and only the reducer changes it. It is
//! `Clone + PartialEq`, so a test can replay the same events into two copies
//! and compare them. The few [`Cell`]s are scroll positions the views keep
//! up to date as they draw (what fits depends on the terminal), which the
//! reducer reads back for paging.

use std::{
    cell::Cell,
    collections::{BTreeMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use crossterm::event::KeyEvent;
use qry_core::{
    QueryResult,
    jobs::{JobId, Jobs},
    schema::SchemaCache,
};
use ratatui_textarea::TextArea;

use qry_core::snippets::Snippet;

use super::{
    dialogs::DialogState,
    event::WallClock,
    history::{EditingSnippet, HistoryBook, Recall},
    schematree::{CatalogView, RowKey, VisibleRow},
};
pub use crate::theme::SessionStatus;
use crate::{
    config::Config,
    connections::{ConnLabel, Driver, Secret, StoredConnection},
    keymap::KeyContext,
    notice::{NoticeLog, TimeOfDay, Toasts},
    theme::{Pill, Theme},
};

/// One open session. Ids are handed out in order and never reused within a
/// run, so an answer from a closed session's worker cannot land on a newer
/// one.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

impl SessionId {
    /// The scratch session every run starts with.
    pub const FIRST: Self = Self(1);
}

/// The id of the in-memory database every run starts on. It is never saved,
/// so a fixed id cannot clash with a stored one's keychain entry.
pub(crate) const SCRATCH_ID: &str = "scratch";

/// The scratch database: an empty in-memory SQLite, listed in the tree like
/// any other connection but never saved.
pub(crate) fn scratch_record() -> StoredConnection {
    let mut scratch = StoredConnection::with_id(SCRATCH_ID, "scratch", Driver::Sqlite);
    scratch.path = Some(":memory:".to_string());
    scratch.secret = Secret::None;
    scratch.ephemeral = true;
    scratch
}

/// One open connection and everything bound to it. Its worker lives on the
/// runtime's side, in [`SessionManager`](crate::sessions::SessionManager).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionState {
    pub(crate) id: SessionId,
    /// The `StoredConnection::id` it was opened on.
    pub(crate) connection_id: String,
    pub(crate) title: String,
    pub(crate) status: SessionStatus,
    pub(crate) driver: Driver,
    /// What it is connected to, for the top bar: `postgres · app@db.local`.
    pub(crate) summary: String,
    /// The database, schema or file, for the status line.
    pub(crate) database: Option<String>,
    pub(crate) editor: EditorState,
    /// One tab per statement run, the oldest first; never empty.
    pub(crate) tabs: Vec<ResultTab>,
    pub(crate) active_tab: usize,
    /// How many tabs this session has made, for their numbers.
    pub(crate) tabs_made: usize,
    /// Statements waiting their turn: the worker gets one at a time, so a
    /// failure can stop the rest.
    pub(crate) queue: VecDeque<Queued>,
    /// The statement the worker is running.
    pub(crate) in_flight: Option<Queued>,
    /// The statement whose last run failed, for the editor's gutter.
    pub(crate) failed: Option<Failed>,
    /// Queries sent and not answered yet. One worker runs them in order.
    pub(crate) running: usize,
    /// What each of those does to the transaction, oldest first: `Some(true)`
    /// begins one, `Some(false)` ends it. Applied once it succeeds.
    pub(crate) tx_changes: VecDeque<Option<bool>>,
    /// A transaction was begun and not yet committed or rolled back.
    pub(crate) in_transaction: bool,
    /// Expand its connection in the tree once it is open.
    pub(crate) expand_when_connected: bool,
    /// What the schema browser has loaded through this session.
    pub(crate) schema: SchemaCache,
    /// Its connection's label; `prod` asks before the first write.
    pub(crate) label: Option<ConnLabel>,
    /// A write was confirmed on this `prod` session, so it is not asked
    /// about again.
    pub(crate) writes_confirmed: bool,
    /// Statements held back while the `prod` confirm is open.
    pub(crate) held: Vec<Queued>,
}

impl SessionState {
    pub(crate) fn new(id: SessionId, record: &StoredConnection) -> Self {
        Self {
            id,
            connection_id: record.id.clone(),
            title: record.title().to_string(),
            status: SessionStatus::Connecting,
            driver: record.driver,
            summary: summary(record),
            database: record
                .database
                .clone()
                .or_else(|| record.service.clone())
                .or_else(|| record.path.clone()),
            editor: EditorState::default(),
            tabs: vec![ResultTab::default()],
            active_tab: 0,
            tabs_made: 0,
            queue: VecDeque::new(),
            in_flight: None,
            failed: None,
            running: 0,
            tx_changes: VecDeque::new(),
            in_transaction: false,
            expand_when_connected: false,
            schema: SchemaCache::default(),
            label: record.label,
            writes_confirmed: false,
            held: Vec::new(),
        }
    }

    /// Takes on an edited record's name and target.
    pub(crate) fn retarget(&mut self, record: &StoredConnection) {
        let fresh = Self::new(self.id, record);
        self.title = fresh.title;
        self.driver = fresh.driver;
        self.summary = fresh.summary;
        self.database = fresh.database;
        self.label = fresh.label;
        self.in_transaction = false;
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.status, SessionStatus::Open { .. })
    }

    /// Whether a statement is running or waiting.
    pub(crate) fn busy(&self) -> bool {
        self.running > 0 || !self.queue.is_empty()
    }

    /// The SQL dialect of its driver.
    pub(crate) fn dialect(&self) -> qry_core::sql::Dialect {
        use qry_core::sql::Dialect;
        match self.driver {
            Driver::Postgres => Dialect::Postgres,
            Driver::Mysql | Driver::Mariadb => Dialect::MySql,
            Driver::Sqlite => Dialect::Sqlite,
            Driver::Oracle => Dialect::Oracle,
        }
    }

    pub(crate) fn results(&self) -> &ResultsState {
        &self.tabs[self.active_tab.min(self.tabs.len() - 1)].results
    }

    pub(crate) fn results_mut(&mut self) -> &mut ResultsState {
        let tab = self.active_tab.min(self.tabs.len() - 1);
        &mut self.tabs[tab].results
    }

    /// A tab for what a statement gave: the first, empty tab if nothing
    /// has used it yet, else a new one at the end. It is shown if the last
    /// tab was, so someone looking back at an earlier tab stays there. The
    /// oldest go past [`MAX_TABS`].
    pub(crate) fn new_tab(&mut self, label: String) -> &mut ResultTab {
        self.tabs_made += 1;
        let tab = ResultTab {
            number: self.tabs_made,
            label,
            ..ResultTab::default()
        };
        let following = self.active_tab + 1 >= self.tabs.len();
        if self.tabs.len() == 1 && self.tabs[0].is_empty() {
            self.tabs[0] = tab;
        } else {
            self.tabs.push(tab);
            if self.tabs.len() > MAX_TABS {
                self.tabs.remove(0);
                self.active_tab = self.active_tab.saturating_sub(1);
            }
        }
        let last = self.tabs.len() - 1;
        if following {
            self.active_tab = last;
        }
        &mut self.tabs[last]
    }
}

/// The jobs drawer: which job is selected, counted from the newest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JobsDrawer {
    pub(crate) selected: usize,
}

/// The most jobs the drawer shows at once.
pub(crate) const DRAWER_JOBS: usize = 7;

/// The most result tabs a session keeps.
pub(crate) const MAX_TABS: usize = 20;

/// A statement to run, and where it is in the editor when it came from
/// there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Queued {
    pub(crate) sql: String,
    /// Byte offsets into the editor's text.
    pub(crate) span: Option<Range<usize>>,
    /// Its entry in [`State::jobs`], once queued.
    pub(crate) job: Option<JobId>,
    /// Every row, not only the first `max_rows`.
    pub(crate) all_rows: bool,
}

impl Queued {
    pub(crate) fn new(sql: impl Into<String>, span: Option<Range<usize>>) -> Self {
        Self {
            sql: sql.into(),
            span,
            job: None,
            all_rows: false,
        }
    }
}

/// The statement whose run failed, as byte offsets into the editor's text,
/// and where the server said the problem is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failed {
    pub(crate) span: Range<usize>,
    pub(crate) position: Option<usize>,
}

/// Why a statement failed, as its result tab shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabError {
    pub(crate) message: String,
    /// A byte offset into `sql`.
    pub(crate) position: Option<usize>,
    pub(crate) sql: String,
}

/// One statement's outcome: its rows, or why it failed.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ResultTab {
    pub(crate) number: usize,
    /// `users · 42`, `UPDATE · 3 rows`, `✗ error`.
    pub(crate) label: String,
    pub(crate) results: ResultsState,
    pub(crate) error: Option<TabError>,
    /// The table the statement read, for INSERTs copied from it.
    pub(crate) table: Option<String>,
    /// The statement, to fetch every row of a truncated result again.
    pub(crate) sql: String,
}

impl ResultTab {
    fn is_empty(&self) -> bool {
        self.results.result.is_none() && self.error.is_none()
    }
}

/// `postgres · app@db.local`, `sqlite · :memory:`: the driver and where it
/// points, without the password or the port.
pub(crate) fn summary(record: &StoredConnection) -> String {
    let driver = record.driver.name();
    let target = match (&record.path, &record.host) {
        (Some(path), _) => path.clone(),
        (None, Some(host)) => {
            let who = record
                .user
                .as_deref()
                .map(|u| format!("{u}@"))
                .unwrap_or_default();
            format!("{who}{host}")
        }
        (None, None) => String::new(),
    };
    if target.is_empty() {
        driver.to_string()
    } else {
        format!("{driver} · {target}")
    }
}

/// The open sessions, in tab order, and the one the editor and the results
/// show. There is always at least one: closing the last opens a fresh
/// scratch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sessions {
    pub(crate) order: Vec<SessionId>,
    pub(crate) active: SessionId,
    pub(crate) by_id: BTreeMap<SessionId, SessionState>,
    /// The id the next session gets.
    next: u32,
}

/// The scratch session alone, as session 1. Its worker is started by
/// `UiEvent::Started`.
impl Default for Sessions {
    fn default() -> Self {
        let scratch = SessionState::new(SessionId::FIRST, &scratch_record());
        Self {
            order: vec![SessionId::FIRST],
            active: SessionId::FIRST,
            by_id: BTreeMap::from([(SessionId::FIRST, scratch)]),
            next: SessionId::FIRST.0 + 1,
        }
    }
}

impl Sessions {
    #[allow(
        clippy::expect_used,
        reason = "`active` always names an open session: closing it moves it on"
    )]
    pub(crate) fn active(&self) -> &SessionState {
        self.by_id.get(&self.active).expect("the active session")
    }

    #[allow(
        clippy::expect_used,
        reason = "`active` always names an open session: closing it moves it on"
    )]
    pub(crate) fn active_mut(&mut self) -> &mut SessionState {
        self.by_id
            .get_mut(&self.active)
            .expect("the active session")
    }

    pub(crate) fn get(&self, id: SessionId) -> Option<&SessionState> {
        self.by_id.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: SessionId) -> Option<&mut SessionState> {
        self.by_id.get_mut(&id)
    }

    /// The sessions in tab order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &SessionState> + '_ {
        self.order.iter().filter_map(|id| self.by_id.get(id))
    }

    /// Adds a session on `record` at the end of the tabs, without switching
    /// to it.
    pub(crate) fn add(&mut self, record: &StoredConnection) -> SessionId {
        let id = SessionId(self.next);
        self.next += 1;
        self.order.push(id);
        self.by_id.insert(id, SessionState::new(id, record));
        id
    }

    /// Takes a session out. The active one moves to its right-hand
    /// neighbour, or the left-hand one if it was last. The caller makes sure
    /// another is left.
    pub(crate) fn remove(&mut self, id: SessionId) -> Option<SessionState> {
        let index = self.order.iter().position(|s| *s == id)?;
        self.order.remove(index);
        if self.active == id
            && let Some(next) = self
                .order
                .get(index)
                .or_else(|| self.order.get(index.wrapping_sub(1)))
        {
            self.active = *next;
        }
        self.by_id.remove(&id)
    }

    /// The session to use for `connection`: the active one if it is on it,
    /// else the first in tab order.
    pub(crate) fn for_connection(&self, connection: &str) -> Option<SessionId> {
        let active = self.active();
        if active.connection_id == connection {
            return Some(active.id);
        }
        self.iter()
            .find(|s| s.connection_id == connection)
            .map(|s| s.id)
    }

    /// What the tree shows for `connection`: open if any session on it is,
    /// else connecting, else failed; `None` without a session.
    pub(crate) fn connection_status(&self, connection: &str) -> Option<&SessionStatus> {
        let mut statuses: Vec<_> = self
            .iter()
            .filter(|s| s.connection_id == connection)
            .map(|s| &s.status)
            .collect();
        statuses.sort_by_key(|status| match status {
            SessionStatus::Open { .. } => 0,
            SessionStatus::Connecting => 1,
            SessionStatus::Failed(_) => 2,
            SessionStatus::Closed => 3,
        });
        statuses.first().copied()
    }
}

/// A right-hand region that can be maximised.
pub(crate) use crate::views::layout::Region;

/// The screen's adjustable chrome: the split, maximise, the inspector slot
/// and the key-hint line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chrome {
    /// The editor's share of the right-hand side, in percent.
    pub(crate) split_percent: u16,
    pub(crate) maximised: Option<Region>,
    pub(crate) inspector: bool,
    pub(crate) hints: bool,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            split_percent: crate::views::layout::DEFAULT_SPLIT,
            maximised: None,
            inspector: false,
            hints: true,
        }
    }
}

/// The pane the keys go to while nothing is on top of the panes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    #[default]
    Tree,
    Editor,
    Results,
    /// The record inspector, while it is shown (T304).
    Inspector,
}

impl Focus {
    /// The next pane round. The inspector is only in the ring while it is
    /// shown, which the reducer decides; from it, Tab goes back to the grid.
    pub(crate) fn next(self) -> Self {
        match self {
            Focus::Tree => Focus::Editor,
            Focus::Editor => Focus::Results,
            Focus::Results => Focus::Tree,
            Focus::Inspector => Focus::Results,
        }
    }

    pub(crate) fn prev(self) -> Self {
        match self {
            Focus::Tree => Focus::Results,
            Focus::Editor => Focus::Tree,
            Focus::Results => Focus::Editor,
            Focus::Inspector => Focus::Results,
        }
    }

    /// The pane's own key bindings.
    pub(crate) fn context(self) -> KeyContext {
        match self {
            Focus::Tree => KeyContext::Tree,
            Focus::Editor => KeyContext::Editor,
            Focus::Results => KeyContext::Results,
            Focus::Inspector => KeyContext::Inspector,
        }
    }
}

/// Something drawn over the panes. The last one in [`State::overlays`] is on
/// top and gets the keys.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Overlay {
    Dialog(DialogState),
    Help(HelpState),
    Messages(MessagesState),
    Palette(crate::palette::PaletteState),
    /// One cell's whole value (T304).
    Value(ValuePopup),
    /// The query history (ctrl-r); keys as the palette's.
    History(super::history::HistoryDialog),
    /// Manage snippets; keys as the palette's.
    Snippets(super::history::SnippetList),
}

/// The Messages dialog: which entry of the history, newest first, is
/// selected.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MessagesState {
    pub(crate) selected: usize,
    /// First line in view; kept by the view.
    pub(crate) scroll: Cell<usize>,
    /// Lines that fit, from the last draw, for PageUp/PageDown.
    pub(crate) page: Cell<usize>,
}

/// An answer a modal is waiting for, so a late one meant for a modal that has
/// since been closed, or from another session, cannot act on whatever is
/// open now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Awaiting {
    Connect(SessionId),
    Export(SessionId),
    Import,
    ExportConnections,
}

/// A password a session's worker asked for while another prompt was open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PasswordRequest {
    pub(crate) session: SessionId,
    pub(crate) connection: String,
    pub(crate) name: String,
}

/// Mints connection ids without reading the clock: the runtime hands in a
/// seed that differs between runs, and the ids follow from it in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IdGen {
    seed: u64,
    next: u64,
}

impl IdGen {
    pub(crate) fn new(seed: u64) -> Self {
        Self { seed, next: 0 }
    }

    /// Eight hex characters, like the ids the rest of qry mints.
    pub(crate) fn connection_id(&mut self) -> String {
        let mut hasher = DefaultHasher::new();
        (self.seed, self.next).hash(&mut hasher);
        self.next += 1;
        format!("{:08x}", hasher.finish() as u32)
    }
}

/// The whole UI.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub(crate) config: Arc<Config>,
    /// Resolved once at startup and adapted to the terminal.
    pub(crate) theme: Theme,
    /// The terminal's size, from the last resize.
    pub(crate) size: (u16, u16),
    pub(crate) focus: Focus,
    pub(crate) chrome: Chrome,
    /// At most one dialog and one help, the help always on top.
    pub(crate) overlays: Vec<Overlay>,
    /// What the open modal is waiting for the worker to answer, if anything.
    pub(crate) awaiting: Option<Awaiting>,
    /// The open sessions; the editor and the results are the active one's.
    pub(crate) sessions: Sessions,
    /// Password prompts waiting for the one on screen to close, oldest
    /// first.
    pub(crate) password_queue: Vec<PasswordRequest>,
    pub(crate) tree: TreeState,
    /// The toast on screen and those waiting.
    pub(crate) toasts: Toasts,
    /// Every notice so far, for the Messages dialog.
    pub(crate) messages: NoticeLog,
    /// The wall-clock time of day, from the last `UiEvent::Clock`.
    pub(crate) clock: TimeOfDay,
    /// The labels of the palette items run lately, newest first.
    pub(crate) palette_recent: Vec<String>,
    /// The date and time, from the last `UiEvent::WallClock`.
    pub(crate) wall: WallClock,
    /// Every connection's query history.
    pub(crate) history: HistoryBook,
    /// Where alt-↑/↓ got to in the history.
    pub(crate) recall: Option<Recall>,
    /// The saved snippets, as in the file.
    pub(crate) snippets: Vec<Snippet>,
    /// The snippet the editor holds for editing, if any.
    pub(crate) editing_snippet: Option<EditingSnippet>,
    /// The start of a multi-key binding typed so far, normalised.
    pub(crate) pending_keys: Vec<KeyEvent>,
    /// Which `Timer::KeySequence` is the live one: each key that leaves a
    /// sequence waiting starts a new one, and an older one that fires is
    /// ignored.
    pub(crate) key_timer: u64,
    /// The frame of a waiting dialog's spinner, and which `Timer::Spinner`
    /// is the live one.
    pub(crate) spinner: usize,
    pub(crate) spinner_timer: u64,
    /// Every statement run, running or waiting, and the latest finished.
    pub(crate) jobs: Jobs<SessionId>,
    /// The jobs drawer, while it is open.
    pub(crate) drawer: Option<JobsDrawer>,
    /// Quit once no session runs anything: the rollbacks asked for on the
    /// way out are done.
    pub(crate) quit_when_idle: bool,
    /// The first tick's time, and how long since then at the last tick:
    /// the clock jobs are timed on.
    pub(crate) epoch: Option<Instant>,
    pub(crate) now: Duration,
    pub(crate) ids: IdGen,
    /// How many connection tests have been asked for; the open dialog
    /// waits for the answer to the last.
    pub(crate) tests_made: u64,
    /// Something changed since the runtime last drew.
    pub(crate) dirty: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            config: Arc::default(),
            theme: Theme::DARK,
            size: (80, 24),
            focus: Focus::default(),
            chrome: Chrome::default(),
            overlays: Vec::new(),
            awaiting: None,
            sessions: Sessions::default(),
            password_queue: Vec::new(),
            tree: TreeState::default(),
            toasts: Toasts::default(),
            messages: NoticeLog::default(),
            clock: TimeOfDay::default(),
            palette_recent: Vec::new(),
            wall: WallClock::default(),
            history: HistoryBook::default(),
            recall: None,
            snippets: Vec::new(),
            editing_snippet: None,
            pending_keys: Vec::new(),
            key_timer: 0,
            spinner: 0,
            spinner_timer: 0,
            jobs: Jobs::default(),
            drawer: None,
            quit_when_idle: false,
            epoch: None,
            now: Duration::ZERO,
            ids: IdGen::default(),
            tests_made: 0,
            dirty: true,
        }
    }
}

impl State {
    /// What the layout of a screen of this size depends on.
    pub(crate) fn layout_input(
        &self,
        area: ratatui::layout::Rect,
    ) -> crate::views::layout::LayoutInput {
        crate::views::layout::LayoutInput {
            area,
            split_percent: self.chrome.split_percent,
            maximised: self.chrome.maximised,
            inspector: self.chrome.inspector,
            jobs_rows: self.drawer.map_or(0, |_| self.drawer_rows()),
            hints: self.chrome.hints,
        }
    }

    /// The jobs drawer's height: a title and up to seven jobs.
    pub(crate) fn drawer_rows(&self) -> u16 {
        1 + u16::try_from(self.jobs.len().clamp(1, DRAWER_JOBS)).unwrap_or(1)
    }

    /// The terminal is too small to draw the main screen.
    pub(crate) fn too_small(&self) -> bool {
        let (min_w, min_h) = crate::views::layout::MIN_SIZE;
        self.size.0 < min_w || self.size.1 < min_h
    }

    /// The active session's editor.
    #[cfg(test)]
    pub(crate) fn editor(&self) -> &EditorState {
        &self.sessions.active().editor
    }

    pub(crate) fn editor_mut(&mut self) -> &mut EditorState {
        &mut self.sessions.active_mut().editor
    }

    /// The active session's results.
    pub(crate) fn results(&self) -> &ResultsState {
        self.sessions.active().results()
    }

    pub(crate) fn results_mut(&mut self) -> &mut ResultsState {
        self.sessions.active_mut().results_mut()
    }

    pub(crate) fn dialog(&self) -> Option<&DialogState> {
        self.overlays.iter().find_map(|overlay| match overlay {
            Overlay::Dialog(dialog) => Some(dialog),
            Overlay::Help(_)
            | Overlay::Messages(_)
            | Overlay::Palette(_)
            | Overlay::Value(_)
            | Overlay::History(_)
            | Overlay::Snippets(_) => None,
        })
    }

    pub(crate) fn dialog_mut(&mut self) -> Option<&mut DialogState> {
        self.overlays.iter_mut().find_map(|overlay| match overlay {
            Overlay::Dialog(dialog) => Some(dialog),
            Overlay::Help(_)
            | Overlay::Messages(_)
            | Overlay::Palette(_)
            | Overlay::Value(_)
            | Overlay::History(_)
            | Overlay::Snippets(_) => None,
        })
    }

    pub(crate) fn help(&self) -> Option<&HelpState> {
        self.overlays.iter().find_map(|overlay| match overlay {
            Overlay::Help(help) => Some(help),
            Overlay::Dialog(_)
            | Overlay::Messages(_)
            | Overlay::Palette(_)
            | Overlay::Value(_)
            | Overlay::History(_)
            | Overlay::Snippets(_) => None,
        })
    }

    /// Shows `dialog` in place of any open one, under the help if that is
    /// open.
    pub(crate) fn open_dialog(&mut self, dialog: DialogState) {
        self.close_dialog();
        self.overlays.insert(0, Overlay::Dialog(dialog));
    }

    pub(crate) fn close_dialog(&mut self) {
        self.overlays
            .retain(|overlay| !matches!(overlay, Overlay::Dialog(_)));
    }

    /// The contexts a key is looked up in, innermost first: the top
    /// overlay's alone, or the focused pane's and then `Global`.
    pub fn contexts(&self) -> Vec<KeyContext> {
        match self.overlays.last() {
            // The Messages dialog and the value popup scroll and close with
            // the help's keys.
            Some(Overlay::Help(_) | Overlay::Messages(_) | Overlay::Value(_)) => {
                vec![KeyContext::Help]
            }
            Some(Overlay::Palette(_) | Overlay::History(_) | Overlay::Snippets(_)) => {
                vec![KeyContext::Palette]
            }
            Some(Overlay::Dialog(_)) => vec![KeyContext::Dialog],
            None if self.drawer.is_some() => vec![KeyContext::Jobs, KeyContext::Global],
            None => vec![self.focus.context(), KeyContext::Global],
        }
    }

    /// The mode pill: that of the topmost overlay that has one, else the
    /// focused pane's. The help and the dialogs have none, so they keep
    /// the pill of what is underneath.
    pub fn pill(&self) -> Pill {
        let overlays = self.overlays.iter().rev().map(|overlay| match overlay {
            Overlay::Help(_) | Overlay::Messages(_) | Overlay::Value(_) => KeyContext::Help,
            Overlay::Palette(_) | Overlay::History(_) | Overlay::Snippets(_) => KeyContext::Palette,
            Overlay::Dialog(_) => KeyContext::Dialog,
        });
        overlays
            .chain([self.focus.context()])
            .find_map(KeyContext::pill)
            .unwrap_or(Pill::Normal)
    }
}

/// A connection the tree knows about, and what of it is open.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub(crate) record: StoredConnection,
    /// What the database called itself; `None` until it has connected once.
    pub(crate) label: Option<String>,
    pub(crate) expanded: bool,
    /// Which catalogs, folders and objects under it are open, by name, so
    /// they stay open across reloads.
    pub(crate) catalogs: BTreeMap<String, CatalogView>,
    /// Its catalogs were loaded once and the connection's own was opened;
    /// later loads leave what is open alone.
    pub(crate) opened: bool,
    /// Added this session and not written to the file yet.
    pub(crate) unsaved: bool,
}

impl Node {
    pub(crate) fn new(record: StoredConnection, unsaved: bool) -> Self {
        Self {
            record,
            label: None,
            expanded: false,
            catalogs: BTreeMap::new(),
            opened: false,
            unsaved,
        }
    }

    pub(crate) fn title(&self) -> &str {
        self.record.title()
    }
}

/// The tree's `/` filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TreeFilter {
    pub(crate) text: String,
    /// The prompt is taking keys; Enter keeps the filter and stops.
    pub(crate) editing: bool,
    /// What was selected before it opened, for Esc to go back to.
    pub(crate) before: Option<RowKey>,
    /// The text changed: the selection moves to the first match.
    pub(crate) jump: bool,
}

/// The schema browser: the stored connections, what is open under them,
/// and the rows that follow. Which connections are open is the sessions'
/// business, and what they hold is each session's `schema` cache.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TreeState {
    pub(crate) connections: Vec<Node>,
    /// The rows, worked out after each event that can change them.
    pub(crate) rows: Vec<VisibleRow>,
    pub(crate) selected: usize,
    /// The selected row by name, so it follows its node when rows move.
    pub(crate) selected_key: Option<RowKey>,
    /// First row in view, and rows that fit; kept by the view.
    pub(crate) offset: Cell<usize>,
    pub(crate) page: Cell<usize>,
    /// System catalogs (`pg_catalog`, `mysql`, `SYS`) are shown.
    pub(crate) show_system: bool,
    pub(crate) filter: Option<TreeFilter>,
    /// The filter is on and some of what it could match is not loaded.
    pub(crate) filter_partial: bool,
    /// The spinner's frame, which `Timer::Tree` is the live one, and
    /// whether one is scheduled.
    pub(crate) spinner: usize,
    pub(crate) spinner_timer: u64,
    pub(crate) spinning: bool,
}

impl TreeState {
    /// The index of the connection with this id.
    pub(crate) fn position(&self, connection: &str) -> Option<usize> {
        self.connections
            .iter()
            .position(|n| n.record.id == connection)
    }

    /// The selected row, if there are any.
    pub(crate) fn selected_row(&self) -> Option<VisibleRow> {
        self.rows
            .get(self.selected.min(self.rows.len().saturating_sub(1)))
            .copied()
    }
}

/// The query editor. `ratatui-textarea` keeps the text, the cursor, the
/// selection and the undo history; the view draws it itself.
#[derive(Debug, Clone, Default)]
pub(crate) struct EditorState {
    pub(crate) textarea: TextArea<'static>,
    /// The first row and display column in view; kept by the view.
    pub(crate) scroll: Cell<(usize, usize)>,
    /// Rows that fit, from the last draw, for PageUp/PageDown.
    pub(crate) page: Cell<usize>,
}

impl EditorState {
    pub(crate) fn query(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// The byte offset of `(row, col)`, a char column, in [`query`](Self::query).
    pub(crate) fn offset_of(&self, (row, col): (usize, usize)) -> usize {
        let lines = self.textarea.lines();
        let before: usize = lines.iter().take(row).map(|l| l.len() + 1).sum();
        let line = lines.get(row).map_or("", String::as_str);
        before
            + line
                .char_indices()
                .nth(col)
                .map_or(line.len(), |(byte, _)| byte)
    }

    /// The `(row, col)` of a byte offset into [`query`](Self::query).
    pub(crate) fn position_of(&self, offset: usize) -> (usize, usize) {
        let lines = self.textarea.lines();
        let mut start = 0;
        for (row, line) in lines.iter().enumerate() {
            let end = start + line.len();
            if offset <= end {
                let mut byte = (offset - start).min(line.len());
                while !line.is_char_boundary(byte) {
                    byte -= 1;
                }
                return (row, line[..byte].chars().count());
            }
            start = end + 1;
        }
        let last = lines.len().saturating_sub(1);
        (last, lines.last().map_or(0, |l| l.chars().count()))
    }

    /// The cursor's byte offset into [`query`](Self::query).
    pub(crate) fn cursor_offset(&self) -> usize {
        let cursor = self.textarea.cursor();
        self.offset_of((cursor.0, cursor.1))
    }
}

/// Compares what was typed, where the cursor is and what is selected; the
/// styling and the undo history are not part of it.
impl PartialEq for EditorState {
    fn eq(&self, other: &Self) -> bool {
        self.textarea.lines() == other.textarea.lines()
            && self.textarea.cursor() == other.textarea.cursor()
            && self.textarea.selection_range() == other.textarea.selection_range()
    }
}

/// The results grid.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ResultsState {
    pub(crate) result: Option<Arc<QueryResult>>,
    /// Display width of each column: its widest value, capped.
    pub(crate) widths: Vec<u16>,
    /// The selected cell.
    pub(crate) row: usize,
    pub(crate) col: usize,
    /// First row and column in view; kept by the view.
    pub(crate) row_offset: Cell<usize>,
    pub(crate) col_offset: Cell<usize>,
    /// Data rows that fit on screen, from the last draw. Used by
    /// PageUp/PageDown.
    pub(crate) page_rows: Cell<usize>,
    /// The sort keys, most important first, and the rows' order under
    /// them as indexes into the result; `None` while unsorted.
    pub(crate) sort: Vec<(usize, qry_core::grid::Order)>,
    pub(crate) order: Option<Vec<u32>>,
    /// The search in this result, if one was started.
    pub(crate) search: Option<GridSearch>,
    /// The first column stays put while the others scroll.
    pub(crate) frozen: bool,
    /// The inspector's count of the cursor's column, under way or done.
    pub(crate) profile: Option<qry_core::inspect::Profiler>,
    /// The inspector's first RECORD field in view; kept by the view.
    pub(crate) record_scroll: Cell<usize>,
}

impl ResultsState {
    /// The result row shown at grid row `row`, through the sort order.
    pub(crate) fn data_row(&self, row: usize) -> usize {
        self.order
            .as_ref()
            .and_then(|order| order.get(row))
            .map_or(row, |&i| i as usize)
    }

    /// The grid row a result row is shown at.
    pub(crate) fn grid_row(&self, data_row: usize) -> usize {
        self.order
            .as_ref()
            .and_then(|order| order.iter().position(|&i| i as usize == data_row))
            .unwrap_or(data_row)
    }
}

/// A search in a result: what is typed, and what it has found so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridSearch {
    /// Still being typed: the keys go to the query line.
    pub(crate) typing: bool,
    pub(crate) input: String,
    pub(crate) search: qry_core::grid::Search,
}

/// The help overlay: what it lists, built from the keymap when it opens.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HelpState {
    pub(crate) groups: Vec<crate::help_model::HelpGroup>,
    /// The cheat-sheet pinned at the bottom.
    pub(crate) footer: Vec<crate::help_model::FooterItem>,
    /// The help's own keys, for its bottom border.
    pub(crate) keys: Vec<(String, &'static str)>,
    /// First line in view; the view keeps it within `max_scroll`.
    pub(crate) scroll: Cell<usize>,
    /// Lines that fit, and the largest useful `scroll`, from the last draw.
    pub(crate) page: Cell<usize>,
    pub(crate) max_scroll: Cell<usize>,
}

/// The value popup: one cell's whole value, laid out for the popup's width
/// when it opens, when `w` or `f` is pressed and when the terminal resizes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ValuePopup {
    /// `users.notes · text · row 3`.
    pub(crate) title: String,
    /// The value in full, for copying.
    pub(crate) value: Option<String>,
    pub(crate) kind: qry_core::ValueKind,
    /// It is shown as JSON, so `f` switches between pretty and raw.
    pub(crate) json: bool,
    pub(crate) wrap: bool,
    pub(crate) pretty: bool,
    pub(crate) rendered: qry_core::inspect::Rendered,
    /// The first line and display column in view.
    pub(crate) scroll: usize,
    pub(crate) hscroll: usize,
}
