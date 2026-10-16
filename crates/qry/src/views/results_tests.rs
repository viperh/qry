//! Unit tests for `results.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qry_core::QueryResult;
use ratatui::buffer::Buffer;

use super::*;
use crate::{
    app::{App, DbAnswer, SessionId, UiEvent},
    testing::{buffer_lines, render_with, styled},
};

/// Draws the results pane of `app` into a `width`x`height` terminal.
fn drawn(app: &App, width: u16, height: u16) -> Buffer {
    render_with(width, height, |frame| {
        draw(app.state(), &Theme::DARK, frame, frame.area());
    })
}

/// The rows of the results pane drawn at 40x6.
fn render(app: &App) -> Vec<String> {
    buffer_lines(&drawn(app, 40, 6))
}

pub(crate) fn press(app: &mut App, code: KeyCode, times: usize) {
    for _ in 0..times {
        // Terminals report capital letters with Shift held.
        let shift = matches!(code, KeyCode::Char(c) if c.is_ascii_uppercase());
        let modifiers = if shift {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.handle(UiEvent::Key(KeyEvent::new(code, modifiers)));
    }
}

/// An app with the results pane focused, showing `result`.
pub(crate) fn showing(result: QueryResult) -> App {
    let mut app = App::embedded();
    app.state.focus = Focus::Results;
    app.handle(UiEvent::Db(
        SessionId::FIRST,
        DbAnswer::QueryDone(Arc::new(result)),
    ));
    app
}

/// A focused results pane showing `qry_core::testing::grid(rows, cols)`:
/// columns c0, c1, … holding "r{row}c{col}".
pub(crate) fn grid(rows: usize, cols: usize) -> App {
    showing(qry_core::testing::grid(rows, cols))
}

#[test]
fn query_done_renders_header_rows_and_null() {
    let results = showing(QueryResult {
        columns: vec!["id".into(), "name".into()],
        rows: vec![
            vec![Some("1".into()), Some("alice".into())],
            vec![Some("2".into()), None],
        ],
        affected: Some(2),
        ..QueryResult::affected(0)
    });

    let lines = render(&results);
    assert!(
        lines[0].contains("id") && lines[0].contains("name"),
        "{lines:#?}"
    );
    assert!(
        lines[1].contains('1') && lines[1].contains("alice"),
        "{lines:#?}"
    );
    assert!(
        lines[2].contains('2') && lines[2].contains("NULL"),
        "{lines:#?}"
    );
}

#[test]
fn statement_without_columns_says_executed() {
    let results = showing(QueryResult {
        columns: vec![],
        rows: vec![],
        affected: Some(0),
        ..QueryResult::affected(0)
    });

    assert!(
        render(&results)
            .iter()
            .any(|line| line.contains("Statement executed"))
    );
}

#[test]
fn row_scrolling_keeps_the_selection_in_view() {
    assert_eq!(scroll_rows(0, 3, 5, 100), 0);
    assert_eq!(scroll_rows(0, 5, 5, 100), 1);
    assert_eq!(scroll_rows(10, 4, 5, 100), 4);
    // A taller view never leaves empty rows at the bottom.
    assert_eq!(scroll_rows(90, 95, 20, 100), 80);
}

#[test]
fn column_scrolling_keeps_the_selected_column_whole() {
    let widths = [10, 10, 10, 10];
    // 10 + 1 + 10 fits in 21, 32 does not.
    assert_eq!(scroll_columns(&widths, 0, 1, 21), 0);
    assert_eq!(scroll_columns(&widths, 0, 2, 21), 1);
    assert_eq!(scroll_columns(&widths, 3, 1, 21), 1);
    // A column wider than the view still gets selected on its own.
    assert_eq!(scroll_columns(&[50, 50], 0, 1, 20), 1);
}

#[test]
fn visible_columns_fill_the_width_and_cut_the_last_one() {
    assert_eq!(
        visible_columns(&[10, 10, 10], 0, 25),
        [(0, 10), (1, 10), (2, 3)]
    );
    assert_eq!(visible_columns(&[10, 10, 10], 1, 25), [(1, 10), (2, 10)]);
    assert_eq!(visible_columns(&[50], 0, 20), [(0, 20)]);
}

#[test]
fn the_selected_cell_is_highlighted() {
    let mut results = grid(3, 2);
    press(&mut results, KeyCode::Down, 1);
    press(&mut results, KeyCode::Right, 1);
    let buffer = drawn(&results, 40, 8);

    // The header, then rows: "r1c1" is on line 2, starting after "r1c0 ".
    let x = 4 + 1;
    assert_eq!(buffer[(x, 2)].symbol(), "r");
    let cursor = Theme::DARK.cursor_cell();
    assert_eq!(buffer[(x, 2)].style().fg, cursor.fg);
    assert_eq!(buffer[(x, 2)].style().bg, cursor.bg);
    assert!(buffer[(x, 2)].modifier.contains(Modifier::BOLD));
    assert_ne!(buffer[(x, 2)].bg, buffer[(0, 2)].bg, "the cell stands out");
}

/// The colours of the grid, the header and the selected cell, from the theme.
#[test]
fn results_grid_snapshot() {
    let mut results = grid(30, 8);
    press(&mut results, KeyCode::Down, 2);
    press(&mut results, KeyCode::Right, 1);
    insta::assert_snapshot!("results_grid_80x24", styled(&drawn(&results, 80, 24)));
}

#[test]
fn scrolls_down_past_the_bottom_and_keeps_the_header() {
    // 8 lines tall: 1 header = 7 visible data rows.
    let mut results = grid(20, 2);
    press(&mut results, KeyCode::Down, 7);
    let lines = buffer_lines(&drawn(&results, 40, 8));

    assert!(lines[0].contains("c0"), "{lines:#?}");
    assert!(lines[1].contains("r1c0"), "{lines:#?}");
    assert!(lines[7].contains("r7c0"), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("r0c0")), "{lines:#?}");
}

#[test]
fn scrolls_right_past_the_edge() {
    // Each column is 4 wide ("r0c9" style values); 20 columns never fit in 30.
    let mut results = grid(2, 20);
    press(&mut results, KeyCode::End, 1);
    let lines = buffer_lines(&drawn(&results, 30, 6));

    assert!(lines[0].contains("c19"), "{lines:#?}");
    assert!(lines[1].contains("r0c19"), "{lines:#?}");
    assert!(!lines[1].contains("r0c0 "), "{lines:#?}");
}
