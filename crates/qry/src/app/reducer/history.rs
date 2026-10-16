//! The query history: recording each statement a session's worker
//! answered, the history dialog (ctrl-r) and recalling with alt-↑/↓.
//!
//! What is recorded is the statement's SQL with its secrets redacted, never
//! a connection's password. A connection's file is only appended to; once
//! it is 10% past the cap it is rewritten from what is kept. The scratch
//! database's history stays in memory unless the config says otherwise.

use qry_core::history::{Entry, History, redact, to_line};
use ratatui_textarea::CursorMove;

use super::{notices::notify, run};
use crate::{
    action::Action,
    app::{
        effects::Effect,
        history::{HistoryDialog, Kept, Recall},
        state::{EditorState, Focus, Overlay, SCRATCH_ID, SessionId, State},
    },
    history_store::{HistoryWrite, Loaded},
    notice::Notice,
};

/// How a recorded statement ended.
pub(super) struct Outcome {
    pub(super) ok: bool,
    pub(super) rows: Option<u64>,
    pub(super) ms: u64,
    pub(super) error: Option<String>,
}

/// Whether `connection`'s history goes to disk: not the scratch
/// database's, unless the config asks for it.
fn to_disk(state: &State, connection: &str) -> bool {
    let ephemeral = state
        .tree
        .position(connection)
        .map_or(connection == SCRATCH_ID, |i| {
            state.tree.connections[i].record.ephemeral
        });
    !ephemeral || state.config.history.ephemeral_to_disk
}

/// Session `id` ran `sql` with this outcome: it goes into its connection's
/// history, and to the file.
pub(super) fn record(
    state: &mut State,
    id: SessionId,
    sql: &str,
    outcome: Outcome,
    fx: &mut Vec<Effect>,
) {
    let config = &state.config.history;
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    if !config.enabled || sql.trim().is_empty() {
        return;
    }
    let connection = session.connection_id.clone();
    let words: Vec<&str> = config.redact.iter().map(String::as_str).collect();
    let entry = Entry {
        ts: Some(state.wall.unix),
        sql: redact(session.dialect(), sql, &words),
        ok: outcome.ok,
        rows: outcome.rows,
        ms: outcome.ms,
        error: outcome.error,
    };
    let cap = config.max_entries;
    let disk = to_disk(state, &connection);
    let line = to_line(&entry);
    let kept = state
        .history
        .by_connection
        .entry(connection.clone())
        .or_insert_with(|| Kept {
            history: History::new(cap),
            lines: 0,
        });
    kept.history.set_cap(cap);
    kept.history.push(entry);
    if !disk {
        return;
    }
    kept.lines += 1;
    fx.push(Effect::History(HistoryWrite::Append {
        connection: connection.clone(),
        line,
    }));
    if kept.lines > cap + cap / 10 {
        kept.lines = kept.history.len();
        fx.push(Effect::History(HistoryWrite::Rewrite {
            connection,
            content: kept.history.to_jsonl(),
        }));
    }
}

/// The files were read. What was recorded before they came is kept after
/// what they hold, unless the file has it already.
pub(super) fn loaded(state: &mut State, files: Vec<Loaded>) {
    let cap = state.config.history.max_entries;
    for file in files {
        let mut history = file.history;
        history.set_cap(cap);
        let mut lines = file.lines;
        if let Some(kept) = state.history.by_connection.get(&file.connection) {
            for e in kept.history.entries() {
                if !history.entries().any(|had| had == e) {
                    history.push(e.clone());
                    lines += 1;
                }
            }
        }
        state
            .history
            .by_connection
            .insert(file.connection, Kept { history, lines });
    }
    refresh(state);
}

/// A history file could not be written. The first failure of the run is
/// toasted; the rest are only logged.
pub(super) fn write_failed(state: &mut State, message: String, fx: &mut Vec<Effect>) {
    if !std::mem::replace(&mut state.history.write_failed, true) {
        notify(state, Notice::error(message), fx);
    }
}

/// Opens the history dialog on the active session's connection.
pub(super) fn open(state: &mut State) {
    let connection = state.sessions.active().connection_id.clone();
    let dialog = HistoryDialog::new(connection, &state.history);
    state.overlays.retain(|o| matches!(o, Overlay::Dialog(_)));
    state.overlays.push(Overlay::History(dialog));
}

fn dialog_mut(state: &mut State) -> Option<&mut HistoryDialog> {
    match state.overlays.last_mut() {
        Some(Overlay::History(dialog)) => Some(dialog),
        _ => None,
    }
}

fn close(state: &mut State) {
    state.overlays.retain(|o| !matches!(o, Overlay::History(_)));
}

/// Matches an open dialog again, after the history changed.
fn refresh(state: &mut State) {
    let book = &state.history;
    if let Some(Overlay::History(dialog)) = state.overlays.last_mut() {
        dialog.refilter(book);
    }
}

/// Types `c` into the dialog's query.
pub(super) fn type_char(state: &mut State, c: char) {
    if let Some(dialog) = dialog_mut(state) {
        dialog.input.insert(c);
        dialog.selected = 0;
    }
    refresh(state);
}

