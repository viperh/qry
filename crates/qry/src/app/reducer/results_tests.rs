//! Unit tests for `results.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;
use qry_core::testing::{FakeDatabase, Reply, grid as qgrid, result};

use super::*;
use crate::{
    app::{
        App, UiEvent,
        state::{Focus, MAX_TABS},
    },
    config::Config,
    testing::AppHarness,
    views::results::results_tests::{grid, press},
};

#[test]
fn columns_are_as_wide_as_their_widest_value_up_to_a_cap() {
    let result = QueryResult {
        columns: vec!["id".into(), "n".into(), "long".into()],
        rows: vec![vec![Some("12345".into()), None, Some("x".repeat(100))]],
        affected: None,
        ..QueryResult::affected(0)
    };
    assert_eq!(
        qry_core::grid::column_widths(&result, MAX_COLUMN_WIDTH),
        [5, 4, MAX_COLUMN_WIDTH]
    );
}

/// The selected row and column.
fn cell(app: &App) -> (usize, usize) {
    (app.state().results().row, app.state().results().col)
}

#[test]
fn keys_move_the_selection_and_stop_at_the_edges() {
    let mut results = grid(5, 3);
    press(&mut results, KeyCode::Up, 1);
    press(&mut results, KeyCode::Left, 1);
    assert_eq!(cell(&results), (0, 0));

    press(&mut results, KeyCode::Down, 2);
    press(&mut results, KeyCode::Char('l'), 9);
    assert_eq!(cell(&results), (2, 2));

    press(&mut results, KeyCode::Char('G'), 1);
    press(&mut results, KeyCode::Home, 1);
    assert_eq!(cell(&results), (4, 0));
    press(&mut results, KeyCode::Char('g'), 2);
    press(&mut results, KeyCode::End, 1);
    assert_eq!(cell(&results), (0, 2));
}

