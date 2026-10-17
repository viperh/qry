//! The results grid's keys, and what a new result does to it: moving,
//! sorting, searching, copying, going to a row, column widths and freezing
//! the first column. Each result tab keeps its own.

use std::sync::Arc;

use qry_core::{
    QueryResult,
    copy::{self, CopyFormat, Selection},
    grid::{self, Order, Search, sort_permutation},
};

use super::{notices::notify, run};
use crate::{
    action::Action,
    app::{
        dialogs::{DialogState, copyform::CopyForm, gotoform::GotoForm},
        effects::Effect,
        state::{GridSearch, ResultsState, State},
    },
    notice::Notice,
};

/// Wider values are cut off at this many columns.
pub(crate) const MAX_COLUMN_WIDTH: u16 = 40;
/// How narrow and how wide `<` and `>` take a column.
pub(crate) const COLUMN_WIDTH_RANGE: (u16, u16) = (3, 200);
/// Rows a search looks at per event, so a huge result never holds up input.
pub(crate) const SEARCH_BUDGET: usize = 10_000;

/// Shows `result` from its first cell, unsorted.
pub(super) fn show(results: &mut ResultsState, result: Arc<QueryResult>) {
    results.widths = grid::column_widths(&result, MAX_COLUMN_WIDTH);
    results.result = Some(result);
    (results.row, results.col) = (0, 0);
    results.row_offset.set(0);
    results.col_offset.set(0);
    results.sort.clear();
    results.order = None;
    results.search = None;
    results.profile = None;
    results.record_scroll.set(0);
}

/// Adds `rows` to the end of the result, which is `truncated` while more
/// wait. The selection, the widths' fit, the sort and a search follow;
/// the inspector counts again.
pub(super) fn append(results: &mut ResultsState, rows: Vec<Vec<Option<String>>>, truncated: bool) {
    let kept = results.data_row(results.row);
    let Some(shared) = &mut results.result else {
        return;
    };
    let result = Arc::make_mut(shared);
    result.rows.extend(rows);
    result.truncated = truncated;
    let fresh = grid::column_widths(result, MAX_COLUMN_WIDTH);
    for (width, fresh) in results.widths.iter_mut().zip(fresh) {
        *width = (*width).max(fresh);
    }
    if !results.sort.is_empty() {
        results.order = Some(sort_permutation(result, &results.sort));
        results.row = results.grid_row(kept);
    }
    if let Some(search) = &mut results.search {
        search.search = Search::new(&search.input);
    }
    results.profile = None;
}

/// What a `Results` binding does.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
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
        Action::CancelQuery => {
            run::cancel(state, fx);
            return;
        }
        Action::FetchAll => return run::fetch_all(state),
        Action::FetchMore => return run::fetch_more(state, fx),
        _ => {}
    }
    let Some((rows, cols)) = state
        .results()
        .result
        .as_ref()
        .map(|r| (r.rows.len(), r.columns.len()))
    else {
        return;
    };
    match action {
        Action::CopyCell | Action::CopyRow => copy_now(state, action, fx),
        Action::CopyMenu => {
            state.open_dialog(DialogState::Copy(CopyForm::default()));
            state.awaiting = None;
        }
        Action::GotoRow => {
            state.open_dialog(DialogState::Goto(GotoForm::default()));
            state.awaiting = None;
        }
        _ => {}
    }
    let results = state.results_mut();
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
        Action::SortColumn => sort(results, false),
        Action::AddSortColumn => sort(results, true),
        Action::Search => {
            results.search = Some(GridSearch {
                typing: true,
                input: String::new(),
                search: Search::new(""),
            });
        }
        Action::NextMatch => jump(results, true),
        Action::PrevMatch => jump(results, false),
        Action::NarrowColumn | Action::WidenColumn => {
            if let Some(width) = results.widths.get_mut(results.col) {
                let next = if action == Action::NarrowColumn {
                    width.saturating_sub(1)
                } else {
                    width.saturating_add(1)
                };
                *width = next.clamp(COLUMN_WIDTH_RANGE.0, COLUMN_WIDTH_RANGE.1);
            }
        }
        Action::ResetWidths => {
            if let Some(result) = &results.result {
                results.widths = grid::column_widths(result, MAX_COLUMN_WIDTH);
            }
        }
        Action::FreezeColumn => results.frozen = !results.frozen,
        _ => {}
    }
}

/// `s` cycles the current column's sort: ascending, descending, off, as
/// the only key. `S` does the same to it as one more key.
fn sort(results: &mut ResultsState, add: bool) {
    let col = results.col;
    let kept = results.data_row(results.row);
    let at = results.sort.iter().position(|(c, _)| *c == col);
    let next = match at.map(|i| results.sort[i].1) {
        None => Some(Order::Asc),
        Some(Order::Asc) => Some(Order::Desc),
        Some(Order::Desc) => None,
    };
    if !add {
        // Cycles while it is the only key; otherwise it becomes the only
        // key, ascending.
        let only = at.is_some() && results.sort.len() == 1;
        results.sort = match (only, next) {
            (true, Some(order)) => vec![(col, order)],
            (true, None) => Vec::new(),
            (false, _) => vec![(col, Order::Asc)],
        };
    } else {
        match (at, next) {
            (Some(i), Some(order)) => results.sort[i].1 = order,
            (Some(i), None) => _ = results.sort.remove(i),
            (None, Some(order)) => results.sort.push((col, order)),
            (None, None) => {}
        }
    }
    results.order = match (&results.result, results.sort.is_empty()) {
        (Some(result), false) => Some(sort_permutation(result, &results.sort)),
        _ => None,
    };
    // A sorted column is wide enough for its name and mark (` ▼2`).
    if let Some(result) = &results.result {
        for &(c, _) in &results.sort {
            let wanted = grid::width(&result.columns[c].name) + 3;
            if let Some(width) = results.widths.get_mut(c) {
                *width = (*width).max(u16::try_from(wanted).unwrap_or(u16::MAX));
            }
        }
    }
    // The selection stays on the same row of data.
    results.row = results.grid_row(kept);
}

