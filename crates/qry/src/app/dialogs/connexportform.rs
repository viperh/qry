use super::{
    Submission,
    form::{Field, Form},
};
use crate::{action::Action, theme::Accent};

const FILE: usize = 0;

/// The Export Connections form: where to write a backup that Import
/// Connections can read back, and which connections go in it (all of them
/// to start with; Space takes one out).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConnExportForm {
    /// The file, then one checkbox per connection.
    fields: Vec<Field>,
    labels: Vec<&'static str>,
    /// The connections' ids, in the checkboxes' order.
    ids: Vec<String>,
    focus: usize,
    error: Option<String>,
    /// Set while the file is being written.
    pending: Option<&'static str>,
}

impl ConnExportForm {
    /// A form offering these `(id, name)`s, all included.
    pub(crate) fn new(connections: &[(String, String)]) -> Self {
        let mut fields = vec![Field::text_with_placeholder("~/qry-connections.json")];
        let mut labels = vec!["File"];
        for (i, (_, name)) in connections.iter().enumerate() {
            fields.push(Field::checkbox(true, name.clone()));
            labels.push(if i == 0 { "Include" } else { "" });
        }
        Self {
            fields,
            labels,
            ids: connections.iter().map(|(id, _)| id.clone()).collect(),
            focus: FILE,
            error: None,
            pending: None,
        }
    }

    /// The ids of the connections still ticked.
    pub(crate) fn included(&self) -> Vec<String> {
        self.ids
            .iter()
            .enumerate()
            .filter(|(i, _)| self.checked(i + 1))
            .map(|(_, id)| id.clone())
            .collect()
    }
}

impl Form for ConnExportForm {
    fn title(&self) -> &'static str {
        "Export Connections"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "export"),
            (Action::NextChoice, "toggle"),
            (Action::NextField, "next"),
            (Action::Cancel, "cancel"),
        ]
    }

    fn width(&self) -> u16 {
        64
    }

    fn accent(&self) -> Accent {
        Accent::Act
    }

    fn subtitle(&self) -> Option<String> {
        Some("passwords are not included".into())
    }

    fn labels(&self) -> &[&'static str] {
        &self.labels
    }

    fn fields(&self) -> &[Field] {
        &self.fields
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    fn focus(&self) -> usize {
        self.focus
    }

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus.min(self.fields.len() - 1);
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn pending(&self) -> Option<&'static str> {
        self.pending
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        self.pending = pending;
    }

    fn submit(&self) -> Result<Submission, String> {
        let path = self.text(FILE).trim().to_string();
        if path.is_empty() {
            return Err("File is required".into());
        }
        if self.ids.is_empty() {
            return Err("There are no saved connections to export".into());
        }
        let ids = self.included();
        if ids.is_empty() {
            return Err("Choose at least one connection to export".into());
        }
        Ok(Submission::ExportConnections { path, ids })
    }

    /// Up and down move between the rows too, so a long list reads like
    /// one.
    fn act(&mut self, action: Action) {
        let count = self.fields.len();
        match action {
            Action::NextField | Action::Down => self.focus = (self.focus + 1) % count,
            Action::PrevField | Action::Up => self.focus = (self.focus + count - 1) % count,
            Action::Submit => {}
            action => {
                self.error = None;
                let focus = self.focus;
                self.fields[focus].apply(action);
            }
        }
    }
}

#[cfg(test)]
#[path = "connexportform_tests.rs"]
mod connexportform_tests;
