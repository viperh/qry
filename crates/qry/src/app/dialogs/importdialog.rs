//! The import preview: everything the picked file held, before anything
//! is added. New connections (`+`) are selected and Space takes one out or
//! puts it back; `a` / `A` select or clear them all. Those already here
//! (`=`) and those qry could not read (`!`) are listed but cannot be picked.
//! Enter adds the selection and saves once.

use std::cell::Cell;

use super::{
    Submission,
    form::{Field, Form},
};
use crate::{
    action::Action,
    connections::StoredConnection,
    import::{Format, Imported},
};

/// How an entry of the file stands against the saved connections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    New,
    /// The same id, or the same database as the same user.
    Existing,
    Skipped(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    /// `postgres · alice@db.local`: what it points at.
    pub(crate) detail: String,
    pub(crate) kind: EntryKind,
    pub(crate) selected: bool,
    pub(crate) record: Option<StoredConnection>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImportPreview {
    pub(crate) format: Format,
    pub(crate) entries: Vec<Entry>,
    pub(crate) warnings: Vec<String>,
    /// The entry the keys act on.
    pub(crate) cursor: usize,
    /// First entry in view, and how many fit; kept by the view.
    pub(crate) scroll: Cell<usize>,
    pub(crate) page: Cell<usize>,
    error: Option<String>,
}

impl ImportPreview {
    /// Sorts what `imported` held against `known`, the saved connections:
    /// new first in the file's order, then those already here, then the
    /// skipped.
    pub(crate) fn new(imported: Imported, known: &[StoredConnection]) -> Self {
        let mut new = Vec::new();
        let mut existing = Vec::new();
        for record in imported.connections {
            let here = known
                .iter()
                .any(|k| k.id == record.id || k.same_target(&record))
                // Twice in the one file is still one connection.
                || new.iter().any(|e: &Entry| {
                    e.record
                        .as_ref()
                        .is_some_and(|r| r.id == record.id || r.same_target(&record))
                });
            let entry = Entry {
                name: record.title().to_string(),
                detail: crate::app::state::summary(&record),
                kind: if here {
                    EntryKind::Existing
                } else {
                    EntryKind::New
                },
                selected: !here,
                record: Some(record),
            };
            if here {
                existing.push(entry);
            } else {
                new.push(entry);
            }
        }
        let skipped = imported.skipped.into_iter().map(|s| Entry {
            name: s.name,
            detail: String::new(),
            kind: EntryKind::Skipped(s.reason),
            selected: false,
            record: None,
        });
        let entries = new.into_iter().chain(existing).chain(skipped).collect();
        Self {
            format: imported.format,
            entries,
            warnings: imported.warnings,
            cursor: 0,
            scroll: Cell::new(0),
            page: Cell::new(10),
            error: None,
        }
    }

    /// Selects every new connection, or none.
    pub(crate) fn select_all(&mut self, selected: bool) {
        for entry in &mut self.entries {
            if entry.kind == EntryKind::New {
                entry.selected = selected;
            }
        }
    }

    pub(crate) fn count(&self, kind: fn(&EntryKind) -> bool) -> usize {
        self.entries.iter().filter(|e| kind(&e.kind)).count()
    }

    /// How many are selected.
    pub(crate) fn selected(&self) -> usize {
        self.entries.iter().filter(|e| e.selected).count()
    }
}

impl Form for ImportPreview {
    fn title(&self) -> &'static str {
        "Import Preview"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "import selected"),
            (Action::NextChoice, "toggle"),
            (Action::Cancel, "cancel"),
        ]
    }

    fn width(&self) -> u16 {
        96
    }

    fn subtitle(&self) -> Option<String> {
        Some(format!("{} · {} found", self.format, self.entries.len()))
    }

    fn labels(&self) -> &[&'static str] {
        &[]
    }

    fn fields(&self) -> &[Field] {
        &[]
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut []
    }

    fn focus(&self) -> usize {
        0
    }

    fn set_focus(&mut self, _focus: usize) {}

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn submit(&self) -> Result<Submission, String> {
        let records = self
            .entries
            .iter()
            .filter(|e| e.selected)
            .filter_map(|e| e.record.clone())
            .collect();
        Ok(Submission::ImportSelected(records))
    }

    fn act(&mut self, action: Action) {
        let last = self.entries.len().saturating_sub(1);
        let page = self.page.get().max(1);
        match action {
            Action::Up | Action::PrevField => self.cursor = self.cursor.saturating_sub(1),
            Action::Down | Action::NextField => self.cursor = (self.cursor + 1).min(last),
            Action::PageUp => self.cursor = self.cursor.saturating_sub(page),
            Action::PageDown => self.cursor = (self.cursor + page).min(last),
            Action::Top | Action::LineStart => self.cursor = 0,
            Action::Bottom | Action::LineEnd => self.cursor = last,
            Action::NextChoice => {
                if let Some(entry) = self.entries.get_mut(self.cursor)
                    && entry.kind == EntryKind::New
                {
                    entry.selected = !entry.selected;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "importdialog_tests.rs"]
mod importdialog_tests;
