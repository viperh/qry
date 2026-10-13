//! Toasts through the reducer: lifetimes on the virtual clock, the queue,
//! Esc, and what produces them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{notice::Notice, testing::AppHarness, views::layout::compute_layout};

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.take_effects();
    h
}

fn visible(h: &AppHarness) -> Option<String> {
    h.state()
        .toasts
        .visible()
        .map(|(t, _)| t.notice.text.clone())
}

#[test]
fn info_expires_after_4s_error_after_8s() {
    let mut h = harness();
    h.send(UiEvent::Notice(Notice::info("hello")));
    h.advance(3999);
    assert_eq!(visible(&h).as_deref(), Some("hello"));
    h.advance(1);
    assert_eq!(visible(&h), None);

    h.send(UiEvent::Notice(Notice::error("boom")));
    h.advance(7999);
    assert!(h.render(120, 40).contains("✗ boom"));
    h.advance(1);
    assert!(!h.render(120, 40).contains("boom"));
}

#[test]
fn next_toast_appears_when_current_expires() {
    let mut h = harness();
    h.send(UiEvent::Notice(Notice::warning("first")));
    h.send(UiEvent::Notice(Notice::success("second")));
    assert!(h.render(120, 40).contains("(+1)"));
    h.advance(6000);
    assert_eq!(visible(&h).as_deref(), Some("second"));
    // Its own 4 s start when it shows.
    h.advance(3999);
    assert_eq!(visible(&h).as_deref(), Some("second"));
    h.advance(1);
    assert_eq!(visible(&h), None);
}

#[test]
fn esc_dismisses_toast_when_nothing_else_uses_it() {
    let mut h = harness();
    h.send(UiEvent::Notice(Notice::info("one")));
    h.send(UiEvent::Notice(Notice::info("two")));
    h.keys("esc");
    assert_eq!(visible(&h).as_deref(), Some("two"));
    // In the editor too, which leaves Esc unbound.
    h.keys("tab esc");
    assert_eq!(visible(&h), None);
}

#[test]
fn esc_in_dialog_closes_dialog_not_toast() {
    let mut h = harness();
    h.send(UiEvent::Notice(Notice::info("stay")));
    h.keys("ctrl-n esc");
    assert!(h.state().dialog().is_none());
    assert_eq!(visible(&h).as_deref(), Some("stay"));
}

#[test]
fn action_error_becomes_a_toast() {
    let mut h = harness();
    h.send(UiEvent::Notice(Notice::error(
        "cannot remove the stored password: denied",
    )));
    assert!(
        h.render(120, 40)
            .contains("✗ cannot remove the stored password: denied")
    );
    assert_eq!(h.state().messages.len(), 1);
}

#[test]
fn failed_save_connections_is_visible() {
    let mut h = harness();
    h.file(FileAnswer::SaveFailed(
        "cannot save connections: permission denied (os error 13)".into(),
    ));
    let text = h.render(120, 40);
    assert!(
        text.contains("✗ cannot save connections: permission denied"),
        "{text}"
    );
}

#[test]
fn toast_does_not_move_layout() {
    let mut h = harness();
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let before = compute_layout(&h.state().layout_input(area));
    let screen = h.render(120, 40);
    h.send(UiEvent::Notice(Notice::info("hello")));
    assert_eq!(compute_layout(&h.state().layout_input(area)), before);
    let with = h.render(120, 40);
    // Only the toast's row changed.
    let changed: Vec<_> = screen
        .lines()
        .zip(with.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(changed, [before.unwrap().toast.y as usize]);
}

#[test]
fn identical_notices_coalesce_on_screen() {
    let mut h = harness();
    for _ in 0..3 {
        h.send(UiEvent::Notice(Notice::error("again")));
    }
    let text = h.render(120, 40);
    assert!(text.contains("✗ again ×3"), "{text}");
    assert!(!text.contains("(+"), "{text}");
}

#[test]
fn toast_draws_above_dialog() {
    let mut h = harness();
    h.keys("ctrl-n");
    h.send(UiEvent::Notice(Notice::error(
        "cannot save connections: permission denied (os error 13)",
    )));
    let buffer = h.render_buffer(120, 40);
    let text = crate::testing::buffer_to_string(&buffer);
    assert!(text.contains("New Connection"), "{text}");
    assert!(text.contains("✗ cannot save connections"), "{text}");
    // Not dimmed by the backdrop.
    let y = text
        .lines()
        .position(|l| l.contains("✗ cannot save"))
        .unwrap();
    let x = text
        .lines()
        .nth(y)
        .unwrap()
        .chars()
        .position(|c| c == '✗')
        .unwrap();
    let cell = &buffer[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())];
    assert_eq!(cell.fg, h.theme().coral);
    insta::assert_snapshot!("screen_toast_over_dialog_120x40", text);
}
