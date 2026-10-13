//! Opening, submitting and closing the modals.
//!
//! Connecting, exporting and importing are only known to have worked once
//! the answer comes back, so a modal that starts one waits for it instead of
//! closing and losing everything that was typed: it shows what it is waiting
//! for, closes on success and keeps its fields on failure. [`Awaiting`]
//! records which answer that is, so a late one meant for a modal that has
//! since closed, or for a different one, changes nothing.

use super::{connect, tree};
use crate::{
    app::{
        dialogs::{
            DialogState, Submission, connexportform::ConnExportForm, connform::ConnForm,
            exportform::ExportForm, importform::ImportForm, passwordform::PasswordForm,
        },
        effects::Effect,
        state::{Awaiting, State},
    },
    config::KeySection,
    connections::StoredConnection,
    db::DbCommand,
};

/// Opens the modal of `section`, freshly filled in, or closes any with
/// `Home`. Whatever the old one was waiting for no longer matters.
pub(super) fn change_mode(state: &mut State, section: KeySection) {
    let dialog = match section {
        KeySection::Home => None,
        KeySection::AddConnModal => Some(DialogState::Connection(ConnForm::new(
            state.ids.connection_id(),
        ))),
        KeySection::ExpoModal => Some(DialogState::Export(ExportForm::default())),
        KeySection::PasswordModal => Some(DialogState::Password(PasswordForm::new(
            state.asked.clone().unwrap_or_default(),
        ))),
        KeySection::ImportModal => Some(DialogState::Import(ImportForm::default())),
        KeySection::ConnExportModal => Some(DialogState::ConnExport(ConnExportForm::default())),
    };
    match dialog {
        Some(dialog) => state.open_dialog(dialog),
        None => state.close_dialog(),
    }
    state.awaiting = None;
}

/// The tree asked for a connection to be changed.
pub(super) fn edit_connection(state: &mut State, record: &StoredConnection) {
    state.open_dialog(DialogState::Connection(ConnForm::editing(record)));
    state.awaiting = None;
}

/// The worker wants a password typed for `name`.
pub(super) fn need_password(state: &mut State, name: String) {
    state.asked = Some(name);
    change_mode(state, KeySection::PasswordModal);
}

/// Carries out what the open modal produced, or keeps it open showing why it
/// was refused. A second submit while one is waiting does nothing, so one
/// Enter is one attempt.
pub(super) fn submit(state: &mut State, fx: &mut Vec<Effect>) {
    if state.awaiting.is_some() {
        return;
    }
    let Some(dialog) = state.dialog_mut() else {
        return;
    };
    let submission = match dialog.form().submit() {
        Ok(submission) => submission,
        Err(error) => {
            dialog.form_mut().set_error(Some(error));
            return;
        }
    };
    let (awaiting, pending) = match &submission {
        // A typed password is the rest of a connection attempt, so the
        // prompt waits for the same answer.
        Submission::Connect { .. } | Submission::PasswordEntered(_) => {
            (Awaiting::Connect, "Connecting…")
        }
        Submission::Export(_) => (Awaiting::Export, "Exporting…"),
        Submission::Import { .. } => (Awaiting::Import, "Importing…"),
        Submission::ExportConnections(_) => (Awaiting::ExportConnections, "Exporting…"),
    };
    dialog.form_mut().set_error(None);
    dialog.form_mut().set_pending(Some(pending));
    state.awaiting = Some(awaiting);

    match submission {
        Submission::Connect { record, password } => {
            if let Some(password) = password {
                fx.push(Effect::StashPassword(password));
            }
            connect(state, *record, fx);
        }
        Submission::PasswordEntered(password) => {
            fx.push(Effect::StashPassword(password));
            fx.push(Effect::Db(state.session, DbCommand::PasswordEntered));
        }
        Submission::Export(config) => fx.push(Effect::Db(state.session, DbCommand::Export(config))),
        Submission::Import { path, format } => fx.push(Effect::Import { path, format }),
        // The tree holds the list to write.
        Submission::ExportConnections(path) => tree::export_connections(state, path, fx),
    }
}

/// The answer the modal waited for came: it is done, so close it.
pub(super) fn done(state: &mut State, answer: Awaiting) {
    if state.awaiting == Some(answer) {
        state.close_dialog();
        state.awaiting = None;
    }
}

/// The answer the modal waited for is a refusal: keep the modal, with
/// everything in it, and say why.
pub(super) fn failed(state: &mut State, answer: Awaiting, message: &str) {
    if state.awaiting != Some(answer) {
        return;
    }
    state.awaiting = None;
    if let Some(dialog) = state.dialog_mut() {
        dialog.form_mut().set_pending(None);
        dialog.form_mut().set_error(Some(message.to_string()));
    }
}
