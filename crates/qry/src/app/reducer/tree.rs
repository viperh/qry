//! The connection tree: what it lists, its keys, and what the worker's
//! answers mean for it.

use super::{dialogs, sessions};
use crate::{
    action::Action,
    app::{
        dialogs::{DialogState, confirm::Confirm},
        effects::Effect,
        state::{Awaiting, Node, Row, SessionId, SessionStatus, State},
    },
    connections::{Secret, StoredConnection},
    db::DbCommand,
    import::Imported,
    notice::Notice,
};

/// Remembers a connection the app is about to open, adding it to the tree if
/// its id is new and taking on its fields if it was edited. The entry stays
/// once it has connected.
pub(super) fn attempt(state: &mut State, record: &StoredConnection, fx: &mut Vec<Effect>) {
    let tree = &mut state.tree;
    let Some(i) = tree.position(&record.id) else {
        tree.connections
            .push(Node::new(record.clone(), !record.ephemeral));
        return;
    };

    let previous = std::mem::replace(&mut tree.connections[i].record, record.clone());
    if previous == *record {
        return;
    }
    // Edited: worth writing once it opens, and its tables may differ.
    tree.connections[i].unsaved = !record.ephemeral;
    tree.connections[i].tables = None;
    if matches!(previous.secret, Secret::Keyring) && !matches!(record.secret, Secret::Keyring) {
        // Its password is no longer kept, so it should not linger.
        fx.push(Effect::ForgetSecret(previous.id));
    }
}

/// Adds what an import found that the tree does not have, saves once, and
/// says in the status bar what happened to the rest.
pub(super) fn import(state: &mut State, imported: Imported, fx: &mut Vec<Effect>) {
    let found = imported.connections.len();
    let mut added = 0;
    for record in imported.connections {
        let known = state
            .tree
            .connections
            .iter()
            .any(|n| n.record.id == record.id || n.record.same_target(&record));
        if known {
            continue;
        }
        state.tree.connections.push(Node::new(record, false));
        added += 1;
    }
    if added > 0 {
        save(state, fx);
    }

    let mut parts = vec![format!(
        "Imported {added} connection{} from {}",
        if added == 1 { "" } else { "s" },
        imported.format
    )];
    if found > added {
        parts.push(format!("{} already here", found - added));
    }
    for skipped in &imported.skipped {
        parts.push(format!("skipped {}: {}", skipped.name, skipped.reason));
    }
    parts.extend(imported.warnings);
    let message = parts.join("; ");
    let notice = if imported.skipped.is_empty() {
        Notice::success(message)
    } else {
        Notice::warning(message)
    };
    super::notices::notify(state, notice, fx);
}

/// The records worth writing: everything but the scratch database.
pub(super) fn to_save(state: &State) -> Vec<StoredConnection> {
    state
        .tree
        .connections
        .iter()
        .filter(|node| !node.record.ephemeral)
        .map(|node| node.record.clone())
        .collect()
}

fn save(state: &State, fx: &mut Vec<Effect>) {
    fx.push(Effect::SaveConnections(to_save(state)));
}

/// Asks session `session`, which is open on it, for a connection's tables.
fn load_tables(state: &mut State, connection: usize, session: SessionId, fx: &mut Vec<Effect>) {
    state.tree.connections[connection].loading = true;
    fx.push(Effect::Db(session, DbCommand::ListTables));
}

/// The open session on connection `i`, if there is one.
fn open_session(state: &State, i: usize) -> Option<SessionId> {
    let connection = &state.tree.connections[i].record.id;
    let id = state.sessions.for_connection(connection)?;
    let open = state.sessions.get(id).is_some_and(|s| s.is_open());
    if open {
        return Some(id);
    }
    // Another session on it may be open while the preferred one is not.
    state
        .sessions
        .iter()
        .find(|s| s.connection_id == *connection && s.is_open())
        .map(|s| s.id)
}

