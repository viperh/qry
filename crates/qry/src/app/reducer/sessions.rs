//! Opening, switching and closing sessions.
//!
//! A session is one connection's worker plus the editor and results bound
//! to it. The reducer only keeps their data; the runtime starts and drops
//! the workers when told to with `Effect::OpenSession` and
//! `Effect::CloseSession`.

use crate::{
    action::Action,
    app::{
        dialogs::{DialogState, confirm::Confirm},
        effects::Effect,
        state::{SessionId, SessionStatus, State, scratch_record},
    },
    connections::StoredConnection,
    db::DbCommand,
    notice::Notice,
};

/// What a session binding does.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let sessions = &state.sessions;
    let count = sessions.order.len();
    let at = sessions
        .order
        .iter()
        .position(|id| *id == sessions.active)
        .unwrap_or(0);
    let target = match action {
        // Numbered from 1; one past the last does nothing.
        Action::SwitchSession(n) => usize::from(n)
            .checked_sub(1)
            .and_then(|i| sessions.order.get(i)),
        Action::NextSession => sessions.order.get((at + 1) % count),
        Action::PrevSession => sessions.order.get((at + count - 1) % count),
        Action::CloseSession => {
            close_active(state, fx);
            return;
        }
        _ => None,
    };
    if let Some(id) = target.copied() {
        switch(state, id);
    }
}

/// Shows session `id`'s editor and results. Focus stays on the same pane.
pub(super) fn switch(state: &mut State, id: SessionId) {
    if state.sessions.get(id).is_some() {
        state.sessions.active = id;
    }
}

/// Opens a new session on `record`, focuses it and starts its worker.
pub(super) fn open(
    state: &mut State,
    record: &StoredConnection,
    fx: &mut Vec<Effect>,
) -> SessionId {
    let id = state.sessions.add(record);
    state.sessions.active = id;
    fx.push(Effect::OpenSession(id, Box::new(record.clone())));
    id
}

/// Connects session `id` again, to `record` as it is now: after an edit, or
/// once it failed.
pub(super) fn reconnect(
    state: &mut State,
    id: SessionId,
    record: &StoredConnection,
    fx: &mut Vec<Effect>,
) {
    if let Some(session) = state.sessions.get_mut(id) {
        session.status = SessionStatus::Connecting;
        session.retarget(record);
        fx.push(Effect::Db(id, DbCommand::Connect(Box::new(record.clone()))));
    }
}

/// Closes the active session, asking first if a query is running there.
fn close_active(state: &mut State, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    if session.busy() {
        let form = Confirm::close_session(session.id, &session.title);
        state.open_dialog(DialogState::Confirm(form));
        state.awaiting = None;
        return;
    }
    let id = session.id;
    close(state, id, fx);
}

/// Closes session `id`: drops its worker, which ends any query it was
/// running, and forgets its editor and results. Closing the last one opens
/// a fresh scratch, so the editor always has somewhere to run.
pub(super) fn close(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.remove(id) else {
        return;
    };
    fx.push(Effect::CloseSession(id));
    if session.busy() {
        let text = format!("Closed {}: its running query was cancelled", session.title);
        super::notices::notify(state, Notice::warning(text), fx);
    }
    // Nobody is left to answer these.
    state.password_queue.retain(|request| request.session != id);
    if let Some(DialogState::Password(form)) = state.dialog()
        && form.session() == id
    {
        state.close_dialog();
        state.awaiting = None;
    }
    if state.sessions.order.is_empty() {
        let scratch = scratch_record();
        open(state, &scratch, fx);
    }
}

/// Opens `record` the way Enter on the tree does: switches to a session
/// already on it (connecting it again if it failed), or opens one.
pub(super) fn open_or_switch(
    state: &mut State,
    record: &StoredConnection,
    fx: &mut Vec<Effect>,
) -> SessionId {
    let Some(id) = state.sessions.for_connection(&record.id) else {
        return open(state, record, fx);
    };
    switch(state, id);
    let failed = state
        .sessions
        .get(id)
        .is_some_and(|s| matches!(s.status, SessionStatus::Failed(_) | SessionStatus::Closed));
    if failed {
        reconnect(state, id, record, fx);
    }
    id
}
