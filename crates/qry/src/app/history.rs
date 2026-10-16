//! The query history and the saved snippets, as the app keeps them: every
//! connection's history, the history dialog and the snippet list. Plain
//! data; `reducer/history.rs` and `reducer/snippets.rs` change it and
//! `views/history.rs` draws it.

use std::{cell::Cell, cmp::Reverse, collections::BTreeMap};

use qry_core::history::History;

use super::{dialogs::textinput::TextInput, state::SessionId};

/// Every connection's history, by `StoredConnection::id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HistoryBook {
    pub(crate) by_connection: BTreeMap<String, Kept>,
    /// A write failed already this run, so later failures are not toasted
    /// again.
    pub(crate) write_failed: bool,
}

/// One connection's history, and how long its file has grown.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Kept {
    pub(crate) history: History,
    /// Lines in the file, merged duplicates included: an append adds one
    /// even when it only brought the newest entry up to date.
    pub(crate) lines: usize,
}

impl HistoryBook {
    pub(crate) fn get(&self, connection: &str) -> Option<&History> {
        self.by_connection.get(connection).map(|k| &k.history)
    }

    #[cfg(test)]
    pub(crate) fn len(&self, connection: &str) -> usize {
        self.get(connection).map_or(0, History::len)
    }
}

/// One row of the history dialog: which entry, and where the query matched
/// it (char indices into its SQL on one line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryRow {
    pub(crate) connection: String,
    pub(crate) index: usize,
    pub(crate) positions: Vec<usize>,
}

/// The open history dialog.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HistoryDialog {
    pub(crate) input: TextInput,
    /// Every connection's history, not only this one's.
    pub(crate) all: bool,
    /// The connection of the session it was opened from.
    pub(crate) connection: String,
    pub(crate) rows: Vec<HistoryRow>,
    pub(crate) selected: usize,
    /// The first row in view; kept by the view.
    pub(crate) offset: Cell<usize>,
    /// ctrl-d was pressed once on the selected row: the next deletes it.
    pub(crate) deleting: bool,
}

impl HistoryDialog {
    pub(crate) fn new(connection: String, book: &HistoryBook) -> Self {
        let mut dialog = Self {
            connection,
            ..Self::default()
        };
        dialog.refilter(book);
        dialog
    }

    /// Matches the query again, from the top: best first, then newest.
    pub(crate) fn refilter(&mut self, book: &HistoryBook) {
        let query = self.input.value.clone();
        let mut found: Vec<(i32, Option<i64>, HistoryRow)> = Vec::new();
        for (connection, kept) in &book.by_connection {
            if !self.all && *connection != self.connection {
                continue;
            }
            for (index, score, positions) in kept.history.search(&query) {
                let ts = kept.history.get(index).and_then(|e| e.ts);
                found.push((
                    score,
                    ts,
                    HistoryRow {
                        connection: connection.clone(),
                        index,
                        positions,
                    },
                ));
            }
        }
        // `search` already put one connection's in order; across
        // connections the time decides, and an entry without one goes after.
        found.sort_by_key(|(score, ts, row)| {
            (
                Reverse(*score),
                Reverse(ts.unwrap_or(i64::MIN)),
                row.connection.clone(),
                Reverse(row.index),
            )
        });
        self.rows = found.into_iter().map(|(_, _, row)| row).collect();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.deleting = false;
    }

    pub(crate) fn selected_row(&self) -> Option<&HistoryRow> {
        self.rows.get(self.selected)
    }

    pub(crate) fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.deleting = false;
    }
}

/// The Manage snippets list: every snippet, by index into
/// `State::snippets`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SnippetList {
    pub(crate) rows: Vec<usize>,
    pub(crate) selected: usize,
    /// The first row in view; kept by the view.
    pub(crate) offset: Cell<usize>,
}

/// A statement the editor took from the history with alt-↑/↓, so the next
/// step goes on from it as long as the text is still what was put in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recall {
    pub(crate) session: SessionId,
    /// Into the connection's history, from the oldest.
    pub(crate) index: usize,
    pub(crate) text: String,
}

/// The snippet the editor holds for editing (from Manage snippets): saving
/// with the same name and scope overwrites it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditingSnippet {
    pub(crate) session: SessionId,
    pub(crate) name: String,
    pub(crate) connection: Option<String>,
}
