//! Opening, submitting and closing the modals.
//!
//! Connecting, exporting and importing are only known to have worked once
//! the answer comes back, so a modal that starts one waits for it instead of
//! closing and losing everything that was typed: it shows what it is waiting
//! for, closes on success and keeps its fields on failure. [`Awaiting`]
//! records which answer that is, so a late one meant for a modal that has
//! since closed, or for a different one, changes nothing.

use std::time::Duration;

use super::{connect, connections, run, sessions};
use crate::{
    action::Action,
    app::{
        dialogs::{
            DialogState, Submission, confirm::OnConfirm, exportform::ExportForm,
            importform::ImportForm, passwordform::PasswordForm,
        },
        effects::Effect,
        event::Timer,
        state::{Awaiting, PasswordRequest, SessionId, State},
    },
    db::DbCommand,
};

/// Opens the modal `action` asks for, freshly filled in, in place of any
/// open one. Whatever the old one was waiting for no longer matters.
pub(super) fn open(state: &mut State, action: Action) {
    let dialog = match action {
        Action::NewConnection => return connections::new_connection(state),
        Action::ExportConnections => return connections::open_export(state),
        Action::ExportResults => DialogState::Export(ExportForm::default()),
        Action::ImportConnections => DialogState::Import(ImportForm::default()),
        _ => return,
    };
    state.open_dialog(dialog);
    state.awaiting = None;
}

/// Closes the modal, and stops waiting for its answer. A password prompt
/// that was waiting its turn comes up next.
fn close(state: &mut State) {
    // A run held for the prod confirm goes with it.
    run::drop_held(state);
    state.close_dialog();
    state.awaiting = None;
    if !state.password_queue.is_empty() {
        let next = state.password_queue.remove(0);
        prompt(state, next);
    }
}

/// What a `Dialog` binding does: submit, close, or move about the form.
pub(super) fn act(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    match action {
        Action::Submit => submit(state, fx),
        Action::TestConnection => connections::test(state, fx),
        Action::Cancel => close(state),
        action => {
            if let Some(dialog) = state.dialog_mut() {
                dialog.form_mut().act(action);
            }
        }
    }
}

/// Session `session`'s worker wants a password typed for connection `id`,
/// called `name`. While another session's prompt is open, this one waits
/// its turn, so each typed password goes back to the worker that asked.
pub(super) fn need_password(state: &mut State, session: SessionId, id: String, name: String) {
    let request = PasswordRequest {
        session,
        connection: id,
        name,
    };
    match state.dialog() {
        Some(DialogState::Password(form)) if form.session() != session => {
            state.password_queue.push(request);
        }
        _ => prompt(state, request),
    }
}

fn prompt(state: &mut State, request: PasswordRequest) {
    let form = PasswordForm::new(request.session, request.connection, request.name);
    state.open_dialog(DialogState::Password(form));
    state.awaiting = None;
}

