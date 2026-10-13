//! Unit tests for `confirmform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{app::dialogs::form_tests::render, testing::buffer_lines};

#[test]
fn submitting_closes_the_session_it_was_opened_for() {
    let form = ConfirmForm::close_session(SessionId(4), "prod");
    assert_eq!(
        form.submit().unwrap(),
        Submission::CloseSession(SessionId(4))
    );
}

#[test]
fn the_popup_names_the_session_and_says_what_closing_does() {
    let form = ConfirmForm::close_session(SessionId(2), "prod");
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("Close Session"), "{shown}");
    assert!(shown.contains("prod is running a query"), "{shown}");
    assert!(shown.contains("cancelling the query"), "{shown}");
}
