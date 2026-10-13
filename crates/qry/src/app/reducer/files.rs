//! The outcomes of the files the runtime read or wrote.

use super::{dialogs, tree};
use crate::app::{
    effects::Effect,
    event::FileAnswer,
    state::{Awaiting, State, StatusCode},
};

pub(super) fn answer(state: &mut State, answer: FileAnswer, fx: &mut Vec<Effect>) {
    match answer {
        FileAnswer::ConnectionsLoaded(records) => tree::connections_loaded(state, records),
        // The runtime logged these; there is nothing on screen to change.
        FileAnswer::LoadFailed(_) | FileAnswer::SaveFailed(_) => {}
        FileAnswer::Imported(imported) => {
            dialogs::done(state, Awaiting::Import);
            tree::import(state, *imported, fx);
        }
        // The modal's footer is one line; the status bar has room for the
        // whole of a long path.
        FileAnswer::ImportFailed(message) => {
            state.status = StatusCode::Error(message.clone());
            dialogs::failed(state, Awaiting::Import, &message);
        }
        FileAnswer::ConnectionsExported { path, count } => {
            let noun = if count == 1 {
                "connection"
            } else {
                "connections"
            };
            state.status = StatusCode::Success(format!("Exported {count} {noun} to {path}"));
            dialogs::done(state, Awaiting::ExportConnections);
        }
        FileAnswer::ConnectionsExportFailed(message) => {
            state.status = StatusCode::Error(message.clone());
            dialogs::failed(state, Awaiting::ExportConnections, &message);
        }
    }
}
