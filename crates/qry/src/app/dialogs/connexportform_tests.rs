//! Unit tests for `connexportform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    app::dialogs::form_tests::{render, type_str},
    testing::buffer_lines,
};

#[test]
fn the_form_sends_the_file() {
    let mut form = ConnExportForm::default();
    assert_eq!(form.submit().unwrap_err(), "File is required");
    type_str(&mut form, " ~/backup.json ");
    assert_eq!(
        form.submit().unwrap(),
        Submission::ExportConnections("~/backup.json".into())
    );

    let shown = buffer_lines(&render(&form, 80, 20)).join("\n");
    assert!(shown.contains("Export Connections"), "{shown}");
    assert!(shown.contains("no passwords are written"), "{shown}");
}
