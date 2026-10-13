//! The results grid's keys, and what a new result does to it.

use std::sync::Arc;

use crossterm::event::KeyEvent;
use qry_core::QueryResult;
use ratatui::text::Span;

use crate::{
    app::state::{ResultsState, State},
    keymap::ResultsCommand,
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

pub(super) fn key(state: &mut State, key: KeyEvent) {
    let results = &mut state.results;
    let Some((rows, cols)) = results
        .result
        .as_ref()
        .map(|r| (r.rows.len(), r.columns.len()))
    else {
        return;
    };
    let Some(command) = state.config.panes.results.get(key) else {
        return;
    };
    let (last_row, last_col) = (rows.saturating_sub(1), cols.saturating_sub(1));
    let page = results.page_rows.get().max(1);
    match command {
        ResultsCommand::Up => results.row = results.row.saturating_sub(1),
        ResultsCommand::Down => results.row = (results.row + 1).min(last_row),
        ResultsCommand::Left => results.col = results.col.saturating_sub(1),
        ResultsCommand::Right => results.col = (results.col + 1).min(last_col),
        ResultsCommand::PageUp => results.row = results.row.saturating_sub(page),
        ResultsCommand::PageDown => results.row = (results.row + page).min(last_row),
        ResultsCommand::FirstColumn => results.col = 0,
        ResultsCommand::LastColumn => results.col = last_col,
        ResultsCommand::FirstRow => results.row = 0,
        ResultsCommand::LastRow => results.row = last_row,
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
