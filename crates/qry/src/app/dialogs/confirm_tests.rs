//! Unit tests for `confirm.rs`. The keys, `y`, `n` and Esc, are tested
//! through the reducer in `app/overlay_tests.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    config::Config,
    testing::{buffer_lines, render_with},
    theme::Theme,
    views::confirm::{draw, wrap},
};

fn shown(confirm: &Confirm, width: u16, height: u16) -> String {
    let buffer = render_with(width, height, |frame| {
        draw(
            confirm,
            &Config::embedded().keymap,
            &Theme::DARK,
            frame,
            frame.area(),
        );
    });
    buffer_lines(&buffer).join("\n")
}

#[test]
fn destructive_confirm_defaults_to_cancel() {
    let confirm = Confirm::close_session(SessionId(2), "prod");
    assert!(confirm.destructive());
    assert_eq!(confirm.accent(), Accent::Danger);
    assert!(!confirm.confirm_selected());
    assert_eq!(confirm.submit().unwrap(), Submission::Dismissed);
    assert!(shown(&confirm, 80, 24).contains("[ Cancel ]"));
}

#[test]
fn moving_between_buttons_picks_what_enter_does() {
    let mut confirm = Confirm::close_session(SessionId(2), "prod");
    confirm.act(Action::Right);
    assert!(confirm.confirm_selected());
    assert_eq!(
        confirm.submit().unwrap(),
        Submission::Confirmed(OnConfirm::CloseSession(SessionId(2)))
    );
    assert!(shown(&confirm, 80, 24).contains("[ Close ]"));
    confirm.act(Action::NextField);
    assert!(!confirm.confirm_selected());
}

#[test]
fn message_wraps_within_width() {
    let confirm = Confirm::close_session(SessionId(2), "prod");
    for line in wrap(confirm.message(), 30) {
        assert!(line.chars().count() <= 30, "{line}");
    }
    assert_eq!(
        wrap("one two three", 7),
        ["one two", "three"],
        "words are never split"
    );
    let text = shown(&confirm, 80, 24);
    assert!(text.contains("prod is running a query."), "{text}");
    assert!(text.contains("cancels it."), "{text}");
}

#[test]
fn checkboxes_move_with_tab_and_tick_with_space() {
    let mut confirm = Confirm::new("T", "m", "Go", false, OnConfirm::CloseSession(SessionId(1)))
        .with_check(Check::ForgetSecret, "one", true)
        .with_check(Check::DeleteHistory, "two", true);
    confirm.act(Action::NextChoice);
    assert!(!confirm.is_checked(Check::ForgetSecret));
    confirm.act(Action::NextField);
    confirm.act(Action::NextChoice);
    assert!(!confirm.is_checked(Check::DeleteHistory));
    assert!(
        confirm.confirm_selected(),
        "Space no longer moves the buttons"
    );
    let text = shown(&confirm, 80, 24);
    assert!(
        text.contains("[ ] one") && text.contains("[ ] two"),
        "{text}"
    );
    assert!(
        text.contains("Enter confirm · Space toggle · Esc cancel"),
        "{text}"
    );
}
