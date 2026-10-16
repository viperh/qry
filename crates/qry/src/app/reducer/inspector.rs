//! The record inspector and the value popup (T304): showing and focusing
//! the inspector, counting the cursor's column a batch at a time, moving
//! through the RECORD fields, and the popup's keys.
//!
//! The RECORD selection is the grid's cursor column, so the inspector and
//! the grid always agree on the cell: `↑↓` in the inspector move the
//! grid's column, and `←→` its row.

use qry_core::{
    ValueKind,
    inspect::{self, Profiler, Rendered},
};

use super::notices::notify;
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{Focus, Overlay, State, ValuePopup},
    },
    notice::Notice,
    views::value_popup::content_size,
};

/// Rows a column's profile looks at; past them it is a sample.
pub(crate) const PROFILE_CAP: usize = 100_000;
/// Rows counted per event, so a huge result never holds up input.
pub(crate) const PROFILE_BUDGET: usize = 20_000;

/// `i`: shows or hides the inspector. Hiding it while it has focus gives
/// the focus back to the grid.
pub(super) fn toggle(state: &mut State) {
    state.chrome.inspector = !state.chrome.inspector;
    if !state.chrome.inspector && state.focus == Focus::Inspector {
        state.focus = Focus::Results;
    }
}

/// The pane after the focused one: the inspector follows the grid while
/// it is shown.
pub(super) fn focus_next(state: &State) -> Focus {
    match state.focus {
        Focus::Results if state.chrome.inspector => Focus::Inspector,
        focus => focus.next(),
    }
}

/// Counts another batch of the cursor's column while the inspector is
/// shown, starting afresh when the cursor has moved to another column.
/// Returns whether there is something new to draw.
pub(super) fn step_profile(state: &mut State) -> bool {
    if !state.chrome.inspector {
        return false;
    }
    let results = state.results_mut();
    let Some(result) = results.result.clone() else {
        return false;
    };
    if result.columns.is_empty() {
        return false;
    }
    let col = results.col;
    let fresh = results
        .profile
        .as_ref()
        .is_none_or(|profiler| profiler.column() != col);
    let profiler = results
        .profile
        .get_or_insert_with(|| Profiler::new(col, PROFILE_CAP));
    if fresh {
        *profiler = Profiler::new(col, PROFILE_CAP);
    } else if profiler.profile().complete {
        return false;
    }
    profiler.step(&result, PROFILE_BUDGET);
    true
}

/// What an `Inspector` binding does: `↑↓` pick a RECORD field (the grid's
/// column), `←→` the record before or after, `y` copies the field's value.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    if action == Action::CopyCell {
        copy_value(state, None, fx);
        return;
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
    match action {
        Action::Up => results.col = results.col.saturating_sub(1),
        Action::Down => results.col = (results.col + 1).min(last_col),
        Action::Top => results.col = 0,
        Action::Bottom => results.col = last_col,
        Action::Left => results.row = results.row.saturating_sub(1),
        Action::Right => results.row = (results.row + 1).min(last_row),
        _ => {}
    }
}

/// The selected cell: its column, its value, and the title the popup
/// gives it.
fn selected(state: &State) -> Option<(qry_core::ColumnMeta, Option<String>, String)> {
    let session = state.sessions.active();
    let results = session.results();
    let result = results.result.as_ref()?;
    let column = result.columns.get(results.col)?.clone();
    let value = result
        .rows
        .get(results.data_row(results.row))?
        .get(results.col)
        .cloned()
        .flatten();
    let table = session.tabs[session.active_tab.min(session.tabs.len() - 1)]
        .table
        .as_deref();
    let mut title = match table {
        Some(table) => format!("{table}.{}", column.name),
        None => column.name.clone(),
    };
    if !column.type_name.is_empty() {
        title.push_str(&format!(" · {}", column.type_name));
    }
    title.push_str(&format!(" · row {}", results.row + 1));
    Some((column, value, title))
}

/// `Enter`: opens the popup on the selected cell, wrapped and pretty.
pub(super) fn open_popup(state: &mut State) {
    let Some((column, value, title)) = selected(state) else {
        return;
    };
    let json = value
        .as_deref()
        .is_some_and(|v| column.kind != ValueKind::Binary && inspect::is_json(v, column.kind));
    let mut popup = ValuePopup {
        title,
        value,
        kind: column.kind,
        json,
        wrap: true,
        pretty: true,
        rendered: Rendered::Null,
        scroll: 0,
        hscroll: 0,
    };
    lay_out(&mut popup, state.size);
    state.overlays.push(Overlay::Value(popup));
}

