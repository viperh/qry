use super::{
    Submission,
    form::{Field, Form},
};
use crate::action::Action;
use crate::import::Format;

const LABELS: [&str; 2] = ["File", "Format"];
const FILE: usize = 0;
const FORMAT: usize = 1;

/// Same order as `Format::ALL`.
const FORMATS: [&str; 4] = ["DETECT", "QRY BACKUP", "SQL DEVELOPER", "DBEAVER"];

/// The Import Connections form: a file, and what wrote it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImportForm {
    fields: [Field; 2],
    focus: usize,
    error: Option<String>,
    /// Set while the file is being read.
    pending: Option<&'static str>,
}

impl Default for ImportForm {
    fn default() -> Self {
        Self {
            fields: [
                Field::text_with_placeholder("~/data-sources.json"),
                Field::choice(&FORMATS, 0),
            ],
            focus: FILE,
            error: None,
            pending: None,
        }
    }
}

impl Form for ImportForm {
    fn title(&self) -> &'static str {
        "Import Connections"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "import"),
            (Action::Cancel, "cancel"),
            (Action::NextField, "next"),
        ]
    }

    fn width(&self) -> u16 {
        72
    }

    fn subtitle(&self) -> Option<String> {
        Some("passwords are asked on connect".into())
    }

    fn labels(&self) -> &'static [&'static str] {
        &LABELS
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

    fn submit(&self) -> Result<Submission, String> {
        let path = self.text(FILE).trim().to_string();
        if path.is_empty() {
            return Err("File is required".into());
        }
        Ok(Submission::Import {
            path,
            format: Format::ALL[self.selected(FORMAT)],
        })
    }
}

#[cfg(test)]
#[path = "importform_tests.rs"]
mod importform_tests;
