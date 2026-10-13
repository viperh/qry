use crate::action::Action;
use crate::components::form::{Field, Form};

const LABELS: [&str; 1] = ["File"];
const FILE: usize = 0;

/// The Export Connections form: where to write a backup that Import
/// Connections can read back.
pub struct ConnExportForm {
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

    fn submit(&self) -> Result<Action, String> {
        let path = self.text(FILE).trim().to_string();
        if path.is_empty() {
            return Err("File is required".into());
        }
        Ok(Action::ExportConnections(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::form::tests::{lines, render, type_str};

    #[test]
    fn the_form_sends_the_file() {
        let mut form = ConnExportForm::default();
        assert_eq!(form.submit().unwrap_err(), "File is required");
        type_str(&mut form, " ~/backup.json ");
        assert_eq!(
            form.submit().unwrap(),
            Action::ExportConnections("~/backup.json".into())
        );

        let shown = lines(&render(&form, 80, 20)).join("\n");
        assert!(shown.contains("Export Connections"), "{shown}");
        assert!(shown.contains("no passwords are written"), "{shown}");
    }
}
