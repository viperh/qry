use super::{
    Submission,
    form::{Field, Form},
};

const LABELS: [&str; 1] = ["File"];
const FILE: usize = 0;

/// The Export Connections form: where to write a backup that Import
/// Connections can read back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConnExportForm {
    fields: [Field; 1],
    error: Option<String>,
    /// Set while the file is being written.
    pending: Option<&'static str>,
}

impl Default for ConnExportForm {
    fn default() -> Self {
        Self {
            fields: [Field::text_with_placeholder("~/qry-connections.json")],
            error: None,
            pending: None,
        }
    }
}

impl Form for ConnExportForm {
    fn title(&self) -> &'static str {
        "Export Connections"
    }

    fn hint(&self) -> &'static str {
        "Enter export · Esc cancel"
    }

    fn subtitle(&self) -> Option<String> {
        Some("no passwords are written".into())
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
        FILE
    }

    fn set_focus(&mut self, _focus: usize) {
        // One field: there is nowhere else to go.
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
        Ok(Submission::ExportConnections(path))
    }
}

#[cfg(test)]
#[path = "connexportform_tests.rs"]
mod connexportform_tests;
