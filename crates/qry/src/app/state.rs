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
    sync::Arc,
};

use crossterm::event::KeyEvent;
use qry_core::QueryResult;
use ratatui_textarea::TextArea;

use super::dialogs::DialogState;
pub use crate::theme::SessionStatus;
use crate::{
    config::Config,
    connections::{Driver, Secret, StoredConnection},
    keymap::KeyContext,
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
    pub(crate) results: ResultsState,
    /// Queries sent and not answered yet. One worker runs them in order.
    pub(crate) running: usize,
    /// What each of those does to the transaction, oldest first: `Some(true)`
    /// begins one, `Some(false)` ends it. Applied once it succeeds.
    pub(crate) tx_changes: VecDeque<Option<bool>>,
    /// A transaction was begun and not yet committed or rolled back.
    pub(crate) in_transaction: bool,
    /// Expand its connection in the tree once it is open.
    pub(crate) expand_when_connected: bool,
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
            results: ResultsState::default(),
            running: 0,
            tx_changes: VecDeque::new(),
            in_transaction: false,
            expand_when_connected: false,
        }
    }

    /// Takes on an edited record's name and target.
    pub(crate) fn retarget(&mut self, record: &StoredConnection) {
        let fresh = Self::new(self.id, record);
        self.title = fresh.title;
        self.driver = fresh.driver;
        self.summary = fresh.summary;
        self.database = fresh.database;
        self.in_transaction = false;
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.status, SessionStatus::Open { .. })
    }
}

/// `postgres · app@db.local`, `sqlite · :memory:`: the driver and where it
/// points, without the password or the port.
fn summary(record: &StoredConnection) -> String {
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

/// The status bar's line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StatusCode {
    #[default]
    None,
    Success(String),
    Error(String),
}

/// The pane the keys go to while nothing is on top of the panes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    #[default]
    Tree,
    Editor,
    Results,
}

impl Focus {
    pub(crate) fn next(self) -> Self {
        match self {
            Focus::Tree => Focus::Editor,
            Focus::Editor => Focus::Results,
            Focus::Results => Focus::Tree,
        }
    }

    pub(crate) fn prev(self) -> Self {
        match self {
            Focus::Tree => Focus::Results,
            Focus::Editor => Focus::Tree,
            Focus::Results => Focus::Editor,
        }
    }

    /// The pane's own key bindings.
    pub(crate) fn context(self) -> KeyContext {
        match self {
            Focus::Tree => KeyContext::Tree,
            Focus::Editor => KeyContext::Editor,
            Focus::Results => KeyContext::Results,
        }
    }
}

/// Something drawn over the panes. The last one in [`State::overlays`] is on
/// top and gets the keys.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Overlay {
    Dialog(DialogState),
    Help(HelpState),
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
    pub(crate) status: StatusCode,
    /// The start of a multi-key binding typed so far, normalised.
    pub(crate) pending_keys: Vec<KeyEvent>,
    /// Which `Timer::KeySequence` is the live one: each key that leaves a
    /// sequence waiting starts a new one, and an older one that fires is
    /// ignored.
    pub(crate) key_timer: u64,
    pub(crate) ids: IdGen,
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
            status: StatusCode::default(),
            pending_keys: Vec::new(),
            key_timer: 0,
            ids: IdGen::default(),
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
            jobs_rows: 0,
            hints: self.chrome.hints,
        }
    }

    /// The terminal is too small to draw the main screen.
    pub(crate) fn too_small(&self) -> bool {
        let (min_w, min_h) = crate::views::layout::MIN_SIZE;
        self.size.0 < min_w || self.size.1 < min_h
    }

    /// The active session's editor.
    pub(crate) fn editor(&self) -> &EditorState {
        &self.sessions.active().editor
    }

    pub(crate) fn editor_mut(&mut self) -> &mut EditorState {
        &mut self.sessions.active_mut().editor
    }

    /// The active session's results.
    pub(crate) fn results(&self) -> &ResultsState {
        &self.sessions.active().results
    }

    pub(crate) fn results_mut(&mut self) -> &mut ResultsState {
        &mut self.sessions.active_mut().results
    }

    pub(crate) fn dialog(&self) -> Option<&DialogState> {
        self.overlays.iter().find_map(|overlay| match overlay {
            Overlay::Dialog(dialog) => Some(dialog),
            Overlay::Help(_) => None,
        })
    }

    pub(crate) fn dialog_mut(&mut self) -> Option<&mut DialogState> {
        self.overlays.iter_mut().find_map(|overlay| match overlay {
            Overlay::Dialog(dialog) => Some(dialog),
            Overlay::Help(_) => None,
        })
    }

    pub(crate) fn help(&self) -> Option<&HelpState> {
        self.overlays.iter().find_map(|overlay| match overlay {
            Overlay::Help(help) => Some(help),
            Overlay::Dialog(_) => None,
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
            Some(Overlay::Help(_)) => vec![KeyContext::Help],
            Some(Overlay::Dialog(_)) => vec![KeyContext::Dialog],
            None => vec![self.focus.context(), KeyContext::Global],
        }
    }

    /// The mode pill: that of the topmost overlay that has one, else the
    /// focused pane's. The help and the dialogs have none, so they keep
    /// the pill of what is underneath.
    pub fn pill(&self) -> Pill {
        let overlays = self.overlays.iter().rev().map(|overlay| match overlay {
            Overlay::Help(_) => KeyContext::Help,
            Overlay::Dialog(_) => KeyContext::Dialog,
        });
        overlays
            .chain([self.focus.context()])
            .find_map(KeyContext::pill)
            .unwrap_or(Pill::Normal)
    }
}

