//! Unit tests for `inspector.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use qry_core::{QueryResult, testing::typed_result};
use ratatui::buffer::Buffer;

use super::*;
use crate::{
    app::{DbAnswer, UiEvent, state::Overlay},
    config::Config,
    testing::{AppHarness, styled},
    views::layout::compute_layout,
};

/// Six users: repeated e-mails, a NULL, a multi-line note.
pub(crate) fn users() -> QueryResult {
    typed_result(
        &[
            ("id", "int8"),
            ("name", "text"),
            ("email", "text"),
            ("created_at", "timestamp"),
            ("notes", "text"),
        ],
        &[
            &[
                Some("1"),
                Some("alice"),
                Some("alice@example.com"),
                Some("2026-10-01 09:00:00"),
                None,
            ],
            &[
                Some("2"),
                Some("bob"),
                Some("bob@example.com"),
                Some("2026-10-02 10:15:00"),
                Some("ok"),
            ],
            &[
                Some("3"),
                Some("carol"),
                Some("alice@example.com"),
                Some("2026-10-03 11:30:00"),
                Some("first line\nsecond line that goes on and on past the panel"),
            ],
            &[
                Some("4"),
                Some("dave"),
                None,
                Some("2026-10-04 12:45:00"),
                None,
            ],
            &[
                Some("5"),
                Some("erin"),
                Some("bob@example.com"),
                Some("2026-10-05 14:00:00"),
                None,
            ],
            &[
                Some("6"),
                Some("frank"),
                Some("alice@example.com"),
                Some("2026-10-06 15:30:00"),
                None,
            ],
        ],
    )
}

/// A session that ran `select * from users` and got `result`, the grid
/// focused, at `w`x`h`.
pub(crate) fn ran(result: QueryResult, w: u16, h: u16) -> AppHarness {
    let mut app = AppHarness::new(Config::embedded());
    app.send(UiEvent::Started);
    app.db(DbAnswer::Connected(":memory:".into()));
    app.keys("tab").type_str("select * from users");
    app.keys("f8");
    app.db(DbAnswer::QueryDone(Arc::new(result)));
    app.state_mut().focus = Focus::Results;
    app.resize(w, h);
    // The run's toasts go first.
    app.advance(60_000);
    app.take_effects();
    app
}

/// [`ran`] with the inspector toggled on and the cursor on `email`.
fn inspecting(w: u16, h: u16) -> AppHarness {
    let mut app = ran(users(), w, h);
    app.keys("j j l l i");
    app
}

fn snapshot(name: &str, app: &mut AppHarness, w: u16, h: u16) {
    insta::assert_snapshot!(format!("{name}_{w}x{h}"), app.render(w, h));
    insta::assert_snapshot!(
        format!("{name}_{w}x{h}_styled"),
        styled(&app.render_buffer(w, h))
    );
}

/// Where `text` starts on screen.
fn find(buf: &Buffer, text: &str) -> (u16, u16) {
    let area = buf.area;
    for y in 0..area.height {
        let line: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
        if let Some(at) = line.find(text) {
            let x = line[..at].chars().count();
            return (u16::try_from(x).unwrap(), y);
        }
    }
    panic!("{text:?} not on screen");
}

#[test]
fn inspector_docked_160x40() {
    let mut app = inspecting(160, 40);
    let text = app.render(160, 40);
    for wanted in [
        "COLUMN",
        "email · text · col 3/5",
        "nulls 1 / 6 · distinct 2",
        "TOP VALUES · 6 fetched rows",
        "RECORD · row 3 / 6",
        "notes       first line↵second",
    ] {
        assert!(text.contains(wanted), "{wanted:?} in\n{text}");
    }
    snapshot("inspector_docked", &mut app, 160, 40);
}

#[test]
fn inspector_overlay_80x24() {
    let mut app = inspecting(80, 24);
    let text = app.render(80, 24);
    assert!(text.contains("RECORD · row 3 / 6"), "{text}");
    snapshot("inspector_overlay", &mut app, 80, 24);
}

#[test]
fn inspector_focused_160x40() {
    let mut app = inspecting(160, 40);
    app.keys("tab");
    assert_eq!(app.state().focus, Focus::Inspector);
    snapshot("inspector_focused", &mut app, 160, 40);
}

#[test]
fn inspector_profiling_spinner_160x24() {
    let mut app = ran(qry_core::testing::grid(50_000, 3), 160, 24);
    app.keys("i");
    let text = app.render(160, 24);
    assert!(text.contains("counting… 20 000 / 50 000"), "{text}");
    snapshot("inspector_profiling_spinner", &mut app, 160, 24);
    // Two more ticks finish it.
    app.advance(500);
    let text = app.render(160, 24);
    assert!(
        text.contains("nulls 0 / 50 000 · distinct 50 000"),
        "{text}"
    );
    assert!(!text.contains("TOP VALUES"), "every value is distinct");
}

