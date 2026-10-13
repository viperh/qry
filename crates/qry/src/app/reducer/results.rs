//! The results grid's keys, and what a new result does to it.

use std::sync::Arc;

use qry_core::QueryResult;
use ratatui::text::Span;

use crate::{
    action::Action,
    app::state::{ResultsState, State},
};

/// Wider values are cut off at this many columns.
pub(crate) const MAX_COLUMN_WIDTH: u16 = 40;

/// Shows `result` from its first cell.
pub(super) fn show(results: &mut ResultsState, result: Arc<QueryResult>) {
    results.widths = column_widths(&result);
    results.result = Some(result);
    (results.row, results.col) = (0, 0);
    results.row_offset.set(0);
    results.col_offset.set(0);
}

/// What a `Results` binding does. Copying, searching and inspecting come
/// with the grid's redesign (T303, T304).
pub(super) fn run(state: &mut State, action: Action) {
    // `[` and `]` show the result before or after this one.
    let session = state.sessions.active_mut();
    match action {
        Action::PrevResult => {
            session.active_tab = session.active_tab.saturating_sub(1);
            return;
        }
        Action::NextResult => {
            session.active_tab = (session.active_tab + 1).min(session.tabs.len() - 1);
            return;
        }
        _ => {}
    }
    let results = state.results_mut();
    let Some((rows, cols)) = results
        .result
        .as_ref()
        .map(|r| (r.rows.len(), r.columns.len()))
    else {
        return;
    };
    let (last_row, last_col) = (rows.saturating_sub(1), cols.saturating_sub(1));
    let page = results.page_rows.get().max(1);
    match action {
        Action::Up => results.row = results.row.saturating_sub(1),
        Action::Down => results.row = (results.row + 1).min(last_row),
        Action::Left => results.col = results.col.saturating_sub(1),
        Action::Right => results.col = (results.col + 1).min(last_col),
        Action::PageUp => results.row = results.row.saturating_sub(page),
        Action::PageDown => results.row = (results.row + page).min(last_row),
        Action::LineStart => results.col = 0,
        Action::LineEnd => results.col = last_col,
        Action::Top => results.row = 0,
        Action::Bottom => results.row = last_row,
        _ => {}
    }
}

/// How wide each column is drawn: its widest value or its name, capped.
fn column_widths(result: &QueryResult) -> Vec<u16> {
    let width = |s: &str| Span::raw(s).width();
    result
        .columns
        .iter()
        .enumerate()
        .map(|(c, name)| {
            let widest = result
                .rows
                .iter()
                .map(|row| {
                    row.get(c)
                        .and_then(Option::as_deref)
                        .map_or(width("NULL"), width)
                })
                .fold(width(name), usize::max);
            u16::try_from(widest)
                .unwrap_or(u16::MAX)
                .clamp(1, MAX_COLUMN_WIDTH)
        })
        .collect()
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod results_tests;