/// Carries out what the open modal produced, or keeps it open showing why it
/// was refused. A second submit while one is waiting does nothing, so one
/// Enter is one attempt.
pub(super) fn submit(state: &mut State, fx: &mut Vec<Effect>) {
    if state.awaiting.is_some() {
        return;
    }
    // Enter on the connection dialog's `[ Test ]` button tests.
    if let Some(DialogState::Connection(dialog)) = state.dialog()
        && dialog.on_test_button()
    {
        return connections::test(state, fx);
    }
    let Some(dialog) = state.dialog_mut() else {
        return;
    };
    if !dialog.form_mut().check() {
        return;
    }
    let submission = match dialog.form().submit() {
        Ok(submission) => submission,
        Err(error) => {
            dialog.form_mut().set_error(Some(error));
            return;
        }
    };
    let pending = match &submission {
        // A typed password is the rest of a connection attempt, so the
        // prompt waits for the same answer.
        Submission::Connect { .. } | Submission::PasswordEntered { .. } => Some("Connecting…"),
        Submission::Export(_) | Submission::ExportConnections { .. } => Some("Exporting…"),
        Submission::Import { .. } => Some("Importing…"),
        Submission::ImportSelected(_) | Submission::Confirmed(_) | Submission::Dismissed => None,
    };
    dialog.form_mut().set_error(None);
    dialog.form_mut().set_pending(pending);
    if pending.is_some() {
        start_spinner(state, fx);
    }

    let awaiting = match submission {
        Submission::Connect { record, password } => {
            if let Some(password) = password {
                fx.push(Effect::StashPassword {
                    connection: record.id.clone(),
                    password,
                });
            }
            Awaiting::Connect(connect(state, *record, fx))
        }
        Submission::PasswordEntered {
            session,
            connection,
            password,
        } => {
            fx.push(Effect::StashPassword {
                connection,
                password,
            });
            fx.push(Effect::Db(session, DbCommand::PasswordEntered));
            Awaiting::Connect(session)
        }
        Submission::Export(config) => {
            let session = state.sessions.active;
            fx.push(Effect::Db(session, DbCommand::Export(config)));
            Awaiting::Export(session)
        }
        Submission::Import { path, format } => {
            fx.push(Effect::Import { path, format });
            Awaiting::Import
        }
        // The tree holds the list to write.
        Submission::ExportConnections { path, ids } => {
            state.awaiting = Some(Awaiting::ExportConnections);
            connections::export(state, path, &ids, false, fx);
            return;
        }
        Submission::ImportSelected(records) => {
            connections::import_selected(state, records, fx);
            return;
        }
        // Nothing to wait for: a confirm closes now.
        Submission::Confirmed(OnConfirm::CloseSession(session)) => {
            close(state);
            sessions::close(state, session, fx);
            return;
        }
        Submission::Confirmed(OnConfirm::DeleteConnection {
            id,
            forget_secret,
            delete_history,
        }) => {
            close(state);
            connections::delete(state, &id, forget_secret, delete_history, fx);
            return;
        }
        Submission::Confirmed(OnConfirm::RunOnProd(session)) => {
            run::release(state, session, fx);
            close(state);
            return;
        }
        Submission::Confirmed(OnConfirm::OverwriteConnections { path, ids }) => {
            close(state);
            connections::export(state, path, &ids, true, fx);
            return;
        }
        Submission::Dismissed => {
            close(state);
            return;
        }
    };
    state.awaiting = Some(awaiting);
}

/// How often the pending spinner moves on.
pub(crate) const SPINNER_EVERY: Duration = Duration::from_millis(100);

/// Starts the pending footer's spinner from its first frame.
fn start_spinner(state: &mut State, fx: &mut Vec<Effect>) {
    state.spinner = 0;
    state.spinner_timer += 1;
    fx.push(Effect::ScheduleTimer {
        timer: Timer::Spinner(state.spinner_timer),
        after: SPINNER_EVERY,
    });
}

/// The spinner's timer ran out: the next frame, and the next timer, for as
/// long as the open modal still waits. An older timer changes nothing.
pub(super) fn spin(state: &mut State, timer: u64, fx: &mut Vec<Effect>) {
    let waiting = state
        .dialog()
        .is_some_and(|dialog| dialog.form().pending().is_some());
    if timer != state.spinner_timer || !waiting {
        return;
    }
    state.spinner += 1;
    fx.push(Effect::ScheduleTimer {
        timer: Timer::Spinner(timer),
        after: SPINNER_EVERY,
    });
}

/// `y` and `n` answer a confirm dialog: `y` does what it asks, `n` cancels.
/// Returns whether the key was one of them.
pub(super) fn confirm_shortcut(state: &mut State, c: char, fx: &mut Vec<Effect>) -> bool {
    let Some(DialogState::Confirm(confirm)) = state.dialog_mut() else {
        return false;
    };
    match c.to_ascii_lowercase() {
        'y' => {
            confirm.select(true);
            submit(state, fx);
            true
        }
        'n' => {
            close(state);
            true
        }
        _ => false,
    }
}

/// The answer the modal waited for came: it is done, so close it.
pub(super) fn done(state: &mut State, answer: Awaiting) {
    if state.awaiting == Some(answer) {
        close(state);
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