#[test]
fn a_new_result_resets_the_selection() {
    let mut app = grid(5, 3);
    press(&mut app, KeyCode::Down, 3);
    press(&mut app, KeyCode::Right, 2);
    app.state.results().row_offset.set(2);
    app.state.results().col_offset.set(1);
    let next = grid(2, 2).state().results().result.clone().unwrap();
    show(app.state.results_mut(), next);
    let results = app.state().results();
    assert_eq!(
        (
            results.row,
            results.col,
            results.row_offset.get(),
            results.col_offset.get()
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn keys_do_nothing_without_a_result() {
    let mut app = App::embedded();
    app.state.focus = crate::app::state::Focus::Results;
    press(&mut app, KeyCode::Down, 3);
    assert_eq!(app.state().results().row, 0);
}

/// A harness that ran `sql` against a fake database answering each
/// statement with `replies`, the results pane focused.
fn ran(sql: &str, replies: Vec<QueryResult>) -> (AppHarness, FakeDatabase) {
    let mut db = FakeDatabase::default();
    for result in replies {
        db = db.reply(Reply::Rows(result));
    }
    let mut h = AppHarness::new(Config::embedded()).with_database(db.clone());
    h.send(UiEvent::Started);
    h.keys("tab").type_str(sql).keys("f8");
    h.state_mut().focus = Focus::Results;
    (h, db)
}

fn at(h: &AppHarness) -> (usize, usize) {
    (h.state().results().row, h.state().results().col)
}

#[test]
fn tabs_keep_independent_state() {
    let (mut h, _) = ran("select 1; select 2;", vec![qgrid(5, 3), qgrid(5, 3)]);
    assert_eq!(h.state().sessions.active().tabs.len(), 2);
    h.keys("j j l s");
    let second = (at(&h), h.state().results().sort.clone());
    h.keys("[");
    assert_eq!(at(&h), (0, 0), "the first tab has its own selection");
    assert!(h.state().results().sort.is_empty());
    h.keys("]");
    assert_eq!((at(&h), h.state().results().sort.clone()), second);
}

#[test]
fn tab_cap_drops_oldest() {
    let sql = (0..MAX_TABS + 3)
        .map(|i| format!("select {i};"))
        .collect::<String>();
    let (h, _) = ran(&sql, (0..MAX_TABS + 3).map(|_| qgrid(1, 1)).collect());
    let tabs = &h.state().sessions.active().tabs;
    assert_eq!(tabs.len(), MAX_TABS);
    assert_eq!(tabs[0].number, 4, "the three oldest were dropped");
}

#[test]
fn s_cycles_sort_order() {
    let (mut h, _) = ran(
        "select 1;",
        vec![result(&["n"], &[&[Some("10")], &[None], &[Some("9")]])],
    );
    let shown = |h: &AppHarness| {
        let results = h.state().results();
        let r = results.result.as_ref().unwrap();
        (0..3)
            .map(|i| r.rows[results.data_row(i)][0].clone())
            .collect::<Vec<_>>()
    };
    let some = |s: &str| Some(s.to_string());
    h.keys("s");
    assert_eq!(h.state().results().sort, [(0, Order::Asc)]);
    assert_eq!(shown(&h), [some("9"), some("10"), None]);
    h.keys("s");
    assert_eq!(shown(&h), [None, some("10"), some("9")]);
    h.keys("s");
    assert!(h.state().results().sort.is_empty());
    assert_eq!(shown(&h), [some("10"), None, some("9")]);
}

#[test]
fn sorting_keeps_the_selection_on_the_same_row_of_data() {
    let (mut h, _) = ran("select 1;", vec![qgrid(5, 1)]);
    h.keys("j s s");
    let results = h.state().results();
    assert_eq!(results.data_row(results.row), 1);
    assert_eq!(results.row, 3);
}

#[test]
fn slash_search_then_n_moves_selection() {
    let (mut h, _) = ran("select 1;", vec![qgrid(20, 3)]);
    h.keys("/").type_str("c2").keys("enter");
    assert_eq!(at(&h), (0, 2), "the first match is selected");
    assert!(!h.state().results().search.as_ref().unwrap().typing);
    h.keys("n n");
    assert_eq!(at(&h), (2, 2));
    h.keys("N");
    assert_eq!(at(&h), (1, 2));
    h.keys("esc");
    assert!(
        h.state().results().search.is_some(),
        "Esc after Enter keeps it"
    );
    h.keys("/ esc");
    assert!(h.state().results().search.is_none());
}

#[test]
fn a_large_search_runs_a_batch_per_tick() {
    let (mut h, _) = ran("select 1;", vec![qgrid(SEARCH_BUDGET * 3, 1)]);
    h.keys("/").type_str("c0");
    let found = |h: &AppHarness| {
        h.state()
            .results()
            .search
            .as_ref()
            .unwrap()
            .search
            .matches
            .len()
    };
    assert_eq!(found(&h), SEARCH_BUDGET, "the first batch at once");
    let ticks = h.ticks();
    while h.ticks() == ticks {
        h.advance(10);
    }
    assert_eq!(found(&h), SEARCH_BUDGET * 2, "one more batch per tick");
    h.advance(10_000);
    assert_eq!(found(&h), SEARCH_BUDGET * 3);
}

#[test]
fn goto_row_dialog_accepts_number_and_percent() {
    let (mut h, _) = ran("select 1;", vec![qgrid(200, 1)]);
    h.keys("ctrl-g").type_str("42").keys("enter");
    assert_eq!(at(&h).0, 41);
    assert_eq!(h.dialog_title(), None);
    h.keys("ctrl-g").type_str("50%").keys("enter");
    assert_eq!(at(&h).0, 99);
    h.keys("ctrl-g").type_str("nope").keys("enter");
    assert!(
        h.dialog_title().is_some(),
        "a bad row keeps the dialog open"
    );
    h.keys("esc");
    assert_eq!(at(&h).0, 99);
}

#[test]
fn resize_column_bounds() {
    let (mut h, _) = ran("select 1;", vec![qgrid(1, 2)]);
    let width = |h: &AppHarness| h.state().results().widths[0];
    assert_eq!(width(&h), 4);
    h.keys("> >");
    assert_eq!(width(&h), 6);
    for _ in 0..10 {
        h.keys("<");
    }
    assert_eq!(width(&h), COLUMN_WIDTH_RANGE.0);
    for _ in 0..300 {
        h.keys(">");
    }
    assert_eq!(width(&h), COLUMN_WIDTH_RANGE.1);
    h.keys("=");
    assert_eq!(width(&h), 4);
    h.keys("F");
    assert!(h.state().results().frozen);
}

#[test]
fn copy_cell_emits_clipboard_effect_and_toast() {
    let (mut h, _) = ran("select 1;", vec![qgrid(3, 2)]);
    // Earlier toasts go first.
    h.advance(60_000);
    h.take_effects();
    h.keys("j l y");
    let effects = h.take_effects();
    assert!(
        effects.contains(&Effect::CopyToClipboard("r1c1".into())),
        "{effects:?}"
    );
    let text = h.render(80, 24);
    assert!(text.contains("copied 1 cell"), "{text}");

    h.keys("Y");
    assert!(
        h.take_effects()
            .contains(&Effect::CopyToClipboard("r1c0\tr1c1".into()))
    );
}

#[test]
fn the_copy_menu_copies_the_whole_result_as_markdown() {
    let (mut h, _) = ran("select 1;", vec![qgrid(2, 2)]);
    h.take_effects();
    // Scope: row → column → whole result; format: TSV → JSON → Markdown.
    h.keys("ctrl-y right right tab right right enter");
    let effects = h.take_effects();
    let text = effects.iter().find_map(|e| match e {
        Effect::CopyToClipboard(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(
        text.as_deref(),
        Some("| c0 | c1 |\n|---|---|\n| r0c0 | r0c1 |\n| r1c0 | r1c1 |"),
        "{effects:?}"
    );
}

#[test]
fn late_result_does_not_move_selection_of_other_tab() {
    // The third statement is slow; the first's tab is looked at meanwhile.
    let db = FakeDatabase::default()
        .reply(Reply::Rows(qgrid(5, 2)))
        .reply(Reply::Rows(qgrid(1, 1)))
        .reply(Reply::Delay(
            std::time::Duration::from_secs(5),
            Box::new(Reply::Rows(qgrid(3, 3))),
        ));
    let mut h = AppHarness::new(Config::embedded()).with_database(db);
    h.send(UiEvent::Started);
    h.keys("tab")
        .type_str("select 1; select 2; select 3;")
        .keys("f8");
    h.state_mut().focus = Focus::Results;
    assert_eq!(h.state().sessions.active().tabs.len(), 2);
    h.keys("[ j j l");
    h.advance(5_000);
    let session = h.state().sessions.active();
    assert_eq!(session.tabs.len(), 3, "the late result has its tab");
    assert_eq!(session.active_tab, 0, "but the first tab stays shown");
    assert_eq!(at(&h), (2, 1));
}

#[test]
fn the_flow_from_the_spec() {
    let rows: Vec<Vec<Option<String>>> = (0..200)
        .map(|i| {
            vec![
                Some(i.to_string()),
                Some(if i % 7 == 0 { "foo" } else { "bar" }.into()),
            ]
        })
        .collect();
    let mut table = result(&["id", "name"], &[]);
    table.rows = rows;
    let (mut h, _) = ran("select * from t", vec![table]);
    h.keys("G ctrl-g 5 0 enter");
    assert_eq!(at(&h).0, 49, "the 50th row");
    h.keys("l s s / f o o enter n");
    let results = h.state().results();
    // Sorted by name descending: the foo rows come last, in their order.
    assert_eq!(results.sort, [(1, Order::Desc)]);
    let data = results.data_row(results.row);
    assert_eq!(
        results.result.as_ref().unwrap().rows[data][1].as_deref(),
        Some("foo")
    );
    // The search line is under the toasts until they go.
    h.advance(60_000);
    let text = h.render(120, 40);
    assert!(
        text.contains("200 rows") && text.contains("sorted by name ▼"),
        "{text}"
    );
    assert!(
        text.contains("/foo  match ") && text.contains(" / 29"),
        "{text}"
    );
}
