use qry_core::{ExportConfig, ExportType};

use crate::action::Action;
use crate::components::form::{Field, Form};

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
pub struct ExportForm {
    fields: Vec<Field>,
    focus: usize,
    error: Option<String>,
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

    fn hint(&self) -> &'static str {
        "Enter export · Esc cancel"
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

    fn submit(&self) -> Result<Action, String> {
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
        Ok(Action::Export(ExportConfig::new(path, separator, etype)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::form::tests::{key, lines, press, render, type_str};
    use crossterm::event::KeyCode;

    /// Moves the focus to `field` from the start of the form.
    fn focus(form: &mut ExportForm, field: usize) {
        form.set_focus(0);
        for _ in 0..field {
            press(form, key(KeyCode::Tab));
        }
    }

    /// Cycles `field` with the Right key until `option` is chosen.
    fn pick(form: &mut ExportForm, field: usize, options: &[&str], option: &str) {
        focus(form, field);
        let target = options.iter().position(|o| *o == option).unwrap();
        while form.selected(field) != target {
            press(form, key(KeyCode::Right));
        }
    }

    #[test]
    fn export_type_labels_line_up_with_core() {
        let core: Vec<String> = ExportType::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(core, EXPORT_TYPES);
    }

    #[test]
    fn a_path_is_required() {
        let mut form = ExportForm::default();
        assert_eq!(form.submit().unwrap_err(), "Path is required");
        type_str(&mut form, "   ");
        assert_eq!(form.submit().unwrap_err(), "Path is required");
    }

    #[test]
    fn each_separator_choice_has_its_character() {
        for (option, expected) in [("COMMA", ","), ("PIPE", "|"), ("TAB", "\t"), ("SEMICOLON", ";")] {
            let mut form = ExportForm::default();
            type_str(&mut form, "rows.csv");
            pick(&mut form, SEPARATOR, &SEPARATORS, option);
            let Ok(Action::Export(config)) = form.submit() else {
                panic!("expected an export");
            };
            assert_eq!(config.separator, expected, "{option}");
            assert_eq!(form.fields.len(), LABELS_SEPARATOR.len(), "{option} added a field");
        }
    }

    #[test]
    fn other_reveals_a_custom_field_and_hiding_it_removes_the_value() {
        let mut form = ExportForm::default();
        type_str(&mut form, "rows.txt");
        assert_eq!(form.labels(), LABELS_SEPARATOR);

        pick(&mut form, SEPARATOR, &SEPARATORS, "OTHER");
        assert_eq!(form.labels(), LABELS_CUSTOM);
        assert_eq!(form.fields.len(), 4);

        // An empty custom separator is refused.
        assert_eq!(form.submit().unwrap_err(), "Custom separator is required");

        focus(&mut form, CUSTOM);
        type_str(&mut form, "::");
        let Ok(Action::Export(config)) = form.submit() else {
            panic!("expected an export");
        };
        assert_eq!(config.separator, "::");

        // Choosing another separator takes the field away again.
        pick(&mut form, SEPARATOR, &SEPARATORS, "PIPE");
        assert_eq!(form.labels(), LABELS_SEPARATOR);
        assert_eq!(form.fields.len(), 3);
        assert_eq!(form.focus(), SEPARATOR);
    }

    #[test]
    fn the_form_builds_its_export_config() {
        let mut form = ExportForm::default();
        type_str(&mut form, " rows.txt ");
        pick(&mut form, TYPE, &EXPORT_TYPES, "Text");
        pick(&mut form, SEPARATOR, &SEPARATORS, "TAB");
        assert_eq!(
            form.submit().unwrap(),
            Action::Export(ExportConfig {
                path: "rows.txt".into(),
                separator: "\t".into(),
                etype: ExportType::Text,
            })
        );
    }

    #[test]
    fn a_path_without_the_right_extension_gets_one() {
        let expected = [("CSV", "rows.csv"), ("Excel", "rows.xlsx"), ("Text", "rows.txt"), ("JSON", "rows.json")];
        for (option, path) in expected {
            let mut form = ExportForm::default();
            type_str(&mut form, "rows");
            pick(&mut form, TYPE, &EXPORT_TYPES, option);
            let Ok(Action::Export(config)) = form.submit() else {
                panic!("expected an export");
            };
            assert_eq!(config.path, path, "{option}");
        }

        // One that already has it is left alone.
        let mut form = ExportForm::default();
        type_str(&mut form, "rows.csv");
        let Ok(Action::Export(config)) = form.submit() else {
            panic!("expected an export");
        };
        assert_eq!(config.path, "rows.csv");
    }

    #[test]
    fn excel_and_json_have_no_separator_field() {
        for option in ["Excel", "JSON"] {
            let mut form = ExportForm::default();
            type_str(&mut form, "rows.out");
            pick(&mut form, TYPE, &EXPORT_TYPES, option);

            assert_eq!(form.labels(), LABELS_PLAIN, "{option}");
            assert_eq!(form.fields.len(), 2, "{option}");
            let shown = lines(&render(&form, 80, 24)).join("\n");
            assert!(!shown.contains("Separator"), "{option}: {shown}");

            let Ok(Action::Export(config)) = form.submit() else {
                panic!("expected an export");
            };
            assert_eq!(config.separator, "", "{option}");
        }
    }

    #[test]
    fn the_separator_survives_a_trip_through_excel() {
        let mut form = ExportForm::default();
        type_str(&mut form, "rows.txt");
        pick(&mut form, SEPARATOR, &SEPARATORS, "SEMICOLON");
        pick(&mut form, TYPE, &EXPORT_TYPES, "Excel");
        pick(&mut form, TYPE, &EXPORT_TYPES, "CSV");

        assert_eq!(SEPARATORS[form.selected(SEPARATOR)], "SEMICOLON");
        let Ok(Action::Export(config)) = form.submit() else {
            panic!("expected an export");
        };
        assert_eq!(config.separator, ";");
    }

    #[test]
    fn a_windows_path_can_be_typed() {
        let mut form = ExportForm::default();
        type_str(&mut form, r"C:\data\rows.csv");
        assert_eq!(form.text(PATH), r"C:\data\rows.csv");
    }

    #[test]
    fn the_popup_shows_every_field() {
        let mut form = ExportForm::default();
        let shown = lines(&render(&form, 80, 24)).join("\n");
        assert!(shown.contains("› Path") && shown.contains("Type"), "{shown}");
        assert!(shown.contains("Separator") && shown.contains("COMMA"), "{shown}");
        assert!(!shown.contains("Custom"), "{shown}");

        pick(&mut form, SEPARATOR, &SEPARATORS, "OTHER");
        let shown = lines(&render(&form, 80, 24)).join("\n");
        assert!(shown.contains("OTHER") && shown.contains("Custom"), "{shown}");
    }
}
