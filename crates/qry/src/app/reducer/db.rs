//! The database worker's answers.

use super::{dialogs, results, tree};
use crate::app::{
    effects::Effect,
    event::DbAnswer,
    state::{Awaiting, State, StatusCode},
};

pub(super) fn answer(state: &mut State, answer: DbAnswer, fx: &mut Vec<Effect>) {
    match answer {
        DbAnswer::Connected(label) => {
            dialogs::done(state, Awaiting::Connect);
            tree::connected(state, label, fx);
        }
        DbAnswer::ConnectFailed(message) => {
            dialogs::failed(state, Awaiting::Connect, &message);
            tree::connect_failed(state);
        }
        // The worker cannot prompt, so it asks here.
        DbAnswer::NeedPassword { id, name } => dialogs::need_password(state, id, name),
        DbAnswer::SecretNotStored(id) => tree::secret_not_stored(state, &id, fx),
        DbAnswer::QueryDone(result) => results::show(&mut state.results, result),
        DbAnswer::QueryFailed(_) => {}
        DbAnswer::Exported(_) => dialogs::done(state, Awaiting::Export),
        DbAnswer::ExportFailed(message) => dialogs::failed(state, Awaiting::Export, &message),
        DbAnswer::TablesLoaded(tables) => tree::tables_loaded(state, tables),
        DbAnswer::TablesFailed(_) => tree::tables_failed(state),
        DbAnswer::Status(status) => state.status = status,
        DbAnswer::WorkerStopped => {
            state.status = StatusCode::Error("database worker has stopped".into());
        }
    }
}
