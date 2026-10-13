//! Unit tests for `passwordform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    app::dialogs::form_tests::{render, type_str},
    testing::buffer_lines,
};

#[test]
fn what_is_typed_is_kept_as_a_password_and_never_printed() {
    let mut form = PasswordForm::new(SessionId(2), "c7f3a1e2", "prod");
    type_str(&mut form, " hunter2 ");

    assert!(!format!("{form:?}").contains("hunter2"));
    let submitted = form.submit().unwrap();
    assert!(!format!("{submitted:?}").contains("hunter2"));
    assert_eq!(
        submitted,
        Submission::PasswordEntered {
            session: SessionId(2),
            connection: "c7f3a1e2".into(),
            password: Password::new(" hunter2 ")
        }
    );
}

#[test]
fn the_popup_names_the_connection_and_masks_what_is_typed() {
    let mut form = PasswordForm::new(SessionId(2), "c7f3a1e2", "prod");
    type_str(&mut form, "hunter2");
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");

    assert!(shown.contains("Password"), "{shown}");
    assert!(shown.contains("for prod"), "{shown}");
    assert!(shown.contains("•••••••"), "{shown}");
    assert!(!shown.contains("hunter2"), "{shown}");
}
