use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, IntoEnumIterator};

/// The one command vocabulary: what a key in the `keybindings` config can be
/// bound to, in any [`KeyContext`](crate::keymap::KeyContext). The reducer
/// ([`App::handle`](crate::app::App::handle)) interprets these; nothing sends
/// them around as messages. The help lists them in declaration order.
///
/// Some have no effect yet and are bound already so the defaults do not
/// change under users later: the results' copy, search and inspector (T303, T304) and
/// the jobs drawer (T305).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, Serialize, Deserialize, EnumIter)]
pub enum Action {
    Quit,
    Suspend,
    ClearScreen,
    Help,
    Palette,
    FocusNext,
    FocusPrev,
    FocusTree,
    FocusEditor,
    FocusResults,

    NewConnection,
    EditConnection,
    DeleteConnection,
    /// Opens the connection dialog on a copy of the selected connection,
    /// under a new id and without its password.
    DuplicateConnection,
    ImportConnections,
    ExportConnections,

    /// The session with this number, from 1.
    SwitchSession(u8),
    NextSession,
    PrevSession,
    CloseSession,

    RunStatement,
    RunAll,
    RunSelection,
    /// Stops the running query; in the editor, copies when none runs.
    CancelQuery,
    /// Ends the session's transaction, keeping or undoing its changes.
    Commit,
    Rollback,
    /// Turns the session's autocommit off (statements wait for a commit)
    /// or back on.
    ToggleAutocommit,
    ExportResults,
    /// Opens the query history.
    History,
    /// On an empty editor, puts this connection's previous or next
    /// statement from the history in it, like a shell.
    RecallPrev,
    RecallNext,
    /// Saves the statement under the cursor as a named snippet.
    SaveSnippet,
    /// Lists the snippets, to rename, delete or edit them.
    ManageSnippets,
    /// In the history and the palette: insert what is picked and run it.
    SubmitAndRun,
    /// In the history: delete the picked entry (pressed twice).
    DeleteEntry,

    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Top,
    Bottom,
    LineStart,
    LineEnd,

    Toggle,
    Open,
    /// Opens another session on the selected connection, even if one is
    /// open already.
    OpenNewSession,
    Refresh,
    /// Reads everything of the selected connection again.
    RefreshAll,
    Filter,
    /// Shows or hides the system catalogs in the tree.
    ToggleSystemObjects,
    /// Puts `SELECT * FROM` the selected table in the editor, without
    /// running it.
    InsertSelect,
    /// Copies the selected table's qualified name.
    CopyName,
    /// Closes the selected connection's session.
    Disconnect,
    Search,
    CopyCell,
    CopyRow,
    /// Shows the result tab before or after this one.
    PrevResult,
    NextResult,
    /// Sorts by the current column (ascending, descending, off), or adds
    /// it as one more sort key.
    SortColumn,
    AddSortColumn,
    NextMatch,
    PrevMatch,
    /// The copy menu: the cell, row, column or whole result, as CSV, TSV,
    /// JSON, Markdown or SQL INSERTs.
    CopyMenu,
    GotoRow,
    NarrowColumn,
    WidenColumn,
    ResetWidths,
    /// Keeps the first column in view while the others scroll.
    FreezeColumn,
    /// Reads the next rows of a truncated result from its kept cursor,
    /// without running the statement again.
    FetchMore,
    /// Runs a truncated result's statement again, for every row.
    FetchAll,
    Inspect,
    ToggleInspector,
    ToggleJobs,
    ToggleHints,
    /// Gives the editor 5% more of the right-hand side, or less.
    GrowEditor,
    ShrinkEditor,
    /// Maximises the focused right-hand region, or restores the split.
    ToggleMaximise,
    /// Opens the Messages dialog: every notice so far.
    ShowMessages,
    /// Takes the toast away before its time.
    DismissToast,

    Cancel,
    Submit,
    /// Tries the connection dialog's connection without saving it.
    TestConnection,
    NextField,
    PrevField,
    /// Only acts on a dialog's choice fields; text fields type the key.
    NextChoice,