/// Lays the popup's value out for its width at this terminal size, and
/// keeps the scroll position within it.
fn lay_out(popup: &mut ValuePopup, size: (u16, u16)) {
    let (width, _) = content_size(size);
    popup.rendered = inspect::render_value(
        popup.value.as_deref(),
        popup.kind,
        width,
        popup.wrap,
        popup.pretty,
    );
    clamp(popup, size);
}

fn clamp(popup: &mut ValuePopup, size: (u16, u16)) {
    let (width, height) = content_size(size);
    popup.scroll = popup
        .scroll
        .min(popup.rendered.len().saturating_sub(usize::from(height)));
    popup.hscroll = popup
        .hscroll
        .min(widest(&popup.rendered).saturating_sub(usize::from(width)));
}

/// The widest line's display width.
fn widest(rendered: &Rendered) -> usize {
    let width = qry_core::grid::width;
    match rendered {
        Rendered::Text(lines) | Rendered::Hex(lines) => {
            lines.iter().map(|l| width(l)).max().unwrap_or(0)
        }
        Rendered::Json(lines) => lines
            .iter()
            .map(|l| l.iter().map(|(_, t)| width(t)).sum())
            .max()
            .unwrap_or(0),
        Rendered::Null => 0,
    }
}

fn popup_mut(state: &mut State) -> Option<&mut ValuePopup> {
    match state.overlays.last_mut() {
        Some(Overlay::Value(popup)) => Some(popup),
        _ => None,
    }
}

/// Whether the value popup is on top.
pub(super) fn showing_popup(state: &State) -> bool {
    matches!(state.overlays.last(), Some(Overlay::Value(_)))
}

fn close_popup(state: &mut State) {
    state.overlays.retain(|o| !matches!(o, Overlay::Value(_)));
}

/// What a `Help`-context key does in the popup: scroll, or close.
pub(super) fn popup_act(state: &mut State, action: Action) {
    let size = state.size;
    let page = usize::from(content_size(size).1.max(1));
    let Some(popup) = popup_mut(state) else {
        return;
    };
    let scroll = &mut popup.scroll;
    match action {
        Action::Cancel => {
            close_popup(state);
            return;
        }
        Action::Up => *scroll = scroll.saturating_sub(1),
        Action::Down => *scroll += 1,
        Action::PageUp => *scroll = scroll.saturating_sub(page),
        Action::PageDown => *scroll += page,
        Action::Top => *scroll = 0,
        Action::Bottom => *scroll = usize::MAX,
        _ => {}
    }
    clamp(popup, size);
}

/// The popup's own letters: `y` copies the whole value, `w` wraps or not,
/// `f` shows JSON pretty or raw, `h`/`l` scroll sideways, `g` goes to the
/// top and `q` closes. Returns whether the key was one of them.
pub(super) fn popup_shortcut(state: &mut State, c: char, fx: &mut Vec<Effect>) -> bool {
    let size = state.size;
    let Some(popup) = popup_mut(state) else {
        return false;
    };
    match c {
        'y' => {
            let value = popup.value.clone();
            copy_value(state, Some(value), fx);
        }
        'w' => {
            popup.wrap = !popup.wrap;
            popup.hscroll = 0;
            lay_out(popup, size);
        }
        'f' if popup.json => {
            popup.pretty = !popup.pretty;
            popup.scroll = 0;
            lay_out(popup, size);
        }
        'h' => {
            popup.hscroll = popup.hscroll.saturating_sub(4);
        }
        'l' => {
            popup.hscroll += 4;
            clamp(popup, size);
        }
        'g' => popup.scroll = 0,
        'q' => close_popup(state),
        _ => return false,
    }
    true
}

/// The terminal changed size: the popup is laid out for its new width.
pub(super) fn resized(state: &mut State) {
    let size = state.size;
    for overlay in &mut state.overlays {
        if let Overlay::Value(popup) = overlay {
            lay_out(popup, size);
        }
    }
}

/// Copies a value in full, never cut as the grid shows it: `value`, or the
/// selected cell's when `None`.
fn copy_value(state: &mut State, value: Option<Option<String>>, fx: &mut Vec<Effect>) {
    let value = match value {
        Some(value) => value,
        None => match selected(state) {
            Some((_, value, _)) => value,
            None => return,
        },
    };
    match value {
        Some(text) => {
            fx.push(Effect::CopyToClipboard(text));
            notify(state, Notice::success("copied value"), fx);
        }
        None => notify(state, Notice::info("the value is NULL; nothing copied"), fx),
    }
}

#[cfg(test)]
#[path = "inspector_tests.rs"]
mod inspector_tests;