/// What a `Tree` binding does. Filtering comes with the schema browser
/// (T301).
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let rows = state.tree.rows();
    if rows.is_empty() {
        return;
    }
    let tree = &mut state.tree;
    // The rows may have shrunk since the selection was made.
    tree.selected = tree.selected.min(rows.len() - 1);
    let selected = match rows.get(tree.selected) {
        Some(Row::Connection(i)) => Some(*i),
        _ => None,
    };
    // Right opens a closed connection, Left closes an open one or, on a
    // table, goes up to its connection.
    let action = match (action, rows.get(tree.selected)) {
        (Action::Right, Some(Row::Connection(i))) if !tree.connections[*i].expanded => {
            Action::Toggle
        }
        (Action::Left, Some(Row::Connection(i))) if tree.connections[*i].expanded => Action::Toggle,
        (Action::Left, Some(Row::Table(i, _))) => {
            tree.selected = rows
                .iter()
                .position(|row| *row == Row::Connection(*i))
                .unwrap_or(0);
            return;
        }
        (action, _) => action,
    };
    match action {
        Action::Up => tree.selected = tree.selected.saturating_sub(1),
        Action::Down => tree.selected = (tree.selected + 1).min(rows.len() - 1),
        Action::Top => tree.selected = 0,
        Action::Bottom => tree.selected = rows.len() - 1,
        Action::Toggle => {
            if let Some(i) = selected {
                let node = &mut tree.connections[i];
                node.expanded = !node.expanded;
                let needs_tables = node.expanded && node.tables.is_none() && !node.loading;
                if needs_tables {
                    expand(state, i, fx);
                }
            }
        }
        // A connection with a session switches to it; one without opens one.
        Action::Open => {
            if let Some(i) = selected {
                let record = tree.connections[i].record.clone();
                sessions::open_or_switch(state, &record, fx);
            }
        }
        // Another session on the same connection, such as for a second
        // transaction.
        Action::OpenNewSession => {
            if let Some(i) = selected {
                let record = tree.connections[i].record.clone();
                sessions::open(state, &record, fx);
            }
        }
        Action::EditConnection => {
            if let Some(i) = selected {
                let record = tree.connections[i].record.clone();
                dialogs::edit_connection(state, &record);
            }
        }
        // Only the confirm's Delete forgets it; see `delete`.
        Action::DeleteConnection => {
            if let Some(i) = selected {
                let record = &tree.connections[i].record;
                let keychain = matches!(record.secret, Secret::Keyring);
                let confirm = Confirm::delete_connection(&record.id, record.title(), keychain);
                state.open_dialog(DialogState::Confirm(confirm));
                state.awaiting = None;
            }
        }
        // Reads the tables of the open connection again.
        Action::Refresh => {
            let open = match rows.get(tree.selected) {
                Some(Row::Connection(i) | Row::Table(i, _)) => Some(*i),
                _ => None,
            };
            let session = open.and_then(|i| Some((i, open_session(state, i)?)));
            if let Some((i, session)) = session {
                state.tree.connections[i].expanded = true;
                load_tables(state, i, session, fx);
            }
        }
        _ => {}
    }
}

/// Forgets the stored connection `id`, as its delete confirm asked: its
/// password goes with it, so nothing is orphaned, and the file is written.
pub(super) fn delete(state: &mut State, id: &str, fx: &mut Vec<Effect>) {
    let Some(i) = state.tree.position(id) else {
        return;
    };
    let record = &state.tree.connections[i].record;
    let forget =
        matches!(record.secret, Secret::Keyring).then(|| Effect::ForgetSecret(record.id.clone()));
    let saved = !record.ephemeral;
    forget_node(state, i);
    fx.extend(forget);
    if saved {
        save(state, fx);
    }
}