/// A connection the tree knows about, and what it has learned of it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub(crate) record: StoredConnection,
    /// What the database called itself; `None` until it has connected once.
    pub(crate) label: Option<String>,
    pub(crate) expanded: bool,
    pub(crate) tables: Option<Vec<String>>,
    pub(crate) loading: bool,
    /// Added this session and not written to the file yet.
    pub(crate) unsaved: bool,
}

impl Node {
    pub(crate) fn new(record: StoredConnection, unsaved: bool) -> Self {
        Self {
            record,
            label: None,
            expanded: false,
            tables: None,
            loading: false,
            unsaved,
        }
    }

    pub(crate) fn title(&self) -> &str {
        self.record.title()
    }
}

/// One drawn line of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Connection(usize),
    Table(usize, usize),
    /// A word under a connection, such as while its tables load.
    Note(&'static str),
}

/// The connection tree: the stored records and what is expanded. Which of
/// them are open is the sessions' business.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TreeState {
    pub(crate) connections: Vec<Node>,
    pub(crate) selected: usize,
    /// First row in view; kept by the view.
    pub(crate) offset: Cell<usize>,
}

impl TreeState {
    /// The index of the connection with this id.
    pub(crate) fn position(&self, connection: &str) -> Option<usize> {
        self.connections
            .iter()
            .position(|n| n.record.id == connection)
    }

    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, connection) in self.connections.iter().enumerate() {
            rows.push(Row::Connection(i));
            if !connection.expanded {
                continue;
            }
            match (&connection.tables, connection.loading) {
                (_, true) => rows.push(Row::Note("loading…")),
                (Some(tables), _) if tables.is_empty() => rows.push(Row::Note("(no tables)")),
                (Some(tables), _) => rows.extend((0..tables.len()).map(|t| Row::Table(i, t))),
                (None, _) => rows.push(Row::Note("(not connected)")),
            }
        }
        rows
    }
}

/// The query editor.
#[derive(Debug, Clone, Default)]
pub(crate) struct EditorState {
    /// Styled by the reducer for the focus and theme, since drawing it must
    /// not clone it: it keeps its own scroll position as it draws.
    pub(crate) textarea: TextArea<'static>,
}

impl EditorState {
    pub(crate) fn query(&self) -> String {
        self.textarea.lines().join("\n")
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
}

/// The help popup.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HelpState {
    /// `(title, [(keys, description)])`, in display order.
    pub(crate) sections: Vec<(String, Vec<(String, String)>)>,
    /// First line in view; the view keeps it within `max_scroll`.
    pub(crate) scroll: Cell<usize>,
    /// Lines that fit, and the largest useful `scroll`, from the last draw.
    pub(crate) page: Cell<usize>,
    pub(crate) max_scroll: Cell<usize>,
}
