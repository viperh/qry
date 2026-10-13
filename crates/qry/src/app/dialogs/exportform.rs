use qry_core::{ExportConfig, ExportType};

use super::{
    Submission,
    form::{Field, Form},
};
use crate::{action::Action, theme::Accent};

/// The fields every export has.
const LABELS_PLAIN: [&str; 2] = ["Path", "Type"];
/// Plus the separator, for the types that use one.
const LABELS_SEPARATOR: [&str; 3] = ["Path", "Type", "Separator"];
/// Plus the field that only OTHER shows.
const LABELS_CUSTOM: [&str; 4] = ["Path", "Type", "Separator", "Custom"];
const PATH: usize = 0;
const TYPE: usize = 1;
const SEPARATOR: usize = 2;
const CUSTOM: usize = 3;

/// Same order as `ExportType::ALL`.
const EXPORT_TYPES: [&str; 4] = ["CSV", "Excel", "Text", "JSON"];

const SEPARATORS: [&str; 5] = ["COMMA", "PIPE", "TAB", "SEMICOLON", "OTHER"];
/// What each of those writes, in the same order. OTHER has no entry: its
/// value comes from the Custom field, so it is the index past the end.
const SEPARATOR_VALUES: [&str; 4] = [",", "|", "\t", ";"];
const OTHER: usize = SEPARATOR_VALUES.len();

/// The Export form. Its fields depend on the type: only CSV and Text have a
/// separator, and only OTHER has the Custom field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExportForm {
    fields: Vec<Field>,
    focus: usize,
    error: Option<String>,
    /// Set while the database worker is writing the file.
    pending: Option<&'static str>,
    /// Kept so switching to Excel and back does not forget the choice.
    separator: usize,
}

impl Default for ExportForm {
    fn default() -> Self {
        Self {
            fields: vec![
                Field::text(),
                Field::choice(&EXPORT_TYPES, 0),
                Field::choice(&SEPARATORS, 0),
            ],
            focus: 0,
            error: None,
            pending: None,
            separator: 0,
        }
    }
}

impl ExportForm {
    fn etype(&self) -> ExportType {
        ExportType::ALL[self.selected(TYPE)]
    }
}

impl Form for ExportForm {
    fn title(&self) -> &'static str {
        "Export"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "export"),
            (Action::Cancel, "cancel"),
            (Action::NextField, "next"),
        ]
    }

    fn width(&self) -> u16 {
        64
    }

    fn accent(&self) -> Accent {
        Accent::Act
    }

    fn labels(&self) -> &'static [&'static str] {
        match self.fields.len() {
            len if len <= LABELS_PLAIN.len() => &LABELS_PLAIN,
            len if len <= LABELS_SEPARATOR.len() => &LABELS_SEPARATOR,
            _ => &LABELS_CUSTOM,
        }
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
        self.focus = focus;
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

    /// Adds and removes the Separator and Custom fields to match the type and
    /// the chosen separator.
    fn after_change(&mut self) {
        if self.fields.len() > SEPARATOR {
            self.separator = self.selected(SEPARATOR);
        }
        let wanted = match (self.etype().uses_separator(), self.separator == OTHER) {
            (false, _) => LABELS_PLAIN.len(),
            (true, false) => LABELS_SEPARATOR.len(),
            (true, true) => LABELS_CUSTOM.len(),
        };

        self.fields.truncate(wanted);
        while self.fields.len() < wanted {
            let next = self.fields.len();
            self.fields.push(if next == SEPARATOR {
                Field::choice(&SEPARATORS, self.separator)
            } else {
                Field::text()
            });
        }
        self.focus = self.focus.min(self.fields.len() - 1);
    }

    fn submit(&self) -> Result<Submission, String> {
        let path = self.text(PATH).trim().to_string();
        if path.is_empty() {
            return Err("Path is required".into());
        }

        let etype = self.etype();
        // Excel and JSON have no Separator field to read.
        let separator = if !etype.uses_separator() {
            String::new()
        } else {
            match self.selected(SEPARATOR) {
                chosen if chosen < SEPARATOR_VALUES.len() => SEPARATOR_VALUES[chosen].to_string(),
                // Not trimmed: a space or tab is a separator someone may want.
                _ => match self.text(CUSTOM) {
                    "" => return Err("Custom separator is required".into()),
                    custom => custom.to_string(),
                },
            }
        };

        // `new` adds the type's extension if the path is missing it.
        Ok(Submission::Export(ExportConfig::new(
            path, separator, etype,
        )))
    }
}

#[cfg(test)]
#[path = "exportform_tests.rs"]
mod exportform_tests;
