//! Running SQL: the statements of a run wait in the session's queue and go
//! to its worker one at a time, so one that fails can stop the rest; each
//! answer opens a result tab.

use std::sync::Arc;

use qry_core::{
    QueryResult,
    sql::{Dialect, TokenKind, is_read_only, lex, split, statement_at},
};

use super::{results, tx_change};
use crate::{
    app::{
        dialogs::{DialogState, confirm::Confirm},
        effects::Effect,
        state::{EditorState, Failed, Queued, SessionId, State, TabError},
    },
    connections::ConnLabel,
    db::DbCommand,
};

/// The statements of the editor's text, with their spans.
fn statements(editor: &EditorState, dialect: Dialect) -> Vec<Queued> {
    split(dialect, &editor.query())
        .into_iter()
        .map(|s| Queued {
            sql: s.text,
            span: Some(s.span),
        })
        .collect()
}

/// The statement under the cursor.
pub(super) fn statement(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    let editor = &session.editor;
    let all = statements(editor, session.dialect());
    let spans: Vec<qry_core::sql::Statement> = all
        .iter()
        .map(|q| qry_core::sql::Statement {
            span: q.span.clone().unwrap_or_default(),
            text: q.sql.clone(),
        })
        .collect();
    let offset = editor.cursor_offset();
    let picked = statement_at(&spans, offset).map(|i| all[i].clone());
    enqueue(state, picked.into_iter().collect(), fx);
}

/// Every statement, in order.
pub(super) fn all(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    let queued = statements(&session.editor, session.dialect());
    enqueue(state, queued, fx);
}

/// The selected text, split into statements, or the statement under the
/// cursor without a selection.
pub(super) fn selection(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    let editor = &session.editor;
    let Some((from, to)) = editor.textarea.selection_range() else {
        return statement(state, fx);
    };
    let (start, end) = (editor.offset_of(from), editor.offset_of(to));
    let text = editor.query();
    let selected = &text[start..end.max(start)];
    let queued = split(session.dialect(), selected)
        .into_iter()
        .map(|s| Queued {
            sql: s.text,
            span: Some(s.span.start + start..s.span.end + start),
        })
        .collect();
    enqueue(state, queued, fx);
}

/// Queues `statements` on the active session and starts the first if its
/// worker is free. A new run forgets the last one's failure.
pub(crate) fn enqueue(state: &mut State, statements: Vec<Queued>, fx: &mut Vec<Effect>) {
    if statements.is_empty() {
        return;
    }
    // The first write of a session on a prod connection asks first.
    let ask = state.config.confirm_writes_on_prod;
    let session = state.sessions.active();
    let writes = statements
        .iter()
        .any(|q| !is_read_only(session.dialect(), &q.sql));
    if ask && writes && session.label == Some(ConnLabel::Prod) && !session.writes_confirmed {
        let confirm = Confirm::run_on_prod(session.id, &session.title);
        state.sessions.active_mut().held = statements;
        state.open_dialog(DialogState::Confirm(confirm));
        state.awaiting = None;
        return;
    }
    let session = state.sessions.active_mut();
    session.failed = None;
    session.queue.extend(statements);
    let id = session.id;
    pump(state, id, fx);
}

/// The prod confirm said yes: session `id` runs what it held, and is not
/// asked again.
pub(crate) fn release(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.writes_confirmed = true;
    let held = std::mem::take(&mut session.held);
    if held.is_empty() {
        return;
    }
    session.failed = None;
    session.queue.extend(held);
    pump(state, id, fx);
}

/// The prod confirm went away without a yes: what it held is not run.
pub(crate) fn drop_held(state: &mut State) {
    for session in state.sessions.by_id.values_mut() {
        session.held.clear();
    }
}

