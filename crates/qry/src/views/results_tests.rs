//! Unit tests for `results.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qry_core::{
    QueryResult,
    testing::{result, typed_result},
};
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
fn a_statement_without_columns_says_how_many_rows_it_changed() {
    let mut result = QueryResult::affected(3);
    result.elapsed = std::time::Duration::from_millis(12);
    let lines = render(&showing(result));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("✓ 3 rows affected · 12 ms")),
        "{lines:#?}"
    );
    let lines = render(&showing(QueryResult {
        affected: None,
        ..QueryResult::affected(0)
    }));
    assert!(
        lines.iter().any(|l| l.contains("Statement executed")),
        "{lines:#?}"
    );
}

#[test]
fn an_empty_result_shows_its_header_and_no_rows() {
    let lines = render(&showing(result(&["id"], &[])));
    assert!(lines[0].contains("id"), "{lines:#?}");
    assert!(lines[1].contains("(no rows)"), "{lines:#?}");
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
    assert_eq!(scroll_columns(&widths, 0, 1, 21, false), 0);
    assert_eq!(scroll_columns(&widths, 0, 2, 21, false), 1);
    assert_eq!(scroll_columns(&widths, 3, 1, 21, false), 1);
    // A column wider than the view still gets selected on its own.
    assert_eq!(scroll_columns(&[50, 50], 0, 1, 20, false), 1);
    // Frozen, the first column takes 11 of the 32, leaving room for two.
    assert_eq!(scroll_columns(&widths, 0, 2, 32, true), 1);
    assert_eq!(scroll_columns(&widths, 0, 3, 32, true), 2);
    assert_eq!(scroll_columns(&widths, 2, 0, 32, true), 2);
}

#[test]
fn types_show_under_the_names_and_numbers_line_up_right() {
    let app = showing(typed_result(
        &[("id", "int4"), ("name", "text")],
        &[&[Some("7"), Some("al")], &[Some("1234"), None]],
    ));
    let lines: Vec<String> = render(&app)
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    assert_eq!(
        lines[..4],
        ["  │id  │name", "  │int4│text", "1 │   7│al", "2 │1234│NULL"]
    );
}

#[test]
fn long_values_are_cut_with_an_ellipsis_and_control_characters_show() {
    let mut app = showing(result(&["v"], &[&[Some("line one\nline two")]]));
    press(&mut app, KeyCode::Char('<'), 5);
    let lines = render(&app);
    assert!(lines[1].starts_with("1 │line one↵li…"), "{lines:#?}");
}

#[test]
fn sorted_columns_are_marked_with_their_direction_and_priority() {
    let mut app = grid(3, 3);
    press(&mut app, KeyCode::Char('s'), 2);
    let lines = render(&app);
    assert!(lines[0].contains("c0 ▼"), "{lines:#?}");
    assert!(lines[1].contains("r2c0"), "descending: {lines:#?}");

    press(&mut app, KeyCode::Right, 1);
    press(&mut app, KeyCode::Char('S'), 1);
    let lines = render(&app);
    assert!(
        lines[0].contains("▼1") && lines[0].contains("▲2"),
        "{lines:#?}"
    );
    assert!(
        summary(app.state()).contains("sorted by c0 ▼, c1 ▲"),
        "{}",
        summary(app.state())
    );
}

#[test]
fn columns_off_the_edge_are_counted_in_the_header() {
    let lines = buffer_lines(&drawn(&grid(2, 20), 40, 6));
    assert!(lines[0].ends_with("│c3     → 16 more cols "), "{lines:#?}");
    assert!(lines[1].ends_with("│r0c3                  "), "{lines:#?}");
}

#[test]
fn a_frozen_first_column_stays_while_the_rest_scroll() {
    let mut app = grid(2, 20);
    press(&mut app, KeyCode::Char('F'), 1);
    press(&mut app, KeyCode::End, 1);
    let lines = buffer_lines(&drawn(&app, 30, 6));
    assert!(lines[1].starts_with("1 │r0c0┃"), "{lines:#?}");
    assert!(lines[1].contains("r0c19"), "{lines:#?}");
}

