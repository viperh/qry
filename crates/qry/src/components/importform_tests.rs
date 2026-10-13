//! Unit tests for `importform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;

use super::*;
use crate::components::form_tests::{key, lines, press, render, type_str};

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
