use crate::action::Action;
use crate::components::form::{Field, Form};
use crate::import::Format;

const LABELS: [&str; 2] = ["File", "Format"];
const FILE: usize = 0;
const FORMAT: usize = 1;

/// Same order as `Format::ALL`.
const FORMATS: [&str; 4] = ["DETECT", "QRY BACKUP", "SQL DEVELOPER", "DBEAVER"];

/// The Import Connections form: a file, and what wrote it.
pub struct ImportForm {
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

    fn hint(&self) -> &'static str {
        "Enter import · Esc cancel"
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

    fn submit(&self) -> Result<Action, String> {
        let path = self.text(FILE).trim().to_string();
        if path.is_empty() {
            return Err("File is required".into());
        }
        Ok(Action::Import {
            path,
            format: Format::ALL[self.selected(FORMAT)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::form::tests::{key, lines, press, render, type_str};
    use crossterm::event::KeyCode;

    #[test]
    fn format_labels_line_up_with_the_formats() {
        assert_eq!(FORMATS.len(), Format::ALL.len());
        for (label, format) in FORMATS.iter().zip(Format::ALL) {
            assert_eq!(label.to_lowercase(), format.to_string().to_lowercase());
        }
    }

    #[test]
    fn the_form_sends_the_file_and_the_chosen_format() {
        let mut form = ImportForm::default();
        assert_eq!(form.submit().unwrap_err(), "File is required");

        type_str(&mut form, " ~/sqldev.json ");
        press(&mut form, key(KeyCode::Tab));
        press(&mut form, key(KeyCode::Right));
        press(&mut form, key(KeyCode::Right));
        assert_eq!(
            form.submit().unwrap(),
            Action::Import {
                path: "~/sqldev.json".into(),
                format: Format::SqlDeveloper
            }
        );
    }

    #[test]
    fn the_popup_says_passwords_are_not_imported() {
        let shown = lines(&render(&ImportForm::default(), 80, 20)).join("\n");
        assert!(shown.contains("Import Connections"), "{shown}");
        assert!(shown.contains("passwords are asked on connect"), "{shown}");
        assert!(shown.contains("DETECT"), "{shown}");
    }
}