/// What a `Palette` binding does while the dialog is on top.
pub(super) fn run_key(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let Some(dialog) = dialog_mut(state) else {
        return;
    };
    let page = isize::try_from(crate::palette::MAX_ROWS).unwrap_or(1);
    let refilter = match action {
        Action::Up => {
            dialog.move_by(-1);
            false
        }
        Action::Down => {
            dialog.move_by(1);
            false
        }
        Action::PageUp => {
            dialog.move_by(-page);
            false
        }
        Action::PageDown => {
            dialog.move_by(page);
            false
        }
        Action::Left => {
            dialog.input.move_left();
            false
        }
        Action::Right => {
            dialog.input.move_right();
            false
        }
        Action::LineStart => {
            dialog.input.move_home();
            false
        }
        Action::LineEnd => {
            dialog.input.move_end();
            false
        }
        Action::DeleteBack | Action::DeleteForward => {
            if action == Action::DeleteBack {
                dialog.input.delete_back();
            } else {
                dialog.input.delete_forward();
            }
            dialog.selected = 0;
            true
        }
        // Tab: this connection or all of them.
        Action::NextField => {
            dialog.all = !dialog.all;
            dialog.selected = 0;
            true
        }
        Action::DeleteEntry if dialog.deleting => {
            delete(state, fx);
            return;
        }
        Action::DeleteEntry => {
            dialog.deleting = dialog.selected_row().is_some();
            false
        }
        Action::Submit => return pick(state, false, fx),
        Action::SubmitAndRun => return pick(state, true, fx),
        Action::Cancel => return close(state),
        _ => false,
    };
    if refilter {
        refresh(state);
    }
}

/// Puts the selected entry in the editor as a statement of its own, and
/// runs it if `run`.
fn pick(state: &mut State, run: bool, fx: &mut Vec<Effect>) {
    let Some(dialog) = dialog_mut(state) else {
        return;
    };
    let Some(row) = dialog.selected_row().cloned() else {
        return;
    };
    let Some(sql) = state
        .history
        .get(&row.connection)
        .and_then(|h| h.get(row.index))
        .map(|e| e.sql.clone())
    else {
        return;
    };
    close(state);
    insert_into_editor(state, &sql, run, fx);
}

/// Puts `sql` in the active session's editor as a new statement, focuses
/// it, and runs the statement if `run`.
pub(super) fn insert_into_editor(state: &mut State, sql: &str, run: bool, fx: &mut Vec<Effect>) {
    state.focus = Focus::Editor;
    insert_statement(state.editor_mut(), sql);
    if run {
        run::statement(state, fx);
    }
}

/// Deletes the selected entry, from memory and from its file.
fn delete(state: &mut State, fx: &mut Vec<Effect>) {
    let Some(row) = dialog_mut(state).and_then(|d| d.selected_row().cloned()) else {
        return;
    };
    let disk = to_disk(state, &row.connection);
    let Some(kept) = state.history.by_connection.get_mut(&row.connection) else {
        return;
    };
    if kept.history.remove(row.index).is_none() {
        return;
    }
    kept.lines = kept.history.len();
    if disk {
        fx.push(Effect::History(HistoryWrite::Rewrite {
            connection: row.connection,
            content: kept.history.to_jsonl(),
        }));
    }
    refresh(state);
}

/// Inserts `sql` at the cursor as a statement of its own: after the line
/// the cursor is on, with a `;` ending the statement before it if that has
/// none, and one ending it. The cursor ends up just after it, so running
/// the statement under the cursor runs it.
pub(crate) fn insert_statement(editor: &mut EditorState, sql: &str) {
    let sql = sql.trim().trim_end_matches(';').trim_end();
    let text = format!("{sql};");
    let t = &mut editor.textarea;
    t.cancel_selection();
    if t.lines().iter().all(|l| l.trim().is_empty()) {
        t.clear();
        t.insert_str(text);
        return;
    }
    t.move_cursor(CursorMove::End);
    let before = editor.query()[..editor.cursor_offset()]
        .trim_end()
        .to_string();
    let t = &mut editor.textarea;
    if before.is_empty() {
        t.insert_str(text);
        return;
    }
    if !before.ends_with(';') {
        t.insert_str(";");
    }
    t.insert_newline();
    t.insert_str(text);
}

/// alt-↑ (`older`) and alt-↓: on an empty editor, puts the connection's
/// newest statement in it, and each further press the one before or after,
/// like a shell. Past the newest the editor is empty again. Once the text
/// is edited, the keys do nothing until it is empty.
pub(super) fn recall(state: &mut State, older: bool) {
    let session = state.sessions.active();
    let id = session.id;
    let text = session.editor.query();
    let continuing = state
        .recall
        .as_ref()
        .filter(|r| r.session == id && r.text == text)
        .map(|r| r.index);
    let empty = text.trim().is_empty() && session.editor.textarea.cursor().0 == 0;
    if continuing.is_none() && !empty {
        return;
    }
    let Some(history) = state.history.get(&session.connection_id) else {
        return;
    };
    let len = history.len();
    let next = match (continuing, older) {
        (_, _) if len == 0 => return,
        (None, true) => Some(len - 1),
        (None, false) => return,
        (Some(i), true) => Some(i.min(len).saturating_sub(1)),
        (Some(i), false) => (i + 1 < len).then_some(i + 1),
    };
    let sql = next.and_then(|i| history.get(i)).map(|e| e.sql.clone());
    let editor = state.editor_mut();
    editor.textarea.cancel_selection();
    editor.textarea.clear();
    match (next, sql) {
        (Some(index), Some(sql)) => {
            editor.textarea.insert_str(&sql);
            let text = editor.query();
            state.recall = Some(Recall {
                session: id,
                index,
                text,
            });
        }
        _ => state.recall = None,
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
