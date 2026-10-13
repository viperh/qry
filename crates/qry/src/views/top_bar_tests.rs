//! Unit tests for `top_bar.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    app::UiEvent,
    config::Config,
    testing::{AppHarness, buffer_lines, render_with, styled},
};

/// The app after startup with `n` sessions, all on scratch.
fn sessions(n: usize) -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    for _ in 1..n {
        h.keys("shift-enter");
    }
    h
}

fn bar(h: &AppHarness, width: u16) -> ratatui::buffer::Buffer {
    render_with(width, 1, |frame| {
        let state = h.state();
        draw(state, &state.theme, frame, frame.area());
    })
}

fn text(h: &AppHarness, width: u16) -> String {
    buffer_lines(&bar(h, width))[0].clone()
}

#[test]
fn tabs_overflow_with_ellipsis_and_keep_the_active_tab() {
    let mut h = sessions(12);
    let tenth = h.sessions()[9];
    h.state_mut().sessions.active = tenth;
    let line = text(&h, 80);
    assert!(line.starts_with(" qry  … "), "{line}");
    assert!(line.contains(" 10 scratch "), "{line}");
    assert!(!line.contains(" 1 scratch "), "{line}");
    assert!(line.contains(" … "), "both ends are cut: {line}");
    assert!(line.trim_end().ends_with("NORMAL"), "{line}");

    // The hit boxes are the tabs drawn.
    let hits = tab_hits(h.state(), Rect::new(0, 0, 80, 1));
    let (_, rect) = hits.iter().find(|(i, _)| *i == 9).unwrap();
    let drawn: String = line
        .chars()
        .skip(usize::from(rect.x))
        .take(usize::from(rect.width))
        .collect();
    assert_eq!(drawn, " 10 scratch ");
}

#[test]
fn connection_summary_is_dropped_before_tabs() {
    let h = sessions(3);
    assert!(text(&h, 120).contains("sqlite · :memory:"));
    let h = sessions(6);
    let line = text(&h, 80);
    assert!(!line.contains("sqlite"), "{line}");
    assert!(line.contains(" 6 scratch "), "every tab still fits: {line}");
}

#[test]
fn mode_pill_follows_mode_and_keeps_previous_under_dialogs() {
    let mut h = sessions(1);
    assert!(text(&h, 80).contains(" NORMAL "));
    h.keys("tab");
    assert!(text(&h, 80).contains(" EDIT "));
    // A dialog keeps the pill of what is underneath.
    h.keys("ctrl-n");
    assert!(text(&h, 80).contains(" EDIT "));
    h.keys("esc f1");
    assert!(text(&h, 80).contains(" EDIT "), "the help too");
    assert_eq!(h.state().pill(), crate::theme::Pill::Edit);
}

#[test]
fn top_bar_snapshots() {
    let mut h = sessions(3);
    h.keys("ctrl-2");
    insta::assert_snapshot!("top_bar_80x1", text(&h, 80));
    insta::assert_snapshot!("top_bar_160x1", text(&h, 160));
    insta::assert_snapshot!("top_bar_80x1_styled", styled(&bar(&h, 80)));
    insta::assert_snapshot!("top_bar_160x1_styled", styled(&bar(&h, 160)));
}
