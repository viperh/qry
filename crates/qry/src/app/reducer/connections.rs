//! The connection dialogs' side of the reducer: opening the connection
//! dialog for a new, edited or duplicated connection, testing what it
//! holds, the import preview, exporting a chosen set, and deleting.

use super::{notices::notify, sessions, tree};
use crate::{
    app::{
        dialogs::{
            DialogState,
            confirm::Confirm,
            conndialog::{ConnDialog, TestState},
            connexportform::ConnExportForm,
            deleteconfirm,
            form::Form,
            importdialog::{EntryKind, ImportPreview},
        },
        effects::{Effect, test_stash_key},
        event::Tested,
        state::{Awaiting, Node, State},
    },
    connections::{Secret, StoredConnection},
    import::Imported,
    notice::Notice,
};

/// The names of the saved connections other than `id`, which a new name
/// must not repeat.
fn taken(state: &State, id: &str) -> Vec<String> {
    state
        .tree
        .connections
        .iter()
        .filter(|n| n.record.id != id && !n.record.ephemeral)
        .map(|n| n.record.name.clone())
        .collect()
}

fn show(state: &mut State, mut dialog: ConnDialog) {
    dialog.set_taken(taken(state, dialog.id()));
    state.open_dialog(DialogState::Connection(Box::new(dialog)));
    state.awaiting = None;
}

/// The dialog for a new connection, with a freshly minted id.
pub(super) fn new_connection(state: &mut State) {
    let dialog = ConnDialog::new(state.ids.connection_id());
    show(state, dialog);
}

/// The dialog on an existing connection.
pub(super) fn edit(state: &mut State, record: &StoredConnection) {
    show(state, ConnDialog::editing(record));
}

/// The dialog on a copy of `record`: a new id, `name (copy)`, and no
/// password.
pub(super) fn duplicate(state: &mut State, record: &StoredConnection) {
    let dialog = ConnDialog::duplicate(record, state.ids.connection_id());
    show(state, dialog);
}

/// Tests what the open connection dialog holds on a throw-away connection.
/// What is wrong with the fields is shown first, and nothing is tried.
pub(super) fn test(state: &mut State, fx: &mut Vec<Effect>) {
    let next = state.tests_made + 1;
    let Some(DialogState::Connection(dialog)) = state.dialog_mut() else {
        return;
    };
    if dialog.pending().is_some() {
        return;
    }
    if !dialog.check_for_test() {
        return;
    }
    dialog.set_error(None);
    let (record, password) = dialog.build();
    let needs_typing = record.secret == Secret::Prompt && password.is_none();
    if needs_typing {
        dialog.test = TestState::Failed("type the password to test the connection".into());
        return;
    }
    dialog.test = TestState::Running(next);
    state.tests_made = next;
    if let Some(password) = password {
        fx.push(Effect::StashPassword {
            connection: test_stash_key(&record.id),
            password,
        });
    }
    fx.push(Effect::TestConnection {
        test: next,
        record: Box::new(record),
    });
}

/// Test number `test` came back. Only the open dialog's latest test takes
/// it; one cancelled, closed or overtaken is ignored.
pub(super) fn tested(state: &mut State, test: u64, outcome: Result<Tested, String>) {
    let Some(DialogState::Connection(dialog)) = state.dialog_mut() else {
        return;
    };
    if dialog.test != TestState::Running(test) {
        return;
    }
    dialog.test = match outcome {
        Ok(Tested { millis, version }) => TestState::Passed { millis, version },
        Err(message) => TestState::Failed(message),
    };
}

/// The file was read: show what it held before adding anything.
pub(super) fn imported(state: &mut State, imported: Imported) {
    let known: Vec<StoredConnection> = state
        .tree
        .connections
        .iter()
        .map(|n| n.record.clone())
        .collect();
    let preview = ImportPreview::new(imported, &known);
    state.open_dialog(DialogState::ImportPreview(preview));
    state.awaiting = None;
}

/// `a` / `A` in the preview select every new connection, or none.
pub(super) fn preview_shortcut(state: &mut State, c: char) -> bool {
    let Some(DialogState::ImportPreview(preview)) = state.dialog_mut() else {
        return false;
    };
    match c {
        'a' => preview.select_all(true),
        'A' => preview.select_all(false),
        _ => return false,
    }
    true
}

