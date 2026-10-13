//! Unit tests for `connexportform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::components::form_tests::{lines, render, type_str};

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
