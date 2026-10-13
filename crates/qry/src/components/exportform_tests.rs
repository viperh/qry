//! Unit tests for `exportform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;

use super::*;
use crate::components::form_tests::{key, lines, press, render, type_str};

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
    for (option, expected) in [
        ("COMMA", ","),
        ("PIPE", "|"),
        ("TAB", "\t"),
        ("SEMICOLON", ";"),
    ] {
        let mut form = ExportForm::default();
        type_str(&mut form, "rows.csv");
        pick(&mut form, SEPARATOR, &SEPARATORS, option);
        let Ok(Action::Export(config)) = form.submit() else {
            panic!("expected an export");
        };
        assert_eq!(config.separator, expected, "{option}");
        assert_eq!(
            form.fields.len(),
            LABELS_SEPARATOR.len(),
            "{option} added a field"
        );
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
    let expected = [
        ("CSV", "rows.csv"),
        ("Excel", "rows.xlsx"),
        ("Text", "rows.txt"),
        ("JSON", "rows.json"),
    ];
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
    assert!(
        shown.contains("› Path") && shown.contains("Type"),
        "{shown}"
    );
    assert!(
        shown.contains("Separator") && shown.contains("COMMA"),
        "{shown}"
    );
    assert!(!shown.contains("Custom"), "{shown}");

    pick(&mut form, SEPARATOR, &SEPARATORS, "OTHER");
    let shown = lines(&render(&form, 80, 24)).join("\n");
    assert!(
        shown.contains("OTHER") && shown.contains("Custom"),
        "{shown}"
    );
}
