//! The app's state, as plain data.
//!
//! Everything the views draw is here, and only the reducer changes it. It is
//! `Clone + PartialEq`, so a test can replay the same events into two copies
//! and compare them. The few [`Cell`]s are scroll positions the views keep
//! up to date as they draw (what fits depends on the terminal), which the
//! reducer reads back for paging.

use std::{
    cell::Cell,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use crossterm::event::KeyEvent;
use qry_core::QueryResult;
use ratatui_textarea::TextArea;

use super::dialogs::DialogState;
use crate::{
    config::Config,
    connections::StoredConnection,
    keymap::KeyContext,
    theme::{Pill, Theme},
};

/// One open connection's worker. There is a single session for now, so the
/// id is always [`SessionId::FIRST`]; several come with multiple sessions.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

impl SessionId {
    pub const FIRST: Self = Self(0);
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
/// since been closed cannot act on whatever is open now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Awaiting {
    Connect,
    Export,
    Import,
    ExportConnections,
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
    /// At most one dialog and one help, the help always on top.
    pub(crate) overlays: Vec<Overlay>,
    /// What the open modal is waiting for the worker to answer, if anything.
    pub(crate) awaiting: Option<Awaiting>,
    /// The session the editor and the tree talk to.
    pub(crate) session: SessionId,
    pub(crate) tree: TreeState,
    pub(crate) editor: EditorState,
    pub(crate) results: ResultsState,
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
            overlays: Vec::new(),
            awaiting: None,
            session: SessionId::FIRST,
            tree: TreeState::default(),
            editor: EditorState::default(),
            results: ResultsState::default(),
            status: StatusCode::default(),
            pending_keys: Vec::new(),
            key_timer: 0,
            ids: IdGen::default(),
            dirty: true,
        }
    }
}

impl State {
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

/// The connection tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TreeState {
    pub(crate) connections: Vec<Node>,
    /// The connection the worker is on.
    pub(crate) active: Option<usize>,
    /// The connection whose `Connect` was sent and has not been answered.
    pub(crate) attempting: Option<usize>,
    /// Expand this one as soon as it is connected.
    pub(crate) expand_when_connected: Option<usize>,
    pub(crate) selected: usize,
    /// First row in view; kept by the view.
    pub(crate) offset: Cell<usize>,
}

impl TreeState {
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