#[test]
fn docked_at_140_overlay_below() {
    for (w, docked) in [(140, true), (139, false), (80, false)] {
        let app = inspecting(w, 30);
        let area = Rect::new(0, 0, w, 30);
        let screen = compute_layout(&app.state().layout_input(area)).unwrap();
        let (divider, inspector) = placement(app.state(), &screen, area).unwrap();
        assert_eq!(screen.inspector.is_some(), docked, "{w}");
        if docked {
            assert_eq!(inspector.width, 40);
        } else {
            assert_eq!(divider.x, w / 2, "{w}");
            assert_eq!(inspector.right(), w);
            assert_eq!(inspector.height, screen.tree.height, "the body's height");
        }
    }
    let mut hidden = ran(users(), 160, 40);
    let area = Rect::new(0, 0, 160, 40);
    let screen = compute_layout(&hidden.state().layout_input(area)).unwrap();
    assert!(placement(hidden.state(), &screen, area).is_none());
    // The focus survives a resize across the docking width.
    hidden.keys("i tab");
    hidden.resize(100, 30);
    assert_eq!(hidden.state().focus, Focus::Inspector);
    assert!(hidden.render(100, 30).contains("RECORD"));
}

#[test]
fn focus_switches_label_style() {
    let mut app = inspecting(160, 40);
    let theme = *app.theme();
    let buf = app.render_buffer(160, 40);
    let at = find(&buf, "RECORD");
    assert_eq!(buf[at].style().fg, theme.inspector_label().fg);
    app.keys("tab");
    let buf = app.render_buffer(160, 40);
    assert_eq!(buf[at].style().fg, theme.inspector_label_focused().fg);
    assert!(buf[at].modifier.contains(Modifier::BOLD));
    app.keys("esc");
    assert_eq!(
        app.state().focus,
        Focus::Results,
        "Esc goes back to the grid"
    );
    app.keys("tab tab");
    assert_eq!(app.state().focus, Focus::Results, "so does Tab");
}

#[test]
fn record_follows_grid_cursor() {
    let mut app = inspecting(160, 40);
    app.keys("j");
    let text = app.render(160, 40);
    assert!(text.contains("RECORD · row 4 / 6"), "{text}");
    assert!(text.contains("name        dave"), "{text}");
    // In the inspector ↑↓ pick the field, which is the grid's column, and
    // ←→ the record.
    app.keys("tab j j h");
    let results = app.state().results();
    assert_eq!((results.row, results.col), (2, 4));
    let text = app.render(160, 40);
    assert!(text.contains("notes · text · col 5/5"), "{text}");
    assert!(text.contains("name        carol"), "{text}");
}

#[test]
fn enter_on_record_opens_popup() {
    let mut app = inspecting(160, 40);
    app.keys("tab j j enter");
    let Some(Overlay::Value(popup)) = app.state().overlays.last() else {
        panic!("no popup: {:?}", app.state().overlays);
    };
    assert_eq!(popup.title, "users.notes · text · row 3");
    assert_eq!(
        popup.value.as_deref(),
        Some("first line\nsecond line that goes on and on past the panel")
    );
    app.keys("esc");
    assert!(app.state().overlays.is_empty());
    assert_eq!(app.state().focus, Focus::Inspector);
}

#[test]
fn y_in_the_record_copies_the_whole_value() {
    let mut app = inspecting(160, 40);
    app.keys("tab j j y");
    assert!(
        app.take_effects()
            .contains(&crate::app::Effect::CopyToClipboard(
                "first line\nsecond line that goes on and on past the panel".into()
            ))
    );
    assert!(app.render(160, 40).contains("copied value"));
}

#[test]
fn new_result_resets_record_row() {
    let mut app = inspecting(160, 40);
    assert_eq!(app.state().results().row, 2);
    app.state_mut().focus = Focus::Editor;
    app.keys("f8");
    app.db(DbAnswer::QueryDone(Arc::new(users())));
    let results = app.state().results();
    assert_eq!((results.row, results.col), (0, 0));
    assert!(app.render(160, 40).contains("RECORD · row 1 / 6"));
}

#[test]
fn hiding_the_focused_inspector_gives_the_grid_the_focus() {
    let mut app = inspecting(160, 40);
    app.keys("tab i");
    assert!(!app.state().chrome.inspector);
    assert_eq!(app.state().focus, Focus::Results);
}

#[test]
fn bars_fill_by_eighths() {
    assert_eq!(bar(4, 4), "████████");
    assert_eq!(bar(2, 4), "████    ");
    assert_eq!(bar(1, 3), "██▊     ");
    assert_eq!(bar(0, 4), "        ");
}
