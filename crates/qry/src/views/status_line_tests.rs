//! Unit tests for `status_line.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ratatui::buffer::Buffer;

use super::*;
use crate::{
    app::{DbAnswer, UiEvent},
    config::Config,
    connections::{Driver, StoredConnection},
    testing::{AppHarness, buffer_lines, grid, render_with},
};

/// A session on `prod`, a Postgres database `app`, opened and connected.
fn prod() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    let mut record = StoredConnection::with_id("prod", "prod", Driver::Postgres);
    record.host = Some("db.local".into());
    record.user = Some("app".into());
    record.database = Some("app".into());
    h.state_mut()
        .tree
        .connections
        .push(crate::app::state::Node::new(record, false));
    h.state_mut().tree.selected = 1;
    h.keys("enter");
    h.db(DbAnswer::Connected("app@db.local:5432/app".into()));
    h
}

fn drawn(h: &AppHarness, width: u16) -> Buffer {
    render_with(width, 1, |frame| {
        let state = h.state();
        draw(state, &state.theme, frame, frame.area());
    })
}

fn line(h: &AppHarness, width: u16) -> String {
    buffer_lines(&drawn(h, width))[0].trim_end().to_string()
}

#[test]
fn items_drop_in_priority_order() {
    let mut h = prod();
    h.db(DbAnswer::QueryDone(Arc::new(grid(1204, 9))));
    let position = "R 1 / 1\u{2009}204  C 1/9";
    let cases = [
        (120, " prod · postgres · app · auto"),
        (60, " prod · postgres · app · auto"),
        (50, " prod · postgres · app · auto"),
        (45, " prod · postgres · app"),
        (40, " prod · postgres"),
        (30, " prod"),
    ];
    for (width, left) in cases {
        let text = line(&h, width);
        assert!(text.starts_with(left), "{width}: {text:?}");
        assert!(!text[left.len()..].starts_with(" ·"), "{width}: {text:?}");
        assert!(text.ends_with(position), "{width}: {text:?}");
        assert!(text.chars().count() <= usize::from(width), "it never wraps");
    }
}

#[test]
fn position_shown_only_with_a_result() {
    let mut h = prod();
    assert!(!line(&h, 80).contains("R "), "no result yet");
    h.db(DbAnswer::QueryDone(Arc::new(grid(3, 2))));
    h.state_mut().focus = crate::app::state::Focus::Results;
    h.keys("j l");
    assert!(line(&h, 80).ends_with("R 2 / 3  C 2/2"), "{}", line(&h, 80));
}

#[test]
fn in_transaction_is_an_amber_pill() {
    let mut h = prod();
    h.keys("tab").type_str("begin").keys("f8");
    h.db(DbAnswer::QueryDone(Arc::new(grid(0, 0))));
    let text = line(&h, 80);
    assert!(text.ends_with(" TX"), "{text}");
    let buffer = drawn(&h, 80);
    // The line ends in the pill, its last space trimmed: `T` is next to last.
    let x = u16::try_from(text.chars().count() - 2).unwrap();
    let pill = h.theme().mode_pill(crate::theme::Pill::Edit);
    assert_eq!(Some(buffer[(x, 0)].bg), pill.bg);
    assert_eq!(pill.bg, Some(h.theme().amber));

    // A failed COMMIT leaves it open; one that works ends it.
    h.keys("ctrl-a backspace").type_str("commit").keys("f5");
    h.db(DbAnswer::QueryFailed {
        message: "nope".into(),
        position: None,
    });
    assert!(line(&h, 80).ends_with(" TX"));
    h.keys("f5");
    h.db(DbAnswer::QueryDone(Arc::new(grid(0, 0))));
    assert!(line(&h, 80).ends_with("auto"), "{}", line(&h, 80));
}

#[test]
fn thousands_are_grouped_by_thin_spaces() {
    assert_eq!(thousands(7), "7");
    assert_eq!(thousands(1204), "1\u{2009}204");
    assert_eq!(thousands(1_000_000), "1\u{2009}000\u{2009}000");
}