/// The preview's Enter: adds what was selected, saves once, and says what
/// happened in a toast.
pub(super) fn import_selected(
    state: &mut State,
    records: Vec<StoredConnection>,
    fx: &mut Vec<Effect>,
) {
    let Some(DialogState::ImportPreview(preview)) = state.dialog() else {
        return;
    };
    let here = preview.count(|k| *k == EntryKind::Existing);
    let skipped = preview.count(|k| matches!(k, EntryKind::Skipped(_)));
    state.close_dialog();
    let mut added = 0;
    for record in records {
        let known = state
            .tree
            .connections
            .iter()
            .any(|n| n.record.id == record.id || n.record.same_target(&record));
        if !known {
            state.tree.connections.push(Node::new(record, false));
            added += 1;
        }
    }
    if added > 0 {
        tree::save(state, fx);
    }
    let text = format!("imported {added} · {here} already here · {skipped} skipped");
    let notice = if skipped > 0 {
        Notice::warning(text)
    } else {
        Notice::success(text)
    };
    notify(state, notice, fx);
}

/// The export dialog, offering every saved connection.
pub(super) fn open_export(state: &mut State) {
    let offered: Vec<(String, String)> = tree::to_save(state)
        .iter()
        .map(|r| (r.id.clone(), r.title().to_string()))
        .collect();
    state.open_dialog(DialogState::ConnExport(ConnExportForm::new(&offered)));
    state.awaiting = None;
}

/// Writes the chosen connections to `path`; an existing file is only
/// replaced once confirmed.
pub(super) fn export(
    state: &mut State,
    path: String,
    ids: &[String],
    overwrite: bool,
    fx: &mut Vec<Effect>,
) {
    let connections: Vec<StoredConnection> = tree::to_save(state)
        .into_iter()
        .filter(|r| ids.contains(&r.id))
        .collect();
    if connections.is_empty() {
        super::dialogs::failed(
            state,
            Awaiting::ExportConnections,
            "There are no saved connections to export",
        );
        return;
    }
    fx.push(Effect::ExportConnections {
        path,
        connections,
        overwrite,
    });
}

/// The backup's file is already there: ask before replacing it.
pub(super) fn export_exists(state: &mut State, shown: String) {
    if state.awaiting != Some(Awaiting::ExportConnections) {
        return;
    }
    let Some(DialogState::ConnExport(form)) = state.dialog() else {
        return;
    };
    let path = form.text(0).trim().to_string();
    let ids = form.included();
    let _ = shown;
    state.open_dialog(DialogState::Confirm(Confirm::overwrite_connections(
        path, ids,
    )));
    state.awaiting = None;
}

/// Asks before deleting the selected connection.
pub(super) fn ask_delete(state: &mut State, record: &StoredConnection) {
    let confirm = deleteconfirm::delete_connection(record);
    state.open_dialog(DialogState::Confirm(confirm));
    state.awaiting = None;
}

/// Deletes the stored connection `id` as its confirm says: any session
/// open on it is closed first, then the record goes, with its keychain
/// password and its history if those were ticked, and the file is written.
pub(super) fn delete(
    state: &mut State,
    id: &str,
    forget_secret: bool,
    delete_history: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(i) = state.tree.position(id) else {
        return;
    };
    let open: Vec<_> = state
        .sessions
        .iter()
        .filter(|s| s.connection_id == id)
        .map(|s| s.id)
        .collect();
    for session in open {
        sessions::close(state, session, fx);
    }
    let record = state.tree.connections[i].record.clone();
    tree::forget_node(state, i);
    if forget_secret && record.secret == Secret::Keyring {
        fx.push(Effect::ForgetSecret(record.id.clone()));
    }
    if delete_history {
        fx.push(Effect::DeleteHistory(record.id.clone()));
    }
    if !record.ephemeral {
        tree::save(state, fx);
    }
}

#[cfg(test)]
#[path = "connections_tests.rs"]
mod connections_tests;
