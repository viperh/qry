//! Unit tests for `passwordform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    components::form_tests::{render, type_str},
    testing::buffer_lines,
};

#[test]
fn what_is_typed_goes_to_the_stash_and_not_to_the_action() {
    let _guard = secrets::test_lock();
    let _ = secrets::take_stash();
    let mut form = PasswordForm::new("prod");
    type_str(&mut form, " hunter2 ");

    assert_eq!(form.submit().unwrap(), Action::PasswordEntered);
    assert_eq!(
        secrets::take_stash().map(|p| p.as_str().to_string()),
        Some(" hunter2 ".to_string())
    );
}

#[test]
fn the_popup_names_the_connection_and_masks_what_is_typed() {
    let mut form = PasswordForm::new("prod");
    type_str(&mut form, "hunter2");
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");

    assert!(shown.contains("Password"), "{shown}");
    assert!(shown.contains("for prod"), "{shown}");
    assert!(shown.contains("•••••••"), "{shown}");
    assert!(!shown.contains("hunter2"), "{shown}");
}
