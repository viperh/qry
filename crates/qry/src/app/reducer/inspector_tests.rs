//! Unit tests for `inspector.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{QueryResult, testing::result};

use super::*;
use crate::{
    testing::AppHarness,
    views::inspector::inspector_tests::{describe_schema, on_orders, on_orders_with, ran},
};

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

fn detail_loads(effects: &[Effect]) -> Vec<SchemaPath> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Db(_, crate::db::DbCommand::LoadSchema { path, .. }) => Some(path.clone()),
            _ => None,
        })
        .filter(|p| matches!(p, SchemaPath::Details { .. }))
        .collect()
}

fn described(app: &AppHarness) -> Option<(String, String)> {
    let d = app.state().sessions.active().describe.as_ref()?;
    Some((d.catalog.clone(), d.object.clone()))
}

#[test]
fn describe_mode_from_tree_and_esc_returns() {
    let mut app = on_orders(describe_schema(), 160, 40);
    app.keys("d");
    // Not open in the tree, so not cached: it loads through LoadSchema.
    assert_eq!(
        detail_loads(&app.take_effects()),
        [SchemaPath::details("main", "orders")]
    );
    assert_eq!(described(&app), Some(("main".into(), "orders".into())));
    assert!(app.state().chrome.inspector);
    assert_eq!(app.state().focus, Focus::Inspector);
    app.advance(100);
    assert!(app.render(160, 40).contains("TABLE · main.orders"));

    app.keys("esc");
    assert_eq!(described(&app), None);
    assert_eq!(
        app.state().focus,
        Focus::Tree,
        "back to where d was pressed"
    );
    let text = app.render(160, 40);
    assert!(!text.contains("TABLE · main.orders"), "{text}");
    assert!(text.contains("INSPECTOR"), "the panel stays: {text}");
}

#[test]
fn describing_what_the_tree_has_loaded_loads_nothing() {
    let mut app = on_orders(describe_schema(), 160, 40);
    // Open orders in the tree, which loads its columns and indexes.
    app.keys("l");
    app.advance(100);
    app.take_effects();
    // From a column row, it is still the table.
    app.keys("j j d");
    assert_eq!(detail_loads(&app.take_effects()), []);
    assert_eq!(described(&app), Some(("main".into(), "orders".into())));
}

#[test]
fn d_on_a_row_that_is_no_table_says_so() {
    let mut app = on_orders(describe_schema(), 160, 40);
    app.keys("g g d");
    assert_eq!(described(&app), None);
    assert_eq!(app.state().focus, Focus::Tree);
    assert!(
        app.render(160, 40)
            .contains("select a table or view to describe")
    );
}

#[test]
fn a_failed_description_loads_again_on_d() {
    let schema = describe_schema().fail(SchemaPath::details("main", "orders"), "gone");
    let mut app = on_orders(schema, 160, 40);
    app.keys("d");
    app.advance(100);
    app.take_effects();
    // `tree::after` leaves a failure be; `d` asks again.
    app.keys("k");
    assert_eq!(detail_loads(&app.take_effects()), []);
    app.keys("esc d");
    assert_eq!(
        detail_loads(&app.take_effects()),
        [SchemaPath::details("main", "orders")]
    );
}

#[test]
fn a_cleared_cache_loads_the_description_again() {
    let mut app = on_orders(describe_schema(), 160, 40);
    app.keys("d");
    app.advance(100);
    app.take_effects();
    // As DDL or a reconnect leaves it.
    app.state_mut().sessions.active_mut().schema.clear();
    app.keys("j");
    assert_eq!(
        detail_loads(&app.take_effects()),
        [SchemaPath::details("main", "orders")]
    );
}

#[test]
fn up_and_down_scroll_the_description() {
    let mut app = on_orders(describe_schema(), 160, 40);
    app.keys("d");
    app.advance(100);
    let scroll = |app: &AppHarness| {
        app.state()
            .sessions
            .active()
            .describe
            .as_ref()
            .unwrap()
            .scroll
    };
    app.keys("j j");
    assert_eq!(scroll(&app), 2);
    app.keys("k");
    assert_eq!(scroll(&app), 1);
    // Five columns, two indexes, the title, a blank line and INDEXES.
    app.keys("G");
    assert_eq!(scroll(&app), 9);
    app.keys("j");
    assert_eq!(scroll(&app), 9);
    app.keys("g g");
    assert_eq!(scroll(&app), 0);
}

#[test]
fn d_in_the_results_describes_the_table_the_result_came_from() {
    let db = qry_core::testing::FakeDatabase::default().reply(crate::testing::Reply::Rows(result(
        &["id"],
        &[&[Some("1")]],
    )));
    let mut app = on_orders_with(db, describe_schema(), 160, 40);
    // Enter runs SELECT * FROM orders.
    app.keys("enter");
    app.advance(100);
    app.state_mut().focus = Focus::Results;
    app.take_effects();
    app.keys("d");
    assert_eq!(described(&app), Some(("main".into(), "orders".into())));
    assert_eq!(app.state().focus, Focus::Inspector);
    app.keys("esc");
    assert_eq!(app.state().focus, Focus::Results);
}

#[test]
fn d_in_the_results_needs_the_table_in_the_schema() {
    let mut app = ran(qry_core::testing::grid(3, 3), 160, 40);
    // `select * from users`, but nothing of the schema has loaded, so
    // there is no catalog to find it in.
    app.keys("d");
    assert_eq!(described(&app), None);
    assert_eq!(app.state().focus, Focus::Results);
    assert!(
        app.render(160, 40)
            .contains("this result was not read from one table")
    );
}

#[test]
fn one_table_is_told_from_joins_lists_and_unions() {
    let one = |sql| reads_one_table(Dialect::Postgres, sql);
    assert!(one("SELECT * FROM orders"));
    assert!(one(
        "select a, b from public.orders o where x in (1, 2) order by a, b"
    ));
    assert!(one(
        "SELECT * FROM orders WHERE id IN (SELECT id FROM users, teams)"
    ));
    assert!(!one(
        "SELECT * FROM orders o JOIN users u ON u.id = o.user_id"
    ));
    assert!(!one("SELECT * FROM orders, users"));
    assert!(!one("SELECT id FROM orders UNION SELECT id FROM users"));
    assert!(!one("SELECT 1"));
}
