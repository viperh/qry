//! Unit tests for `results.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;

use super::*;
use crate::{
    app::App,
    views::results::results_tests::{grid, press},
};

#[test]
fn columns_are_as_wide_as_their_widest_value_up_to_a_cap() {
    let result = QueryResult {
        columns: vec!["id".into(), "n".into(), "long".into()],
        rows: vec![vec![Some("12345".into()), None, Some("x".repeat(100))]],
        affected: None,
    };
    assert_eq!(column_widths(&result), [5, 4, MAX_COLUMN_WIDTH]);
}

/// The selected row and column.
fn cell(app: &App) -> (usize, usize) {
    (app.state().results.row, app.state().results.col)
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
    press(&mut results, KeyCode::Char('g'), 1);
    press(&mut results, KeyCode::End, 1);
    assert_eq!(cell(&results), (0, 2));
}

#[test]
fn a_new_result_resets_the_selection() {
    let mut app = grid(5, 3);
    press(&mut app, KeyCode::Down, 3);
    press(&mut app, KeyCode::Right, 2);
    app.state.results.row_offset.set(2);
    app.state.results.col_offset.set(1);
    let next = grid(2, 2).state().results.result.clone().unwrap();
    show(&mut app.state.results, next);
    let results = &app.state().results;
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
    assert_eq!(app.state().results.row, 0);
}
