//! The database workers' answers, each from one session.

use super::{dialogs, notices, run, tree};
use crate::app::{
    effects::Effect,
    event::DbAnswer,
    state::{Awaiting, SessionId, SessionStatus, State},
};

/// Handles an answer from session `id`, which the caller has checked is
/// still open.
pub(super) fn answer(state: &mut State, id: SessionId, answer: DbAnswer, fx: &mut Vec<Effect>) {
    match answer {
        DbAnswer::Connected(label) => {
            // A new connection starts outside any transaction.
            if let Some(session) = state.sessions.get_mut(id) {
                session.in_transaction = false;
            }
            set_status(
                state,
                id,
                SessionStatus::Open {
                    label: label.clone(),
                },
            );
            dialogs::done(state, Awaiting::Connect(id));
            tree::connected(state, id, label, fx);
        }
        DbAnswer::ConnectFailed(message) => {
            set_status(state, id, SessionStatus::Failed(message.clone()));
            dialogs::failed(state, Awaiting::Connect(id), &message);
            tree::connect_failed(state, id, fx);
        }
        // The worker cannot prompt, so it asks here.
        DbAnswer::NeedPassword {
            id: connection,
            name,
        } => {
            dialogs::need_password(state, id, connection, name);
        }
        DbAnswer::SecretNotStored(connection) => tree::secret_not_stored(state, &connection, fx),
        DbAnswer::QueryDone(result) => run::done(state, id, result, fx),
        DbAnswer::QueryFailed { message, position } => {
            run::failed(state, id, message, position, fx);
        }
        DbAnswer::Exported(_) => dialogs::done(state, Awaiting::Export(id)),
        DbAnswer::ExportFailed(message) => dialogs::failed(state, Awaiting::Export(id), &message),
        DbAnswer::TablesLoaded(tables) => tree::tables_loaded(state, id, tables),
        DbAnswer::TablesFailed(_) => tree::tables_failed(state, id),
        DbAnswer::Notice(notice) => notices::notify(state, notice, fx),
        // Nothing sent to it will be answered, so nothing is running there
        // any more; the status line says why.
        DbAnswer::WorkerStopped => {
            let failed = SessionStatus::Failed("database worker has stopped".into());
            set_status(state, id, failed);
            if let Some(session) = state.sessions.get_mut(id) {
                session.running = 0;
                session.tx_changes.clear();
                session.in_transaction = false;
            }
            run::stopped(state, id);
            dialogs::failed(state, Awaiting::Connect(id), "database worker has stopped");
            dialogs::failed(state, Awaiting::Export(id), "database worker has stopped");
        }
    }
}

fn set_status(state: &mut State, id: SessionId, status: SessionStatus) {
    if let Some(session) = state.sessions.get_mut(id) {
        session.status = status;
    }
}
