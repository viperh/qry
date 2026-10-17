//! Unit tests for `messages.rs`. The dialog's keys are tested through the
//! reducer in `app/toast_tests.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    app::{Effect, UiEvent},
    config::Config,
    notice::{Notice, TimeOfDay},
    testing::{AppHarness, styled},
};

/// The app with three notices logged a little apart, and the Messages
/// dialog open.
fn with_messages() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    for (at, notice) in [
        (
            12 * 3600 + 3 * 60 + 40,
            Notice::success("Exported 3 connections to /home/u/qry-backup.json"),
        ),
        (
            12 * 3600 + 3 * 60 + 58,
            Notice::success("Connected to app@db.local:5432/app"),
        ),
        (
            12 * 3600 + 4 * 60 + 31,
            Notice::error(
                "cannot save connections: permission denied (os error 13) while writing /home/u/.local/share/qry/connections.json",
            ),
        ),
    ] {
        h.send(UiEvent::Clock(TimeOfDay(at)));
        h.send(UiEvent::Notice(notice));
    }
    h.keys("ctrl-l");
    h
}

#[test]
fn newest_first_and_wrapped() {
    let mut h = with_messages();
    let text = h.render(80, 24);
    let error = text.find("12:04:31  ✗ cannot save").unwrap();
    let connected = text.find("12:03:58  ✓ Connected").unwrap();
    let exported = text.find("12:03:40  ✓ Exported").unwrap();
    assert!(error < connected && connected < exported, "{text}");
    // Wrapped, not cut: the end of the long one is there.
    assert!(text.contains("connections.json"), "{text}");
}

#[test]
fn y_copies_selected_message() {
    let mut h = with_messages();
    h.take_effects();
    h.keys("j y");
    assert_eq!(
        h.take_effects(),
        [Effect::CopyToClipboard(
            "Connected to app@db.local:5432/app".into()
        )]
    );
}

#[test]
fn c_clears_history() {
    let mut h = with_messages();
    h.keys("c");
    assert!(h.state().messages.is_empty());
    assert!(h.render(80, 24).contains("No messages yet"));
    h.keys("esc");
    assert!(h.state().overlays.is_empty());
}

#[test]
fn messages_snapshot() {
    let mut h = with_messages();
    // Let the toasts run out, so the dialog shows on its own.
    h.advance(30_000);
    insta::assert_snapshot!("messages_80x24", h.render(80, 24));
    insta::assert_snapshot!("messages_80x24_styled", styled(&h.render_buffer(80, 24)));
}