/// Sends session `id`'s next statement, if its worker is free.
fn pump(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    if session.in_flight.is_some() {
        return;
    }
    let Some(next) = session.queue.pop_front() else {
        return;
    };
    session.running += 1;
    session.tx_changes.push_back(tx_change(&next.sql));
    fx.push(Effect::Db(id, DbCommand::Query(next.sql.clone())));
    session.in_flight = Some(next);
}

/// The name a result tab gives a statement: the first table after `FROM`,
/// else `result`.
fn table_name(dialect: Dialect, sql: &str) -> Option<String> {
    let tokens: Vec<_> = lex(dialect, sql)
        .into_iter()
        .filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment))
        .collect();
    let from = tokens.iter().position(|t| {
        t.kind == TokenKind::Keyword && sql[t.span.clone()].eq_ignore_ascii_case("from")
    })?;
    let mut name = String::new();
    for t in &tokens[from + 1..] {
        let text = &sql[t.span.clone()];
        match t.kind {
            TokenKind::Ident | TokenKind::QuotedIdent
                if !name.ends_with('.') && !name.is_empty() =>
            {
                break;
            }
            TokenKind::Ident => name.push_str(text),
            TokenKind::QuotedIdent => name.push_str(&text[1..text.len().saturating_sub(1)]),
            TokenKind::Punct if text == "." && !name.is_empty() => name.push('.'),
            _ => break,
        }
    }
    (!name.is_empty()).then_some(name)
}

/// The tab label for `result` of `sql`: `users · 42`, or `UPDATE · 3 rows`.
fn label(dialect: Dialect, sql: &str, result: &QueryResult) -> String {
    if result.columns.is_empty() {
        let verb = lex(dialect, sql)
            .into_iter()
            .find(|t| t.kind == TokenKind::Keyword)
            .map_or("statement".to_string(), |t| {
                sql[t.span].to_ascii_uppercase()
            });
        return match result.affected {
            Some(1) => format!("{verb} · 1 row"),
            Some(n) => format!("{verb} · {n} rows"),
            None => verb,
        };
    }
    let name = table_name(dialect, sql).unwrap_or_else(|| "result".into());
    format!("{name} · {}", result.rows.len())
}

/// Session `id`'s statement finished with `result`.
pub(super) fn done(
    state: &mut State,
    id: SessionId,
    result: Arc<QueryResult>,
    fx: &mut Vec<Effect>,
) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.running = session.running.saturating_sub(1);
    if let Some(Some(begins)) = session.tx_changes.pop_front() {
        session.in_transaction = begins;
    }
    let sql = session.in_flight.take().map(|q| q.sql).unwrap_or_default();
    let label = label(session.dialect(), &sql, &result);
    results::show(&mut session.new_tab(label).results, result);
    pump(state, id, fx);
}

/// Session `id`'s statement failed. The editor marks it, its tab says why,
/// and the rest of the run is dropped unless the config says to go on.
pub(super) fn failed(
    state: &mut State,
    id: SessionId,
    message: String,
    position: Option<usize>,
    fx: &mut Vec<Effect>,
) {
    let keep_going = state.config.continue_on_error;
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.running = session.running.saturating_sub(1);
    session.tx_changes.pop_front();
    let queued = session.in_flight.take();
    if let Some(Queued {
        span: Some(span), ..
    }) = &queued
    {
        session.failed = Some(Failed {
            span: span.clone(),
            position: position
                .map(|p| span.start + p)
                .filter(|p| span.contains(p)),
        });
    }
    let sql = queued.map(|q| q.sql).unwrap_or_default();
    session.new_tab("✗ error".into()).error = Some(TabError {
        message,
        position,
        sql,
    });
    if !keep_going {
        session.queue.clear();
    }
    pump(state, id, fx);
}

/// A dropped worker answers nothing more: forget what was waiting.
pub(super) fn stopped(state: &mut State, id: SessionId) {
    if let Some(session) = state.sessions.get_mut(id) {
        session.queue.clear();
        session.in_flight = None;
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
