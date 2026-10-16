//! The outcomes of the files the runtime read or wrote.

use super::{connections, dialogs, notices::notify, tree};
use crate::{
    app::{
        effects::Effect,
        event::FileAnswer,
        state::{Awaiting, State},
    },
    notice::Notice,
};

pub(super) fn answer(state: &mut State, answer: FileAnswer, fx: &mut Vec<Effect>) {
    match answer {
        FileAnswer::ConnectionsLoaded(records) => tree::connections_loaded(state, records),
        // The runtime logged these too.
        FileAnswer::LoadFailed(message) | FileAnswer::SaveFailed(message) => {
            notify(state, Notice::error(message), fx);
        }
        FileAnswer::Imported(imported) => {
            dialogs::done(state, Awaiting::Import);
            connections::imported(state, *imported);
        }
        // The modal's footer is one line; the toast and the history have
        // room for the whole of a long path.
        FileAnswer::ImportFailed(message) => {
            dialogs::failed(state, Awaiting::Import, &message);
            notify(state, Notice::error(message), fx);
        }
        FileAnswer::ConnectionsExported { path, count } => {
            let noun = if count == 1 {
                "connection"
            } else {
                "connections"
            };
            dialogs::done(state, Awaiting::ExportConnections);
            notify(
                state,
                Notice::success(format!("Exported {count} {noun} to {path}")),
                fx,
            );
        }
        FileAnswer::ConnectionsExportExists(path) => connections::export_exists(state, path),
        FileAnswer::ConnectionsExportFailed(message) => {
            dialogs::failed(state, Awaiting::ExportConnections, &message);
            notify(state, Notice::error(message), fx);
        }
    }
}
