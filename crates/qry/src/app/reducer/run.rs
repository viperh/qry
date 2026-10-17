//! Running SQL: the statements of a run wait in the session's queue and go
//! to its worker one at a time, so one that fails can stop the rest; each
//! answer opens a result tab.

use std::sync::Arc;

use qry_core::{
    QueryResult,
    sql::{Dialect, TokenKind, is_ddl, is_read_only, lex, split, statement_at},
};

use qry_core::jobs::{Cancelled, JobId, JobKind, JobState};

use super::{history, notices::notify, results, tx_change};
use crate::{
    app::{
        dialogs::{DialogState, confirm::Confirm},
        effects::Effect,
        event::MoreRows,
        state::{EditorState, Failed, Queued, SessionId, State, TabError},
    },
    connections::{ConnLabel, Driver},
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
    // The first write of a session on a prod connection asks first.
    let ask = state.config.confirm_writes_on_prod;
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    let writes = statements
        .iter()
        .any(|q| !is_read_only(session.dialect(), &q.sql));
    // The rollbacks asked for on the way out are not asked about again.
    if ask
        && writes
        && !state.quit_when_idle
        && session.label == Some(ConnLabel::Prod)
        && !session.writes_confirmed
    {
        let confirm = Confirm::run_on_prod(session.id, &session.title);
        if let Some(session) = state.sessions.get_mut(id) {
            session.held = statements;
        }
        state.open_dialog(DialogState::Confirm(confirm));
        state.awaiting = None;
        return;
    }
    let jobs = &mut state.jobs;
    // A statement closes the kept stream before it runs: the tab that had
    // it keeps its rows and loses `L`.
    let closes = statements.iter().any(|q| q.more.is_none());
    let statements: Vec<Queued> = statements
        .into_iter()
        .map(|mut q| {
            let kind = match (&q.export, q.more) {
                (Some(_), _) => JobKind::Export,
                (None, Some(_)) => JobKind::FetchMore,
                (None, None) => JobKind::Query,
            };
            q.job = Some(jobs.enqueue(id, kind, q.sql.clone()));
            q
        })
        .collect();
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    if closes {
        session.cursor = None;
    }
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

/// What `ctrl-c` on an Oracle session says.
pub(crate) const ORACLE_CANNOT_CANCEL: &str = "cancelling is not supported for Oracle yet: the \
     oracledb crate has no way to stop a running statement, so it keeps running";

/// Stops session `id`'s running statement and drops what waits behind it.
pub(super) fn cancel_session(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) -> bool {
    let now = state.now;
    let Some(session) = state.sessions.get_mut(id) else {
        return false;
    };
    let Some(job) = session.in_flight.as_ref().and_then(|q| q.job) else {
        return false;
    };
    // The oracledb crate has no way to stop a running call, so the
    // statement is left to run and the user told why.
    if session.driver == Driver::Oracle {
        let warning = Notice::warning(ORACLE_CANNOT_CANCEL);
        notify(state, warning, fx);
        return true;
    }
    let exporting = session
        .in_flight
        .as_ref()
        .is_some_and(|q| q.export.is_some());
    if state.jobs.cancel(job, now) != Cancelled::Running {
        return false;
    }
    for queued in session.queue.drain(..) {
        if let Some(job) = queued.job {
            state.jobs.cancel(job, now);
        }
    }
    fx.push(Effect::CancelQuery(id));
    let what = if exporting {
        "export cancelled"
    } else {
        "query cancelled"
    };
    notify(state, Notice::info(what), fx);
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

/// `A` on a truncated result: asks, then runs its statement again for
/// every row. No row count is known beforehand, so it always asks.
pub(super) fn fetch_all(state: &mut State) {
    let session = state.sessions.active();
    let tab = &session.tabs[session.active_tab.min(session.tabs.len() - 1)];
    if !tab.results.result.as_ref().is_some_and(|r| r.truncated) {
        return;
    }
    let writes = !is_read_only(session.dialect(), &tab.sql);
    let confirm = Confirm::fetch_all(session.id, tab.sql.clone(), writes);
    state.open_dialog(DialogState::Confirm(confirm));
    state.awaiting = None;
}

/// The fetch-all confirm said yes: session `id` runs `sql` again, keeping
/// every row. That closes the kept stream, if there is one.
pub(crate) fn fetch_all_confirmed(
    state: &mut State,
    id: SessionId,
    sql: String,
    fx: &mut Vec<Effect>,
) {
    let mut queued = Queued::new(sql, None);
    queued.all_rows = true;
    enqueue_on(state, id, vec![queued], fx);
}

/// `L` on a truncated result: the next `max_rows` rows from its kept
/// stream, without running the statement again. A result whose stream is
/// gone (another statement ran, or the backend cannot keep one) can only
/// fetch everything again, so `L` asks to, as `A` does.
pub(super) fn fetch_more(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    let tab = &session.tabs[session.active_tab.min(session.tabs.len() - 1)];
    if !tab.results.result.as_ref().is_some_and(|r| r.truncated) {
        return;
    }
    if !session.can_load_more() {
        return fetch_all(state);
    }
    // One read at a time: a second `L` waits for the first.
    let number = tab.number;
    let reading = session
        .in_flight
        .iter()
        .chain(&session.queue)
        .any(|q| q.more.is_some());
    if reading {
        return;
    }
    let mut queued = Queued::new(tab.sql.clone(), None);
    queued.more = Some(number);
    enqueue(state, vec![queued], fx);
}

/// The running statement of session `id` has read `rows` rows so far.
pub(super) fn progress(state: &mut State, id: SessionId, rows: u64) {
    let job = state
        .sessions
        .get(id)
        .and_then(|s| s.in_flight.as_ref())
        .and_then(|q| q.job);
    if let Some(job) = job {
        state.jobs.progress(job, rows);
    }
}

/// The rows a `FetchMore` read: added to the end of their tab's result.
/// The tab keeps `L` while more wait.
pub(super) fn more_rows(state: &mut State, id: SessionId, more: MoreRows, fx: &mut Vec<Effect>) {
    let now = state.now;
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    let queued = match &session.in_flight {
        Some(q) if q.more.is_some() => session.in_flight.take(),
        _ => None,
    };
    let Some(queued) = queued else {
        return;
    };
    session.running = session.running.saturating_sub(1);
    session.tx_changes.pop_front();
    if !more.more {
        session.cursor = None;
    }
    let job = queued.job;
    let cancelled =
        job.and_then(|j| state.jobs.get(j)).map(|j| &j.state) == Some(&JobState::Cancelled);
    // Rows a cancelled read brought are dropped, and with them the place
    // in the stream: the tab cannot load on from there.
    if cancelled {
        session.cursor = None;
        return pump(state, id, fx);
    }
    let added = more.rows.len();
    let number = queued.more;
    let dialect = session.dialect();
    if let Some(tab) = session.tabs.iter_mut().find(|t| Some(t.number) == number) {
        results::append(&mut tab.results, more.rows, more.more);
        if let Some(result) = &tab.results.result {
            tab.label = label(dialect, &tab.sql, result);
        }
        if let Some(error) = &more.error {
            tab.stopped = Some(error.clone());
        }
    }
    if let Some(job) = job {
        let ended = state
            .jobs
            .get(job)
            .and_then(|j| j.started)
            .map_or(now, |started| started + more.elapsed);
        match &more.error {
            Some(error) => state.jobs.fail(job, error.clone(), now),
            None => state.jobs.finish(job, added as u64, ended),
        }
    }
    let notice = match more.error {
        Some(error) => Notice::error(format!("could not load more rows: {error}")),
        None => Notice::success(format!(
            "loaded {} more row{}",
            qry_core::jobs::format_rows(added as u64),
            if added == 1 { "" } else { "s" }
        )),
    };
    notify(state, notice, fx);
    pump(state, id, fx);
}

/// The palette's `Toggle autocommit`: asks the active session's worker to
/// turn it the other way. The status line follows its answer.
pub(super) fn toggle_autocommit(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    if !session.is_open() {
        let notice = Notice::info("no open connection to change autocommit on");
        return notify(state, notice, fx);
    }
    fx.push(Effect::Db(
        session.id,
        DbCommand::SetAutocommit(!session.autocommit),
    ));
}

/// Session `id`'s worker turned autocommit on or off, or could not.
pub(super) fn autocommit_set(
    state: &mut State,
    id: SessionId,
    outcome: Result<bool, String>,
    fx: &mut Vec<Effect>,
) {
    let notice = match outcome {
        Ok(on) => {
            if let Some(session) = state.sessions.get_mut(id) {
                session.autocommit = on;
                // Turning it on commits; off, the status line says TX
                // from now on anyway.
                session.in_transaction = false;
            }
            if on {
                Notice::info("autocommit on")
            } else {
                Notice::info("autocommit off: changes wait for Commit")
            }
        }
        Err(message) => Notice::error(format!("could not change autocommit: {message}")),
    };
    notify(state, notice, fx);
}

/// The prod confirm said yes: session `id` runs what it held, and is not
/// asked again.
pub(crate) fn release(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.writes_confirmed = true;
    let held = std::mem::take(&mut session.held);
    enqueue_on(state, id, held, fx);
}

/// The prod confirm went away without a yes: what it held is not run.
pub(crate) fn drop_held(state: &mut State) {
    for session in state.sessions.by_id.values_mut() {
        session.held.clear();
    }
}

/// Session `id`'s export job `job` has ended: it is no longer what the
/// session runs, and what it did to the transaction holds if it `ok`.
/// Returns whether it was what the session was running.
pub(super) fn export_ended(state: &mut State, id: SessionId, job: JobId, ok: bool) -> bool {
    let Some(session) = state.sessions.get_mut(id) else {
        return false;
    };
    if session.in_flight.as_ref().and_then(|q| q.job) != Some(job) {
        return false;
    }
    session.in_flight = None;
    session.running = session.running.saturating_sub(1);
    if let Some(Some(begins)) = session.tx_changes.pop_front()
        && ok
    {
        session.in_transaction = begins;
    }
    true
}

/// Sends session `id`'s next statement, if its worker is free.
pub(super) fn pump(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
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
    // Reading on from a stream does nothing to the transaction.
    let change = match next.more {
        Some(_) => None,
        None => tx_change(&next.sql),
    };
    session.tx_changes.push_back(change);
    let command = match &next.export {
        // The export answers with its job, so its progress finds it.
        Some(export) => DbCommand::Export(Box::new(crate::db::ExportJob {
            job: next.job,
            ..(**export).clone()
        })),
        None if next.more.is_some() => DbCommand::FetchMore,
        None if next.all_rows => DbCommand::QueryAll(next.sql.clone()),
        None => DbCommand::Query(next.sql.clone()),
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
pub(super) fn label(dialect: Dialect, sql: &str, result: &QueryResult) -> String {
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

/// Session `id`'s statement finished with `result`. It goes into the
/// history too.
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
    let kept = std::mem::take(&mut session.cursor_kept);
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
    tab.sql = sql.clone();
    let number = tab.number;
    if kept && result.truncated {
        session.cursor = Some(number);
    }
    let Some(tab) = session.tabs.iter_mut().find(|t| t.number == number) else {
        return;
    };
    let outcome = history::Outcome {
        ok: true,
        // The rows it returned, or else those it changed.
        rows: if result.columns.is_empty() {
            result.affected
        } else {
            u64::try_from(result.rows.len()).ok()
        },
        ms: u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX),
        error: None,
    };
    results::show(&mut tab.results, result);
    history::record(state, id, &sql, outcome, fx);
    pump(state, id, fx);
}

/// Session `id`'s statement read `result`, then failed with `message`: the
/// rows are shown, with a footer saying why they stop there, and the job
/// and the run end as a failure does.
pub(super) fn stopped(
    state: &mut State,
    id: SessionId,
    result: Arc<QueryResult>,
    message: String,
    fx: &mut Vec<Effect>,
) {
    let keep_going = state.config.continue_on_error;
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.running = session.running.saturating_sub(1);
    session.tx_changes.pop_front();
    session.cursor_kept = false;
    let queued = session.in_flight.take();
    let job = queued.as_ref().and_then(|q| q.job);
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
            position: None,
        });
    }
    let sql = queued.map(|q| q.sql).unwrap_or_default();
    let label = label(session.dialect(), &sql, &result);
    let table = table_name(session.dialect(), &sql);
    let tab = session.new_tab(label);
    tab.table = table;
    tab.sql = sql.clone();
    tab.stopped = Some(message.clone());
    let outcome = history::Outcome {
        ok: false,
        rows: u64::try_from(result.rows.len()).ok(),
        ms: u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX),
        error: Some(message),
    };
    results::show(&mut tab.results, result);
    if !keep_going {
        for queued in session.queue.drain(..) {
            if let Some(job) = queued.job {
                state.jobs.cancel(job, state.now);
            }
        }
    }
    history::record(state, id, &sql, outcome, fx);
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
    session.cursor_kept = false;
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
        message: message.clone(),
        position,
        sql: sql.clone(),
    });
    if !keep_going {
        // The rest of the run is dropped, never sent.
        for queued in session.queue.drain(..) {
            if let Some(job) = queued.job {
                state.jobs.cancel(job, state.now);
            }
        }
    }
    // How long it ran, on the job's clock (ticks, so coarse).
    let ms = job
        .and_then(|j| state.jobs.get(j))
        .and_then(|j| j.elapsed(state.now))
        .map_or(0, |e| u64::try_from(e.as_millis()).unwrap_or(u64::MAX));
    let outcome = history::Outcome {
        ok: false,
        rows: None,
        ms,
        error: Some(message),
    };
    history::record(state, id, &sql, outcome, fx);
    pump(state, id, fx);
}

/// A dropped worker answers nothing more: forget what was waiting.
pub(super) fn worker_gone(state: &mut State, id: SessionId) {
    if let Some(session) = state.sessions.get_mut(id) {
        session.queue.clear();
        session.in_flight = None;
        session.cursor = None;
        session.cursor_kept = false;
    }
    state.jobs.close_session(id, state.now);
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
