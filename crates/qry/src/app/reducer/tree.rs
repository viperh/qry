//! The connection tree: what it lists, its keys, and what the worker's
//! answers mean for it.

use crossterm::event::KeyEvent;

use super::{connect, dialogs};
use crate::{
    app::{
        effects::Effect,
        state::{Awaiting, Node, Row, State, StatusCode},
    },
    connections::{Secret, StoredConnection},
    db::DbCommand,
    import::Imported,
    keymap::TreeCommand,
};

pub(super) fn key(state: &mut State, key: KeyEvent, fx: &mut Vec<Effect>) {
    if let Some(command) = state.config.panes.tree.get(key) {
        run(state, command, fx);
    }
}

/// Remembers a connection the app is about to open, adding it to the tree if
/// its id is new and taking on its fields if it was edited. The entry stays
/// once it has connected.
pub(super) fn attempt(state: &mut State, record: &StoredConnection, fx: &mut Vec<Effect>) {
    let tree = &mut state.tree;
    let known = tree
        .connections
        .iter()
        .position(|n| n.record.id == record.id);
    let Some(i) = known else {
        tree.connections
            .push(Node::new(record.clone(), !record.ephemeral));
        tree.attempting = Some(tree.connections.len() - 1);
        return;
    };

    tree.attempting = Some(i);
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
    state.status = if imported.skipped.is_empty() {
        StatusCode::Success(message)
    } else {
        StatusCode::Error(message)
    };
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

/// Asks the worker for the tables of whatever is connected.
fn load_tables(state: &mut State, connection: usize, fx: &mut Vec<Effect>) {
    state.tree.connections[connection].loading = true;
    fx.push(Effect::Db(state.session, DbCommand::ListTables));
}

fn run(state: &mut State, command: TreeCommand, fx: &mut Vec<Effect>) {
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
    match command {
        TreeCommand::Up => tree.selected = tree.selected.saturating_sub(1),
        TreeCommand::Down => tree.selected = (tree.selected + 1).min(rows.len() - 1),
        TreeCommand::First => tree.selected = 0,
        TreeCommand::Last => tree.selected = rows.len() - 1,
        TreeCommand::Toggle => {
            if let Some(i) = selected {
                let node = &mut tree.connections[i];
                node.expanded = !node.expanded;
                let needs_tables = node.expanded && node.tables.is_none() && !node.loading;
                if needs_tables {
                    if tree.active == Some(i) {
                        load_tables(state, i, fx);
                    } else {
                        // Its tables can only be read once it is open.
                        tree.expand_when_connected = Some(i);
                        let record = tree.connections[i].record.clone();
                        connect(state, record, fx);
                    }
                }
            }
        }
        TreeCommand::Connect => {
            if let Some(i) = selected {
                let record = tree.connections[i].record.clone();
                connect(state, record, fx);
            }
        }
        TreeCommand::Edit => {
            if let Some(i) = selected {
                let record = tree.connections[i].record.clone();
                dialogs::edit_connection(state, &record);
            }
        }
        TreeCommand::Delete => {
            if let Some(i) = selected {
                let record = &tree.connections[i].record;
                // Its password goes with it, so nothing is orphaned.
                let forget = matches!(record.secret, Secret::Keyring)
                    .then(|| Effect::ForgetSecret(record.id.clone()));
                let saved = !record.ephemeral;
                forget_node(state, i);
                fx.extend(forget);
                if saved {
                    save(state, fx);
                }
            }
        }
    }
}

/// Drops a connection, keeping the other indexes right.
fn forget_node(state: &mut State, connection: usize) {
    let tree = &mut state.tree;
    tree.connections.remove(connection);
    let shift = |index: &mut Option<usize>| {
        if let Some(i) = index
            && *i > connection
        {
            *index = Some(*i - 1);
        }
    };
    shift(&mut tree.active);
    shift(&mut tree.expand_when_connected);
    tree.selected = tree.selected.min(tree.rows().len().saturating_sub(1));
}

pub(super) fn connected(state: &mut State, label: String, fx: &mut Vec<Effect>) {
    let Some(i) = state.tree.attempting.take() else {
        return;
    };
    let tree = &mut state.tree;
    tree.connections[i].label = Some(label);
    tree.active = Some(i);
    // Its tables are stale, or were never read.
    tree.connections[i].tables = None;
    if tree.expand_when_connected.take() == Some(i) {
        tree.connections[i].expanded = true;
    }
    if tree.connections[i].expanded {
        load_tables(state, i, fx);
    }
    // It works, so it is worth keeping.
    if state.tree.connections[i].unsaved {
        state.tree.connections[i].unsaved = false;
        save(state, fx);
    }
}

pub(super) fn connect_failed(state: &mut State) {
    if let Some(i) = state.tree.attempting.take()
        // Only one added this session and never written: a saved
        // connection stays, whatever the database says today.
        && state.tree.connections[i].unsaved
    {
        forget_node(state, i);
    }
    state.tree.expand_when_connected = None;
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

pub(super) fn tables_loaded(state: &mut State, tables: Vec<String>) {
    if let Some(i) = state.tree.active {
        state.tree.connections[i].tables = Some(tables);
        state.tree.connections[i].loading = false;
    }
}

pub(super) fn tables_failed(state: &mut State) {
    if let Some(i) = state.tree.active {
        // Left unread, so Space can try again; the status bar says why.
        state.tree.connections[i].loading = false;
        state.tree.connections[i].expanded = false;
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tree_tests;
