//! Unit tests for `connexportform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::Action;
use crate::{
    app::dialogs::form_tests::{render, type_str},
    testing::buffer_lines,
};

fn offered() -> Vec<(String, String)> {
    vec![("a".into(), "local".into()), ("b".into(), "prod".into())]
}

#[test]
fn the_form_sends_the_file_and_the_chosen_connections() {
    let mut form = ConnExportForm::new(&offered());
    assert_eq!(form.submit().unwrap_err(), "File is required");
    type_str(&mut form, " ~/backup.json ");
    assert_eq!(
        form.submit().unwrap(),
        Submission::ExportConnections {
            path: "~/backup.json".into(),
            ids: vec!["a".into(), "b".into()],
        }
    );

    // Space on a connection leaves it out.
    form.act(Action::Down);
    form.act(Action::NextChoice);
    assert_eq!(form.included(), ["b"]);
    form.act(Action::Down);
    form.act(Action::NextChoice);
    assert_eq!(
        form.submit().unwrap_err(),
        "Choose at least one connection to export"
    );

    let shown = buffer_lines(&render(&form, 80, 20)).join("\n");
    assert!(shown.contains("Export Connections"), "{shown}");
    assert!(shown.contains("passwords are not included"), "{shown}");
    assert!(shown.contains("[ ] local"), "{shown}");
}

#[test]
fn export_connections_dialog_100x30() {
    let mut form = ConnExportForm::new(&offered());
    type_str(&mut form, "~/qry-backup.json");
    let buffer = render(&form, 100, 30);
    insta::assert_snapshot!(
        "export_connections_dialog_100x30",
        crate::testing::buffer_to_string(&buffer)
    );
    insta::assert_snapshot!(
        "export_connections_dialog_100x30_styled",
        crate::testing::styled(&buffer)
    );
}