/// Selects the next (or previous) match after the selected cell.
fn jump(results: &mut ResultsState, forward: bool) {
    let Some(search) = &results.search else {
        return;
    };
    let at = (results.data_row(results.row), results.col);
    if let Some((row, col)) = search.search.next_from(at, forward) {
        results.row = results.grid_row(row);
        results.col = col;
    }
}

/// Whether the active result's search line is taking the keys.
pub(super) fn search_typing(state: &State) -> bool {
    state
        .results()
        .search
        .as_ref()
        .is_some_and(|search| search.typing)
}

/// A key while the search line is open: typing, Backspace, Enter to keep
/// the search and stop typing, Esc to drop it.
pub(super) fn search_key(state: &mut State, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    let results = state.results_mut();
    let Some(search) = &mut results.search else {
        return;
    };
    match key.code {
        KeyCode::Esc => {
            results.search = None;
            return;
        }
        KeyCode::Enter => {
            search.typing = false;
            return;
        }
        KeyCode::Backspace => _ = search.input.pop(),
        _ => match crate::app::dialogs::textinput::typed_char(key) {
            Some(c) => search.input.push(c),
            None => return,
        },
    }
    restart_search(results);
}

/// Searches again for what is typed, from the top, a first batch at once,
/// and selects the first match from the selected cell.
fn restart_search(results: &mut ResultsState) {
    let at = (results.data_row(results.row), results.col);
    let (Some(result), Some(search)) = (results.result.clone(), &mut results.search) else {
        return;
    };
    search.search = Search::new(&search.input);
    search.search.step(&result, SEARCH_BUDGET);
    let first = search.search.matches.iter().find(|&&m| m >= at).copied();
    if let Some((row, col)) = first.or_else(|| search.search.matches.first().copied()) {
        results.row = results.grid_row(row);
        results.col = col;
    }
}

/// Moves every unfinished search in the active session on by a batch.
/// Returns whether any moved, so the screen is drawn again.
pub(super) fn step_searches(state: &mut State) -> bool {
    let mut moved = false;
    for tab in &mut state.sessions.active_mut().tabs {
        let results = &mut tab.results;
        if let (Some(result), Some(search)) = (&results.result, &mut results.search)
            && !search.search.done(result)
        {
            search.search.step(result, SEARCH_BUDGET);
            moved = true;
        }
    }
    moved
}

/// What `y` and `Y` copy: the cell, or the row as TSV.
fn copy_now(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let results = state.results();
    let Some(result) = results.result.clone() else {
        return;
    };
    let row = results.data_row(results.row);
    let (selection, what) = if action == Action::CopyCell {
        (Selection::cell(row, results.col), "copied 1 cell")
    } else {
        (Selection::row(&result, row), "copied 1 row (TSV)")
    };
    copy_selection(state, &result, &selection, CopyFormat::Tsv, what, fx);
}

/// What the copy menu asked for: `scope` of the result in `format`.
pub(super) fn copy_as(state: &mut State, scope: usize, format: CopyFormat, fx: &mut Vec<Effect>) {
    let results = state.results();
    let Some(result) = results.result.clone() else {
        return;
    };
    let order: Vec<usize> = (0..result.rows.len())
        .map(|r| results.data_row(r))
        .collect();
    let row = results.data_row(results.row);
    let (selection, what) = match scope {
        0 => (Selection::cell(row, results.col), "1 cell".to_string()),
        1 => (Selection::row(&result, row), "1 row".to_string()),
        2 => (
            Selection::column(order, results.col),
            "1 column".to_string(),
        ),
        _ => {
            let n = result.rows.len();
            (
                Selection::all(&result, order),
                format!("{n} row{}", if n == 1 { "" } else { "s" }),
            )
        }
    };
    let what = format!("copied {what} ({})", format.name());
    copy_selection(state, &result, &selection, format, &what, fx);
}

fn copy_selection(
    state: &mut State,
    result: &QueryResult,
    selection: &Selection,
    format: CopyFormat,
    what: &str,
    fx: &mut Vec<Effect>,
) {
    let session = state.sessions.active();
    let table = session.tabs[session.active_tab.min(session.tabs.len() - 1)]
        .table
        .clone()
        .unwrap_or_else(|| "result".into());
    let text = copy::render(result, selection, format, session.dialect(), &table);
    fx.push(Effect::CopyToClipboard(text));
    notify(state, Notice::success(what), fx);
}

/// What the goto dialog asked for: row `N`, or `N%` of the way down.
/// Returns why it cannot, for the dialog to show.
pub(super) fn goto(state: &mut State, text: &str) -> Result<(), String> {
    let results = state.results_mut();
    let rows = results.result.as_ref().map_or(0, |r| r.rows.len());
    if rows == 0 {
        return Err("There are no rows to go to".into());
    }
    let text = text.trim();
    let row = if let Some(percent) = text.strip_suffix('%') {
        let p: f64 = percent
            .trim()
            .parse()
            .map_err(|_| format!("Not a percentage: {text}"))?;
        if !(0.0..=100.0).contains(&p) {
            return Err(format!("Not a percentage: {text}"));
        }
        ((p / 100.0 * rows as f64).ceil() as usize).max(1)
    } else {
        text.parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("Not a row number: {text}"))?
    };
    results.row = row.min(rows) - 1;
    Ok(())
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod results_tests;
