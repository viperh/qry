//! Unit tests for `confirm.rs`. The keys, `y`, `n` and Esc, are tested
//! through the reducer in `app/overlay_tests.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    testing::{buffer_lines, render_with},
    theme::Theme,
    views::confirm::{draw, wrap},
};

fn shown(confirm: &Confirm, width: u16, height: u16) -> String {
    let buffer = render_with(width, height, |frame| {
        draw(confirm, &Theme::DARK, frame, frame.area());
    });
    buffer_lines(&buffer).join("\n")
}

#[test]
fn destructive_confirm_defaults_to_cancel() {
    let confirm = Confirm::delete_connection("c1", "prod", true);
    assert!(confirm.destructive());
    assert_eq!(confirm.accent(), Accent::Danger);
    assert!(!confirm.confirm_selected());
    assert_eq!(confirm.submit().unwrap(), Submission::Dismissed);
    assert!(shown(&confirm, 80, 24).contains("[ Cancel ]"));
}

#[test]
fn moving_between_buttons_picks_what_enter_does() {
    let mut confirm = Confirm::delete_connection("c1", "prod", false);
    confirm.act(Action::Right);
    assert!(confirm.confirm_selected());
    assert_eq!(
        confirm.submit().unwrap(),
        Submission::Confirmed(OnConfirm::DeleteConnection("c1".into()))
    );
    assert!(shown(&confirm, 80, 24).contains("[ Delete ]"));
    confirm.act(Action::NextField);
    assert!(!confirm.confirm_selected());
}

#[test]
fn message_wraps_within_width() {
    let confirm = Confirm::delete_connection("c1", "prod", true);
    for line in wrap(confirm.message(), 30) {
        assert!(line.chars().count() <= 30, "{line}");
    }
    assert_eq!(
        wrap("one two three", 7),
        ["one two", "three"],
        "words are never split"
    );
    let text = shown(&confirm, 80, 24);
    assert!(
        text.contains("Delete \"prod\" and forget its stored"),
        "{text}"
    );
    assert!(text.contains("This cannot be undone."), "{text}");
}