#[test]
fn the_search_line_counts_matches_and_highlights_them() {
    let mut app = grid(5, 2);
    press(&mut app, KeyCode::Char('/'), 1);
    for c in "c1".chars() {
        press(&mut app, KeyCode::Char(c), 1);
    }
    let buffer = drawn(&app, 40, 8);
    let lines = buffer_lines(&buffer);
    assert!(lines[7].starts_with("/c1"), "{lines:#?}");
    assert!(lines[7].contains("match 1 / 5"), "{lines:#?}");
    // r1c1 on the second row is a match, not the cursor.
    let x = 3 + 4 + 1;
    assert_eq!(buffer[(x, 2)].symbol(), "r");
    assert_eq!(buffer[(x, 2)].bg, Theme::DARK.match_highlight().bg.unwrap());
}

#[test]
fn the_selected_cell_is_highlighted() {
    let mut results = grid(3, 2);
    press(&mut results, KeyCode::Down, 1);
    press(&mut results, KeyCode::Right, 1);
    let buffer = drawn(&results, 40, 8);

    // The header, then rows: "r1c1" is on line 2, after the gutter and
    // "r1c0│".
    let x = 3 + 4 + 1;
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
    // Each column is at least 4 wide; 20 columns never fit in 30.
    let mut results = grid(2, 20);
    press(&mut results, KeyCode::End, 1);
    let lines = buffer_lines(&drawn(&results, 30, 6));

    assert!(lines[0].contains("c19"), "{lines:#?}");
    assert!(lines[1].contains("r0c19"), "{lines:#?}");
    assert!(!lines[1].contains("r0c0 "), "{lines:#?}");
}

/// The grid in each of its looks, plain and styled.
#[test]
fn grid_snapshots() {
    let snap = |name: &str, app: &App, w: u16, h: u16| {
        let buffer = drawn(app, w, h);
        insta::assert_snapshot!(name.to_string(), buffer_lines(&buffer).join("\n"));
        insta::assert_snapshot!(format!("{name}_styled"), styled(&buffer));
    };
    let mut basic = showing(typed_result(
        &[("id", "int8"), ("name", "text"), ("total", "numeric")],
        &[
            &[Some("1"), Some("alice"), Some("12.50")],
            &[Some("2"), Some("bob"), None],
            &[Some("3"), Some("carol"), Some("7.25")],
        ],
    ));
    press(&mut basic, KeyCode::Down, 2);
    press(&mut basic, KeyCode::Right, 1);
    snap("grid_basic_80x24", &basic, 80, 24);

    snap("grid_wide_overflow_hint_80x24", &grid(5, 30), 80, 24);

    let mut frozen = grid(5, 30);
    press(&mut frozen, KeyCode::Char('F'), 1);
    press(&mut frozen, KeyCode::End, 1);
    snap("grid_frozen_first_column_100x24", &frozen, 100, 24);

    let nulls = showing(result(
        &["v", ""],
        &[
            &[None, Some("NULL")],
            &[Some("a\tb\nc\0d"), Some(&"x".repeat(100))],
        ],
    ));
    snap("grid_nulls_and_control_chars_80x12", &nulls, 80, 12);

    let numeric = showing(typed_result(
        &[("n", "int4"), ("f", "float8"), ("s", "text")],
        &[
            &[Some("1"), Some("1.5"), Some("1")],
            &[Some("1000"), Some("-22.25"), Some("1000")],
        ],
    ));
    snap("grid_numeric_right_aligned_80x12", &numeric, 80, 12);

    let mut sorted = showing(result(
        &["a", "b"],
        &[
            &[Some("2"), Some("x")],
            &[Some("1"), Some("y")],
            &[Some("2"), Some("a")],
        ],
    ));
    press(&mut sorted, KeyCode::Char('s'), 2);
    press(&mut sorted, KeyCode::Right, 1);
    press(&mut sorted, KeyCode::Char('S'), 1);
    snap("grid_sorted_multi_key_80x12", &sorted, 80, 12);

    let mut search = grid(8, 3);
    press(&mut search, KeyCode::Char('/'), 1);
    press(&mut search, KeyCode::Char('1'), 1);
    snap("grid_search_matches_80x12", &search, 80, 12);

    snap(
        "grid_dml_affected_80x8",
        &showing(QueryResult::affected(3)),
        80,
        8,
    );
    snap(
        "grid_empty_rows_80x8",
        &showing(result(&["id", "name"], &[])),
        80,
        8,
    );
    snap("grid_tiny_terminal_10x3", &grid(50, 10), 10, 3);
    // Narrower than the gutter: nothing to draw, and no panic.
    drawn(&grid(5000, 3), 3, 2);
    drawn(&grid(5, 3), 0, 0);
}