/// Reads the tables of connection `i`, which was just expanded. They can
/// only be read once it is open, so one that is not is opened first, and
/// expanded once it is.
fn expand(state: &mut State, i: usize, fx: &mut Vec<Effect>) {
    if let Some(session) = open_session(state, i) {
        load_tables(state, i, session, fx);
        return;
    }
    let record = state.tree.connections[i].record.clone();
    let session = match state.sessions.for_connection(&record.id) {
        Some(id)
            if state.sessions.get(id).map(|s| &s.status) == Some(&SessionStatus::Connecting) =>
        {
            id
        }
        _ => sessions::open_or_switch(state, &record, fx),
    };
    if let Some(session) = state.sessions.get_mut(session) {
        session.expand_when_connected = true;
    }
}

/// Drops a connection. Its sessions stay open; what they answer about it
/// finds no connection and changes nothing.
fn forget_node(state: &mut State, connection: usize) {
    let tree = &mut state.tree;
    tree.connections.remove(connection);
    tree.selected = tree.selected.min(tree.rows().len().saturating_sub(1));
}

/// Session `id` is open as `label`.
pub(super) fn connected(state: &mut State, id: SessionId, label: String, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    let expand = std::mem::take(&mut session.expand_when_connected);
    let Some(i) = state.tree.position(&session.connection_id) else {
        return;
    };
    let tree = &mut state.tree;
    tree.connections[i].label = Some(label);
    // Its tables are stale, or were never read.
    tree.connections[i].tables = None;
    tree.connections[i].loading = false;
    if expand {
        tree.connections[i].expanded = true;
    }
    if tree.connections[i].expanded {
        load_tables(state, i, id, fx);
    }
    // It works, so it is worth keeping.
    if state.tree.connections[i].unsaved {
        state.tree.connections[i].unsaved = false;
        save(state, fx);
    }
}

/// Session `id` could not connect. A connection added this session and
/// never written goes, and its session with it; a saved one stays, whatever
/// the database says today, with its session marked failed.
pub(super) fn connect_failed(state: &mut State, id: SessionId, fx: &mut Vec<Effect>) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    session.expand_when_connected = false;
    let Some(i) = state.tree.position(&session.connection_id) else {
        return;
    };
    if state.tree.connections[i].unsaved {
        forget_node(state, i);
        sessions::close(state, id, fx);
    }
}

/// Whatever is already here was opened during startup; the file fills in
/// the rest.
pub(super) fn connections_loaded(state: &mut State, records: Vec<StoredConnection>) {
    for record in records {
        if !state
            .tree
            .connections
            .iter()
            .any(|n| n.record.id == record.id)
        {
            state.tree.connections.push(Node::new(record, false));
        }
    }
}

/// The same list a save writes: everything but the scratch database.
pub(super) fn export_connections(state: &mut State, path: String, fx: &mut Vec<Effect>) {
    let connections = to_save(state);
    if connections.is_empty() {
        dialogs::failed(
            state,
            Awaiting::ExportConnections,
            "There are no saved connections to export",
        );
    } else {
        fx.push(Effect::ExportConnections { path, connections });
    }
}

pub(super) fn secret_not_stored(state: &mut State, id: &str, fx: &mut Vec<Effect>) {
    let node = state
        .tree
        .connections
        .iter_mut()
        .find(|n| n.record.id == id);
    if let Some(node) = node {
        // The keychain would not take it, so the record says so rather
        // than pretending the password is there.
        node.record.secret = Secret::Prompt;
        save(state, fx);
    }
}

/// The connection session `id` is on.
fn connection_of(state: &State, id: SessionId) -> Option<usize> {
    let session = state.sessions.get(id)?;
    state.tree.position(&session.connection_id)
}

pub(super) fn tables_loaded(state: &mut State, id: SessionId, tables: Vec<String>) {
    if let Some(i) = connection_of(state, id) {
        state.tree.connections[i].tables = Some(tables);
        state.tree.connections[i].loading = false;
    }
}

pub(super) fn tables_failed(state: &mut State, id: SessionId) {
    if let Some(i) = connection_of(state, id) {
        // Left unread, so Space can try again; the status bar says why.
        state.tree.connections[i].loading = false;
        state.tree.connections[i].expanded = false;
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tree_tests;
