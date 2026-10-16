//! Running SQL: the statements of a run wait in the session's queue and go
//! to its worker one at a time, so one that fails can stop the rest; each
//! answer opens a result tab.

use std::sync::Arc;

use qry_core::{
    QueryResult,
    sql::{Dialect, TokenKind, is_ddl, lex, split, statement_at},
};

use qry_core::jobs::{Cancelled, JobId, JobKind, JobState};

use super::{notices::notify, results, tx_change};
use crate::{
    app::{
        effects::Effect,
        state::{EditorState, Failed, Queued, SessionId, State, TabError},
    },
    db::DbCommand,
    notice::Notice,
};

/// The statements of the editor's text, with their spans.
fn statements(editor: &EditorState, dialect: Dialect) -> Vec<Queued> {
    split(dialect, &editor.query())
        .into_iter()
        .map(|s| Queued::new(s.text, Some(s.span)))
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
        .map(|s| Queued::new(s.text, Some(s.span.start + start..s.span.end + start)))
        .collect();
    enqueue(state, queued, fx);
}

/// Queues `statements` on the active session and starts the first if its
/// worker is free. A new run forgets the last one's failure.
pub(crate) fn enqueue(state: &mut State, statements: Vec<Queued>, fx: &mut Vec<Effect>) {
    let id = state.sessions.active().id;
    enqueue_on(state, id, statements, fx);
}

/// Queues `statements` on session `id`, as [`enqueue`] does on the active
/// one.
pub(crate) fn enqueue_on(
    state: &mut State,
    id: SessionId,
    statements: Vec<Queued>,
    fx: &mut Vec<Effect>,
) {
    if statements.is_empty() || state.sessions.get(id).is_none() {
        return;
    }
    let jobs = &mut state.jobs;
    let statements: Vec<Queued> = statements
        .into_iter()
        .map(|mut q| {
            q.job = Some(jobs.enqueue(id, JobKind::Query, q.sql.clone()));
            q
        })
        .collect();
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.failed = None;
    session.queue.extend(statements);
    pump(state, id, fx);
}

/// Quits once every session is idle, if quitting waits on that.
pub(super) fn quit_if_idle(state: &State, fx: &mut Vec<Effect>) {
    if state.quit_when_idle && !state.sessions.iter().any(|s| s.busy()) {
        fx.push(Effect::Quit);
    }
}

/// `ctrl-c`: stops the active session's running statement and drops the
/// rest of its run. Returns whether anything was running.
pub(super) fn cancel(state: &mut State, fx: &mut Vec<Effect>) -> bool {
    let id = state.sessions.active().id;
    cancel_session(state, id, fx)
}

/// Stops session `id`'s running statement and drops what waits behind it.
pub(super) fn cancel_session(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) -> bool {
    let now = state.now;
    let Some(session) = state.sessions.get_mut(id) else {
        return false;
    };
    let Some(job) = session.in_flight.as_ref().and_then(|q| q.job) else {
        return false;
    };
    if state.jobs.cancel(job, now) != Cancelled::Running {
        return false;
    }
    for queued in session.queue.drain(..) {
        if let Some(job) = queued.job {
            state.jobs.cancel(job, now);
        }
    }
    fx.push(Effect::CancelQuery(id));
    notify(state, Notice::info("query cancelled"), fx);
    true
}

/// Drops one waiting job, never sent.
pub(super) fn drop_queued(state: &mut State, job: JobId) {
    let now = state.now;
    let owner = state.jobs.get(job).map(|j| j.session);
    if let Some(session) = owner.and_then(|id| state.sessions.get_mut(id)) {
        session.queue.retain(|q| q.job != Some(job));
    }
    state.jobs.cancel(job, now);
}

/// `A` on a truncated result: runs its statement again for every row.
pub(super) fn fetch_all(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    let tab = &session.tabs[session.active_tab.min(session.tabs.len() - 1)];
    if !tab.results.result.as_ref().is_some_and(|r| r.truncated) {
        return;
    }
    let mut queued = Queued::new(tab.sql.clone(), None);
    queued.all_rows = true;
    enqueue(state, vec![queued], fx);
}

/// Sends session `id`'s next statement, if its worker is free.
fn pump(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    if session.in_flight.is_some() {
        return;
    }
    if session.queue.is_empty() {
        return quit_if_idle(state, fx);
    }
    let Some(next) = session.queue.pop_front() else {
        return;
    };
    session.running += 1;
    session.tx_changes.push_back(tx_change(&next.sql));
    let command = if next.all_rows {
        DbCommand::QueryAll(next.sql.clone())
    } else {
        DbCommand::Query(next.sql.clone())
    };
    fx.push(Effect::Db(id, command));
    if let Some(job) = next.job {
        state.jobs.start(job, state.now);
    }
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
    let queued = session.in_flight.take();
    let job = queued.as_ref().and_then(|q| q.job);
    // A cancelled statement that finished anyway: its result is dropped.
    if job.and_then(|j| state.jobs.get(j)).map(|j| &j.state) == Some(&JobState::Cancelled) {
        return pump(state, id, fx);
    }
    if let Some(job) = job {
        let ended = state
            .jobs
            .get(job)
            .and_then(|j| j.started)
            .map_or(state.now, |started| started + result.elapsed);
        state.jobs.finish(job, result.rows.len() as u64, ended);
    }
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    let sql = queued.map(|q| q.sql).unwrap_or_default();
    // The schema may have changed: what the tree holds of it loads again.
    if is_ddl(session.dialect(), &sql) {
        session.schema.clear();
    }
    let label = label(session.dialect(), &sql, &result);
    let table = table_name(session.dialect(), &sql);
    let tab = session.new_tab(label);
    tab.table = table;
    tab.sql = sql;
    results::show(&mut tab.results, result);
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
    let job = queued.as_ref().and_then(|q| q.job);
    // The failure a cancel causes says nothing new.
    if job.and_then(|j| state.jobs.get(j)).map(|j| &j.state) == Some(&JobState::Cancelled) {
        return pump(state, id, fx);
    }
    if let Some(job) = job {
        state.jobs.fail(job, message.clone(), state.now);
    }
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
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
        // The rest of the run is dropped, never sent.
        for queued in session.queue.drain(..) {
            if let Some(job) = queued.job {
                state.jobs.cancel(job, state.now);
            }
        }
    }
    pump(state, id, fx);
}

/// A dropped worker answers nothing more: forget what was waiting.
pub(super) fn stopped(state: &mut State, id: SessionId) {
    if let Some(session) = state.sessions.get_mut(id) {
        session.queue.clear();
        session.in_flight = None;
    }
    state.jobs.close_session(id, state.now);
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
