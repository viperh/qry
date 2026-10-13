use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, IntoEnumIterator};

/// The one command vocabulary: what a key in the `keybindings` config can be
/// bound to, in any [`KeyContext`](crate::keymap::KeyContext). The reducer
/// ([`App::handle`](crate::app::App::handle)) interprets these; nothing sends
/// them around as messages. The help lists them in declaration order.
///
/// Some have no effect yet and are bound already so the defaults do not
/// change under users later: the palette (T206), filtering
/// the tree (T301), the results' copy, search and inspector (T303, T304) and
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
    ExportResults,

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
    Filter,
    Search,
    CopyCell,
    CopyRow,
    Inspect,
    ToggleInspector,
    ToggleJobs,
    ToggleHints,
    /// Gives the editor 5% more of the right-hand side, or less.
    GrowEditor,
    ShrinkEditor,
    /// Maximises the focused right-hand region, or restores the split.
    ToggleMaximise,

    Cancel,
    Submit,
    NextField,
    PrevField,
    /// Only acts on a dialog's choice fields; text fields type the key.
    NextChoice,

    // The editor's text commands.
    NewLine,
    Indent,
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
            Self::ExportResults => "Export the results to a file",
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
            Self::Filter => "Filter",
            Self::Search => "Search",
            Self::CopyCell => "Copy the cell",
            Self::CopyRow => "Copy the row",
            Self::Inspect => "Inspect the value",
            Self::ToggleInspector => "Show / hide the inspector",
            Self::ToggleJobs => "Show / hide the jobs",
            Self::ToggleHints => "Show / hide the key hints",
            Self::GrowEditor => "Grow the editor",
            Self::ShrinkEditor => "Shrink the editor",
            Self::ToggleMaximise => "Maximise the pane / restore",
            Self::Cancel => "Cancel / close",
            Self::Submit => "Submit",
            Self::NextField => "Next field",
            Self::PrevField => "Previous field",
            Self::NextChoice => "Next choice",
            Self::NewLine => "New line",
            Self::Indent => "Indent",
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