    // The editor's text commands.
    NewLine,
    Indent,
    /// Takes one indent off the start of the line.
    Dedent,
    WordLeft,
    WordRight,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectLineStart,
    SelectLineEnd,
    SelectAll,
    DeleteBack,
    DeleteForward,
    DeleteWordBack,
    DeleteWordForward,
    DeleteToLineEnd,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
}

impl Action {
    /// One line for the help and the palette.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::Suspend => "Suspend to the shell",
            Self::ClearScreen => "Redraw the screen",
            Self::Help => "Show / hide the help",
            Self::Palette => "Command palette",
            Self::FocusNext => "Next pane",
            Self::FocusPrev => "Previous pane",
            Self::FocusTree => "Go to the connection tree",
            Self::FocusEditor => "Go to the editor",
            Self::FocusResults => "Go to the results",
            Self::NewConnection => "New connection",
            Self::EditConnection => "Edit this connection",
            Self::DeleteConnection => "Forget this connection",
            Self::DuplicateConnection => "Duplicate this connection",
            Self::ImportConnections => "Import connections",
            Self::ExportConnections => "Export connections",
            Self::SwitchSession(_) => "Switch to session N",
            Self::NextSession => "Next session",
            Self::PrevSession => "Previous session",
            Self::CloseSession => "Close this session",
            Self::RunStatement => "Run the statement",
            Self::RunAll => "Run everything",
            Self::RunSelection => "Run the selection",
            Self::CancelQuery => "Cancel the query (copy when none runs)",
            Self::Commit => "Commit the transaction",
            Self::Rollback => "Roll the transaction back",
            Self::ToggleAutocommit => "Turn autocommit off / on",
            Self::ExportResults => "Export the result to a file",
            Self::History => "Query history",
            Self::RecallPrev => "Previous statement from the history",
            Self::RecallNext => "Next statement from the history",
            Self::SaveSnippet => "Save the statement as a snippet",
            Self::ManageSnippets => "Manage the saved snippets",
            Self::SubmitAndRun => "Insert and run",
            Self::DeleteEntry => "Delete the entry",
            Self::Up => "Up",
            Self::Down => "Down",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::PageUp => "Up a page",
            Self::PageDown => "Down a page",
            Self::Top => "To the top",
            Self::Bottom => "To the bottom",
            Self::LineStart => "Start of the line",
            Self::LineEnd => "End of the line",
            Self::Toggle => "Expand or collapse",
            Self::Open => "Open",
            Self::OpenNewSession => "Open another session on it",
            Self::Refresh => "Refresh",
            Self::RefreshAll => "Refresh the whole connection",
            Self::Filter => "Filter",
            Self::ToggleSystemObjects => "Show / hide system objects",
            Self::InsertSelect => "Insert SELECT * FROM the table",
            Self::CopyName => "Copy the qualified name",
            Self::Disconnect => "Disconnect its session",
            Self::Search => "Search",
            Self::CopyCell => "Copy the cell",
            Self::PrevResult => "Previous result",
            Self::NextResult => "Next result",
            Self::SortColumn => "Sort by the column",
            Self::AddSortColumn => "Add the column to the sort",
            Self::NextMatch => "Next match",
            Self::PrevMatch => "Previous match",
            Self::CopyMenu => "Copy as…",
            Self::GotoRow => "Go to a row",
            Self::NarrowColumn => "Narrow the column",
            Self::WidenColumn => "Widen the column",
            Self::ResetWidths => "Reset the column widths",
            Self::FreezeColumn => "Freeze the first column",
            Self::FetchMore => "Load the next rows",
            Self::FetchAll => "Fetch every row",
            Self::CopyRow => "Copy the row",
            Self::Inspect => "Inspect the value",
            Self::ToggleInspector => "Show / hide the inspector",
            Self::ToggleJobs => "Show / hide the jobs",
            Self::ToggleHints => "Show / hide the key hints",
            Self::GrowEditor => "Grow the editor",
            Self::ShrinkEditor => "Shrink the editor",
            Self::ToggleMaximise => "Maximise the pane / restore",
            Self::ShowMessages => "Show the messages",
            Self::DismissToast => "Dismiss the message",
            Self::Cancel => "Cancel / close",
            Self::Submit => "Submit",
            Self::TestConnection => "Test the connection",
            Self::NextField => "Next field",
            Self::PrevField => "Previous field",
            Self::NextChoice => "Next choice",
            Self::NewLine => "New line",
            Self::Indent => "Indent",
            Self::Dedent => "Dedent",
            Self::WordLeft => "Previous word",
            Self::WordRight => "Next word",
            Self::SelectLeft => "Select left",
            Self::SelectRight => "Select right",
            Self::SelectUp => "Select up",
            Self::SelectDown => "Select down",
            Self::SelectWordLeft => "Select the previous word",
            Self::SelectWordRight => "Select the next word",
            Self::SelectLineStart => "Select to the start of the line",
            Self::SelectLineEnd => "Select to the end of the line",
            Self::SelectAll => "Select all",
            Self::DeleteBack => "Delete the character before",
            Self::DeleteForward => "Delete the character after",
            Self::DeleteWordBack => "Delete the word before",
            Self::DeleteWordForward => "Delete the word after",
            Self::DeleteToLineEnd => "Delete to the end of the line",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
        }
    }

    /// What the command palette calls it, for the actions a user would look
    /// for there; `None` for moving about, editing text and answering a
    /// dialog, which only make sense on a key.
    pub fn palette_label(&self) -> Option<&'static str> {
        Some(match self {
            Self::Quit => "Quit",
            Self::Suspend => "Suspend to the shell",
            Self::ClearScreen => "Redraw the screen",
            Self::Help => "Help",
            Self::FocusTree => "Go to the tree",
            Self::FocusEditor => "Go to the editor",
            Self::FocusResults => "Go to the results",
            Self::NewConnection => "New connection",
            Self::EditConnection => "Edit connection",
            Self::DeleteConnection => "Delete connection",
            Self::DuplicateConnection => "Duplicate connection",
            Self::ImportConnections => "Import connections",
            Self::ExportConnections => "Export connections…",
            Self::OpenNewSession => "Open another session",
            Self::NextSession => "Next session",
            Self::PrevSession => "Previous session",
            Self::CloseSession => "Close session",
            Self::RunStatement => "Run statement",
            Self::RunAll => "Run all",
            Self::RunSelection => "Run selection",
            Self::CancelQuery => "Cancel query",
            Self::Commit => "Commit",
            Self::Rollback => "Rollback",
            Self::ToggleAutocommit => "Toggle autocommit",
            Self::ExportResults => "Export result…",
            Self::History => "History",
            Self::SaveSnippet => "Save snippet",
            Self::ManageSnippets => "Manage snippets",
            Self::Refresh => "Refresh node",
            Self::RefreshAll => "Refresh connection",
            Self::Filter => "Filter the tree",
            Self::ToggleSystemObjects => "Toggle system objects",
            Self::InsertSelect => "Insert SELECT for table",
            Self::CopyName => "Copy qualified name",
            Self::Disconnect => "Disconnect",
            Self::Search => "Search the results",
            Self::CopyCell => "Copy cell",
            Self::PrevResult => "Previous result",
            Self::NextResult => "Next result",
            Self::SortColumn => "Sort by the column",
            Self::AddSortColumn => "Add the column to the sort",
            Self::NextMatch => "Next match",
            Self::PrevMatch => "Previous match",
            Self::CopyMenu => "Copy as…",
            Self::GotoRow => "Go to a row",
            Self::NarrowColumn => "Narrow the column",
            Self::WidenColumn => "Widen the column",
            Self::ResetWidths => "Reset the column widths",
            Self::FreezeColumn => "Freeze the first column",
            Self::FetchMore => "Load more rows",
            Self::FetchAll => "Fetch every row",
            Self::CopyRow => "Copy row",
            Self::Inspect => "Inspect value",
            Self::ToggleInspector => "Toggle inspector",
            Self::ToggleJobs => "Toggle jobs",
            Self::ToggleHints => "Toggle hints",
            Self::GrowEditor => "Grow editor",
            Self::ShrinkEditor => "Shrink editor",
            Self::ToggleMaximise => "Maximise pane",
            Self::ShowMessages => "Show messages",
            _ => return None,
        })
    }

    /// The palette's category tag for it.
    pub fn palette_category(&self) -> &'static str {
        match self {
            Self::NewConnection
            | Self::EditConnection
            | Self::DeleteConnection
            | Self::DuplicateConnection
            | Self::ImportConnections
            | Self::ExportConnections
            | Self::Refresh
            | Self::RefreshAll
            | Self::Filter
            | Self::ToggleSystemObjects
            | Self::InsertSelect
            | Self::CopyName
            | Self::Disconnect => "connection",
            Self::OpenNewSession | Self::NextSession | Self::PrevSession | Self::CloseSession => {
                "session"
            }
            Self::RunStatement
            | Self::RunAll
            | Self::RunSelection
            | Self::CancelQuery
            | Self::Commit
            | Self::Rollback
            | Self::ToggleAutocommit => "query",
            Self::History | Self::SaveSnippet | Self::ManageSnippets => "query",
            Self::ExportResults
            | Self::Search
            | Self::CopyCell
            | Self::CopyRow
            | Self::PrevResult
            | Self::NextResult
            | Self::SortColumn
            | Self::AddSortColumn
            | Self::NextMatch
            | Self::PrevMatch
            | Self::CopyMenu
            | Self::GotoRow
            | Self::NarrowColumn
            | Self::WidenColumn
            | Self::ResetWidths
            | Self::FreezeColumn
            | Self::FetchMore
            | Self::FetchAll
            | Self::Inspect
            | Self::ToggleInspector => "results",
            Self::FocusTree
            | Self::FocusEditor
            | Self::FocusResults
            | Self::ToggleJobs
            | Self::ToggleHints
            | Self::GrowEditor
            | Self::ShrinkEditor
            | Self::ToggleMaximise
            | Self::ShowMessages
            | Self::ClearScreen => "view",
            _ => "app",
        }
    }

    /// The pane whose keys an action belongs to, for running it from the
    /// palette as its key would: the tree's actions act on the selected
    /// connection, the editor's on its text, the results' on the grid.
    pub(crate) fn home_context(&self) -> crate::keymap::KeyContext {
        use crate::keymap::KeyContext;
        match self {
            Self::EditConnection
            | Self::DeleteConnection
            | Self::DuplicateConnection
            | Self::OpenNewSession
            | Self::Refresh
            | Self::RefreshAll
            | Self::Filter
            | Self::ToggleSystemObjects
            | Self::InsertSelect
            | Self::CopyName
            | Self::Disconnect => KeyContext::Tree,
            Self::RunStatement | Self::RunAll | Self::RunSelection | Self::CancelQuery => {
                KeyContext::Editor
            }
            Self::History | Self::SaveSnippet | Self::RecallPrev | Self::RecallNext => {
                KeyContext::Editor
            }
            Self::Search
            | Self::CopyCell
            | Self::CopyRow
            | Self::Inspect
            | Self::PrevResult
            | Self::NextResult
            | Self::SortColumn
            | Self::AddSortColumn
            | Self::NextMatch
            | Self::PrevMatch
            | Self::CopyMenu
            | Self::GotoRow
            | Self::NarrowColumn
            | Self::WidenColumn
            | Self::ResetWidths
            | Self::FreezeColumn
            | Self::FetchMore
            | Self::FetchAll => KeyContext::Results,
            _ => KeyContext::Global,
        }
    }

    /// Where the variant is declared, for listing actions in that order.
    /// Every `SwitchSession(n)` shares one place, after which `n` orders
    /// them.
    pub(crate) fn rank(&self) -> (usize, u8) {
        let n = match self {
            Self::SwitchSession(n) => *n,
            _ => 0,
        };
        let place = Self::iter()
            .position(|a| std::mem::discriminant(&a) == std::mem::discriminant(self))
            .unwrap_or(usize::MAX);
        (place, n)
    }
}
