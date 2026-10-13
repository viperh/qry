//! Unit tests for `results.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::buffer::Buffer;

use super::*;
use crate::testing::{buffer_lines, render_with, styled};

/// Draws `results` into a `width`x`height` terminal.
fn drawn(results: &mut Results, width: u16, height: u16) -> Buffer {
    render_with(width, height, |frame| {
        results.draw(frame, frame.area(), &Theme::DARK).unwrap();
    })
}

/// The rows of `results` drawn at 40x6.
fn render(results: &mut Results) -> Vec<String> {
    buffer_lines(&drawn(results, 40, 6))
}

fn press(results: &mut Results, code: KeyCode, times: usize) {
    for _ in 0..times {
        // Terminals report capital letters with Shift held.
        let shift = matches!(code, KeyCode::Char(c) if c.is_ascii_uppercase());
        let modifiers = if shift {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        results
            .handle_key_event(KeyEvent::new(code, modifiers))
            .unwrap();
    }
}

/// A results pane showing `qry_core::testing::grid(rows, cols)`: columns
/// c0, c1, … holding "r{row}c{col}".
fn grid(rows: usize, cols: usize) -> Results {
    let mut results = Results::default();
    results.register_config_handler(Config::embedded()).unwrap();
    results
        .update(Action::QueryDone(Arc::new(qry_core::testing::grid(
            rows, cols,
        ))))
        .unwrap();
    results
}

#[test]
fn query_done_renders_header_rows_and_null() {
    let mut results = Results::default();
    results
        .update(Action::QueryDone(Arc::new(QueryResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![Some("1".into()), Some("alice".into())],
                vec![Some("2".into()), None],
            ],
            affected: Some(2),
        })))
        .unwrap();

    let lines = render(&mut results);
    assert!(
        lines[1].contains("id") && lines[1].contains("name"),
        "{lines:#?}"
    );
    assert!(
        lines[2].contains('1') && lines[2].contains("alice"),
        "{lines:#?}"
    );
    assert!(
        lines[3].contains('2') && lines[3].contains("NULL"),
        "{lines:#?}"
    );
}

#[test]
fn statement_without_columns_says_executed() {
    let mut results = Results::default();
    results
        .update(Action::QueryDone(Arc::new(QueryResult {
            columns: vec![],
            rows: vec![],
            affected: Some(0),
        })))
        .unwrap();

    assert!(
        render(&mut results)
            .iter()
            .any(|line| line.contains("Statement executed"))
    );
}

#[test]
fn columns_are_as_wide_as_their_widest_value_up_to_a_cap() {
    let result = QueryResult {
        columns: vec!["id".into(), "n".into(), "long".into()],
        rows: vec![vec![Some("12345".into()), None, Some("x".repeat(100))]],
        affected: None,
    };
    assert_eq!(column_widths(&result), [5, 4, MAX_COLUMN_WIDTH]);
}

#[test]
fn keys_move_the_selection_and_stop_at_the_edges() {
    let mut results = grid(5, 3);
    press(&mut results, KeyCode::Up, 1);
    press(&mut results, KeyCode::Left, 1);
    assert_eq!((results.row, results.col), (0, 0));

    press(&mut results, KeyCode::Down, 2);
    press(&mut results, KeyCode::Char('l'), 9);
    assert_eq!((results.row, results.col), (2, 2));

    press(&mut results, KeyCode::Char('G'), 1);
    press(&mut results, KeyCode::Home, 1);
    assert_eq!((results.row, results.col), (4, 0));
    press(&mut results, KeyCode::Char('g'), 1);
    press(&mut results, KeyCode::End, 1);
    assert_eq!((results.row, results.col), (0, 2));
}

#[test]
fn a_new_result_resets_the_selection() {
    let mut results = grid(5, 3);
    press(&mut results, KeyCode::Down, 3);
    press(&mut results, KeyCode::Right, 2);
    results.show(grid(2, 2).result.unwrap());
    assert_eq!(
        (
            results.row,
            results.col,
            results.row_offset,
            results.col_offset
        ),
        (0, 0, 0, 0)
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
    results.set_focus(true);
    press(&mut results, KeyCode::Down, 1);
    press(&mut results, KeyCode::Right, 1);
    let buffer = drawn(&mut results, 40, 8);

    // Border, then header, then rows: "r1c1" is on line 3, starting after "r1c0 ".
    let x = 1 + 4 + 1;
    assert_eq!(buffer[(x, 3)].symbol(), "r");
    let cursor = Theme::DARK.cursor_cell();
    assert_eq!(buffer[(x, 3)].style().fg, cursor.fg);
    assert_eq!(buffer[(x, 3)].style().bg, cursor.bg);
    assert!(buffer[(x, 3)].modifier.contains(Modifier::BOLD));
    assert_ne!(buffer[(x, 3)].bg, buffer[(1, 3)].bg, "the cell stands out");
    assert!(buffer_lines(&buffer)[0].contains("row 2/3 · col 2/2"));
}

/// The colours of the grid, the header and the selected cell, from the theme.
#[test]
fn results_grid_snapshot() {
    let mut results = grid(30, 8);
    results.set_focus(true);
    press(&mut results, KeyCode::Down, 2);
    press(&mut results, KeyCode::Right, 1);
    insta::assert_snapshot!("results_grid_80x24", styled(&drawn(&mut results, 80, 24)));
}

#[test]
fn scrolls_down_past_the_bottom_and_keeps_the_header() {
    // 8 lines tall: 2 border + 1 header = 5 visible data rows.
    let mut results = grid(20, 2);
    press(&mut results, KeyCode::Down, 7);
    let lines = buffer_lines(&drawn(&mut results, 40, 8));

    assert!(lines[1].contains("c0"), "{lines:#?}");
    assert!(lines[2].contains("r3c0"), "{lines:#?}");
    assert!(lines[6].contains("r7c0"), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("r0c0")), "{lines:#?}");
}

#[test]
fn scrolls_right_past_the_edge() {
    // Each column is 4 wide ("r0c9" style values); 20 columns never fit in 30.
    let mut results = grid(2, 20);
    press(&mut results, KeyCode::End, 1);
    let lines = buffer_lines(&drawn(&mut results, 30, 6));

    assert!(lines[1].contains("c19"), "{lines:#?}");
    assert!(lines[2].contains("r0c19"), "{lines:#?}");
    assert!(!lines[2].contains("r0c0 "), "{lines:#?}");
}

#[test]
fn keys_do_nothing_without_a_result() {
    let mut results = Results::default();
    press(&mut results, KeyCode::Down, 3);
    assert_eq!(results.row, 0);
}
