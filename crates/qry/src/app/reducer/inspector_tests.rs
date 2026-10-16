//! Unit tests for `inspector.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{QueryResult, testing::result};

use super::*;
use crate::{testing::AppHarness, views::inspector::inspector_tests::ran};

/// `rows` rows of one column, every tenth value repeated.
fn many(rows: usize) -> QueryResult {
    let mut r = result(&["n"], &[]);
    r.rows = (0..rows)
        .map(|i| vec![Some((i % 10).to_string())])
        .collect();
    r
}

fn profile(app: &AppHarness) -> &Profiler {
    app.state().results().profile.as_ref().unwrap()
}

#[test]
fn profiling_runs_a_batch_per_event_and_stops_at_the_cap() {
    let mut app = ran(many(PROFILE_CAP + 50_000), 160, 40);
    assert!(app.state().results().profile.is_none(), "not while hidden");
    app.keys("i");
    // One batch for the key, so input is never held up by a big result.
    assert_eq!(profile(&app).profile().sampled, PROFILE_BUDGET as u64);
    assert!(!profile(&app).profile().complete);
    app.advance(10_000);
    let done = profile(&app).profile();
    assert!(done.complete);
    assert_eq!(done.sampled, PROFILE_CAP as u64, "a sample past the cap");
    assert_eq!(done.distinct, 10);
    let text = app.render(160, 40);
    assert!(text.contains("sampled 100\u{2009}000 rows"), "{text}");
    assert!(
        text.contains("TOP VALUES · 100\u{2009}000 fetched rows"),
        "{text}"
    );
}

#[test]
fn moving_to_another_column_starts_a_new_count() {
    let mut app = ran(qry_core::testing::grid(3, 3), 160, 40);
    app.keys("i");
    assert_eq!(profile(&app).column(), 0);
    assert!(profile(&app).profile().complete);
    app.keys("l");
    assert_eq!(profile(&app).column(), 1);
}

#[test]
fn tab_reaches_the_inspector_only_while_it_is_shown() {
    let mut app = ran(qry_core::testing::grid(3, 3), 160, 40);
    app.keys("tab");
    assert_eq!(app.state().focus, Focus::Tree);
    app.state_mut().focus = Focus::Results;
    app.keys("i tab");
    assert_eq!(app.state().focus, Focus::Inspector);
    assert_eq!(app.contexts()[0], crate::keymap::KeyContext::Inspector);
}

#[test]
fn the_popup_takes_its_keys_before_the_panes() {
    let mut app = ran(qry_core::testing::grid(3, 3), 80, 24);
    app.keys("enter");
    assert!(showing_popup(app.state()));
    assert_eq!(app.contexts(), [crate::keymap::KeyContext::Help]);
    // `j` scrolls the popup, not the grid.
    app.keys("j");
    assert_eq!(app.state().results().row, 0);
}
