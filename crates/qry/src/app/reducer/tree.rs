//! The schema browser: the stored connections, their keys, what loads
//! when, and what the workers' answers mean for it. The rows themselves are
//! worked out in [`schematree`].

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use qry_core::{
    schema::{Children, SchemaPath},
    sql::{Dialect, qualified_name, quote_ident},
};
use ratatui_textarea::CursorMove;

use super::{connections, sessions};
use crate::{
    action::Action,
    app::{
        dialogs::textinput::typed_char,
        effects::Effect,
        event::Timer,
        schematree::{self, Folder, Section, TreeNode},
        state::{Node, Queued, SessionId, SessionState, SessionStatus, State, TreeFilter},
    },
    connections::{Secret, StoredConnection},
    db::DbCommand,
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
    // Edited: worth writing once it opens, and it may point elsewhere now.
    tree.connections[i].unsaved = !record.ephemeral;
    tree.connections[i].opened = false;
    if matches!(previous.secret, Secret::Keyring) && !matches!(record.secret, Secret::Keyring) {
        // Its password is no longer kept, so it should not linger.
        fx.push(Effect::ForgetSecret(previous.id));
    }
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

pub(super) fn save(state: &State, fx: &mut Vec<Effect>) {
    fx.push(Effect::SaveConnections(to_save(state)));
}

/// The open session on connection `i`, if there is one.
fn open_session(state: &State, i: usize) -> Option<SessionId> {
    schematree::schema_session(state, i).map(|s| s.id)
}

/// How long a spinner frame lasts.
pub(crate) const SPINNER_FRAME: Duration = Duration::from_millis(100);

/// What a `Tree` binding does.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    // What needs no row.
    match action {
        Action::Filter => {
            let before = state.tree.selected_key.clone();
            let text = state.tree.filter.take().map(|f| f.text).unwrap_or_default();
            state.tree.filter = Some(TreeFilter {
                text,
                editing: true,
                before,
                jump: false,
            });
            return;
        }
        Action::ToggleSystemObjects => {
            state.tree.show_system = !state.tree.show_system;
            return;
        }
        // Esc: the filter goes first, else the toast, as everywhere else.
        Action::Cancel => {
            if state.tree.filter.is_some() {
                clear_filter(state);
            } else {
                super::notices::dismiss(state, fx);
            }
            return;
        }
        _ => {}
    }
    let Some(row) = state.tree.selected_row() else {
        return;
    };
    let tree = &mut state.tree;
    let len = tree.rows.len();
    tree.selected = tree.selected.min(len - 1);
    let at = tree.selected;
    let page = tree.page.get().max(1);
    let node = row.node;
    let conn = node.conn();
    let open = schematree::is_open(state, node);
    match action {
        Action::Up => select(state, at.saturating_sub(1)),
        Action::Down => select(state, (at + 1).min(len - 1)),
        Action::PageUp => select(state, at.saturating_sub(page)),
        Action::PageDown => select(state, (at + page).min(len - 1)),
        Action::Top => select(state, 0),
        Action::Bottom => select(state, len - 1),
        // Right opens a closed node, or goes into an open one.
        Action::Right if node.expandable() && !open => expand(state, node, fx),
        Action::Right => {
            let child = state
                .tree
                .rows
                .get(at + 1)
                .is_some_and(|r| r.depth > row.depth);
            if child {
                select(state, at + 1);
            }
        }
        // Left closes an open node, or goes up to the one it is under.
        Action::Left if node.expandable() && open => collapse(state, node),
        Action::Left => {
            let parent = state.tree.rows[..at]
                .iter()
                .rposition(|r| r.depth < row.depth);
            if let Some(parent) = parent {
                select(state, parent);
            }
        }
        Action::Toggle if node.expandable() => {
            if open {
                collapse(state, node);
            } else {
                expand(state, node, fx);
            }
        }
        // A connection with a session switches to it; one without opens
        // one. A table runs its rows; anything else opens or closes.
        Action::Open => match node {
            TreeNode::Connection(i) => {
                let record = state.tree.connections[i].record.clone();
                sessions::open_or_switch(state, &record, fx);
            }
            TreeNode::Object { .. } => browse(state, node, true, fx),
            node if node.expandable() && open => collapse(state, node),
            node if node.expandable() => expand(state, node, fx),
            _ => {}
        },
        Action::InsertSelect => browse(state, node, false, fx),
        Action::CopyName => {
            if let Some(name) = copy_name(state, node) {
                fx.push(Effect::CopyToClipboard(name.clone()));
                let notice = Notice::info(format!("Copied {name}"));
                super::notices::notify(state, notice, fx);
            }
        }
        Action::Refresh => refresh(state, node, fx),
        // Everything of the connection goes, and what is open loads again.
        Action::RefreshAll => {
            if let Some(id) = open_session(state, conn)
                && let Some(session) = state.sessions.get_mut(id)
            {
                session.schema.clear();
            }
        }
        // Another session on the same connection, such as for a second
        // transaction.
        Action::OpenNewSession => {
            let record = state.tree.connections[conn].record.clone();
            sessions::open(state, &record, fx);
        }
        Action::EditConnection => {
            let record = state.tree.connections[conn].record.clone();
            connections::edit(state, &record);
        }
        Action::DuplicateConnection => {
            let record = state.tree.connections[conn].record.clone();
            connections::duplicate(state, &record);
        }
        // Only the confirm's Delete forgets it; see `connections::delete`.
        Action::DeleteConnection => {
            let record = state.tree.connections[conn].record.clone();
            connections::ask_delete(state, &record);
        }
        Action::Disconnect => {
            let id = &state.tree.connections[conn].record.id;
            if let Some(session) = state.sessions.for_connection(id) {
                sessions::close_or_confirm(state, session, fx);
            }
        }
        _ => {}
    }
}

/// Moves the selection to row `i`.
fn select(state: &mut State, i: usize) {
    let Some(row) = state.tree.rows.get(i).copied() else {
        return;
    };
    state.tree.selected = i;
    state.tree.selected_key = Some(schematree::key_of(state, row.node));
}

/// The catalog and object names of an object row.
fn object_names(state: &State, node: TreeNode) -> Option<(String, String)> {
    let TreeNode::Object { conn, cat, obj } = node else {
        return None;
    };
    let object = schematree::object(state, conn, cat, obj)?;
    Some((object.catalog.clone(), object.name.clone()))
}

/// The session, catalog and name of the selected row's table or view, for
/// `d`: an object row, or a row under one (its columns and indexes).
pub(super) fn selected_object(state: &State) -> Option<(SessionId, String, String)> {
    let node = match state.tree.selected_row()?.node {
        TreeNode::Section { conn, cat, obj, .. }
        | TreeNode::Column { conn, cat, obj, .. }
        | TreeNode::Index { conn, cat, obj, .. } => TreeNode::Object { conn, cat, obj },
        node => node,
    };
    let (catalog, object) = object_names(state, node)?;
    Some((open_session(state, node.conn())?, catalog, object))
}

/// `SELECT * FROM` the table or view of an object row, run or not.
fn browse(state: &mut State, node: TreeNode, run: bool, fx: &mut Vec<Effect>) {
    let Some((catalog, object)) = object_names(state, node) else {
        return;
    };
    let Some(session) = open_session(state, node.conn()) else {
        return;
    };
    select_object(state, session, &catalog, &object, run, fx);
}

/// How `catalog.object` is written in `session`: quoted where it must be,
/// and without the catalog for SQLite's `main`, which every name means.
fn qualified(session: &SessionState, catalog: &str, object: &str) -> String {
    let dialect = session.dialect();
    let catalog = (dialect != Dialect::Sqlite || catalog != "main").then_some(catalog);
    qualified_name(dialect, catalog, object)
}

/// Appends `SELECT * FROM catalog.object;` to session `session`'s editor as
/// a statement of its own, shows that session, and runs it when `run` is
/// set: Enter on a table, and the palette's `Table: …` items.
pub(super) fn select_object(
    state: &mut State,
    session: SessionId,
    catalog: &str,
    object: &str,
    run: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(s) = state.sessions.get(session) else {
        return;
    };
    let statement = format!("SELECT * FROM {}", qualified(s, catalog, object));
    sessions::switch(state, session);

    let editor = state.editor_mut();
    let text = editor.query();
    let trimmed = text.trim_end();
    // The statement before it must end, and this one starts on a line of
    // its own.
    let lead = match () {
        _ if trimmed.is_empty() => "",
        _ if !trimmed.ends_with(';') => ";\n",
        _ if text.ends_with('\n') => "",
        _ => "\n",
    };
    let t = &mut editor.textarea;
    t.cancel_selection();
    t.move_cursor(CursorMove::Bottom);
    t.move_cursor(CursorMove::End);
    let start = text.len() + lead.len();
    t.insert_str(format!("{lead}{statement};"));
    if run {
        let span = start..start + statement.len();
        let queued = Queued::new(statement, Some(span));
        super::run::enqueue(state, vec![queued], fx);
    }
}

/// The name `y` copies: a table's or view's qualified and quoted, a
/// catalog's or column's own.
fn copy_name(state: &State, node: TreeNode) -> Option<String> {
    let session = schematree::schema_session(state, node.conn())?;
    let dialect = session.dialect();
    match node {
        TreeNode::Object { .. } => {
            let (catalog, object) = object_names(state, node)?;
            Some(qualified(session, &catalog, &object))
        }
        TreeNode::Catalog { .. } | TreeNode::Column { .. } | TreeNode::Index { .. } => {
            Some(quote_ident(dialect, schematree::name(state, node)?))
        }
        _ => None,
    }
}

/// Reads again what the row stands for: its own children, or those of the
/// node above it that loads. What was there stays until the answer comes,
/// and stays if it fails.
fn refresh(state: &mut State, node: TreeNode, fx: &mut Vec<Effect>) {
    let conn = node.conn();
    let Some(session) = open_session(state, conn) else {
        return;
    };
    let Some(path) = schematree::load_path(state, conn, schematree::load_of(node)) else {
        return;
    };
    load(state, session, path, fx);
}

/// Asks session `session` for `path`.
pub(super) fn load(state: &mut State, session: SessionId, path: SchemaPath, fx: &mut Vec<Effect>) {
    let Some(s) = state.sessions.get_mut(session) else {
        return;
    };
    if s.schema.is_loading(&path) {
        return;
    }
    let generation = s.schema.begin(&path);
    fx.push(Effect::Db(
        session,
        DbCommand::LoadSchema { path, generation },
    ));
}

/// Opens `node`. Whatever it needs loads once the event is handled
/// ([`after`]). A connection that is not open is connected first, and
/// opens once it is.
fn expand(state: &mut State, node: TreeNode, fx: &mut Vec<Effect>) {
    set_open(state, node, true);
    if let TreeNode::Connection(i) = node
        && open_session(state, i).is_none()
    {
        state.tree.connections[i].expanded = false;
        connect_to_expand(state, i, fx);
    }
}

fn collapse(state: &mut State, node: TreeNode) {
    set_open(state, node, false);
}

fn set_open(state: &mut State, node: TreeNode, open: bool) {
    let names = match node {
        TreeNode::Connection(i) => {
            state.tree.connections[i].expanded = open;
            return;
        }
        TreeNode::Catalog { conn, cat } | TreeNode::Folder { conn, cat, .. } => {
            let Some(c) = schematree::catalog(state, conn, cat) else {
                return;
            };
            (conn, c.name.clone(), None)
        }
        TreeNode::Object { conn, cat, obj } | TreeNode::Section { conn, cat, obj, .. } => {
            let Some(o) = schematree::object(state, conn, cat, obj) else {
                return;
            };
            (conn, o.catalog.clone(), Some(o.name.clone()))
        }
        _ => return,
    };
    let (conn, catalog, object) = names;
    let view = state.tree.connections[conn]
        .catalogs
        .entry(catalog)
        .or_default();
    match (node, object) {
        (TreeNode::Catalog { .. }, _) => view.open = open,
        (TreeNode::Folder { folder, .. }, _) => match folder {
            Folder::Tables => view.tables = open,
            Folder::Views => view.views = open,
        },
        (TreeNode::Object { .. }, Some(object)) => {
            view.objects.entry(object).or_default().open = open
        }
        (TreeNode::Section { section, .. }, Some(object)) => {
            let object = view.objects.entry(object).or_default();
            match section {
                Section::Columns => object.columns = open,
                Section::Indexes => object.indexes = open,
            }
        }
        _ => {}
    }
}

/// Connection `i` was expanded without an open session: one is opened (or
/// the one connecting is waited for), and the connection opens once it is
/// connected.
fn connect_to_expand(state: &mut State, i: usize, fx: &mut Vec<Effect>) {
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

/// The filter is gone, and the selection goes back to where it was before
/// it opened.
fn clear_filter(state: &mut State) {
    if let Some(filter) = state.tree.filter.take()
        && filter.before.is_some()
    {
        state.tree.selected_key = filter.before;
    }
}

/// Takes a key for the filter prompt while it is open: characters type,
/// Backspace deletes, Enter keeps the filter, Esc clears it. Returns whether
/// the key was taken.
pub(super) fn filter_key(state: &mut State, key: KeyEvent) -> bool {
    let Some(filter) = state.tree.filter.as_mut().filter(|f| f.editing) else {
        return false;
    };
    match key.code {
        KeyCode::Enter => filter.editing = false,
        KeyCode::Esc => clear_filter(state),
        KeyCode::Backspace => {
            filter.text.pop();
        }
        _ => match typed_char(key) {
            Some(c) => filter.text.push(c),
            None => return false,
        },
    }
    // The first match is where the keys go.
    if let Some(filter) = state.tree.filter.as_mut() {
        filter.jump = true;
    }
    true
}

/// What every event that can change the tree ends with: what is open and
/// not loaded is asked for, the rows are worked out again with the
/// selection on the same node, and the spinner runs while anything loads.
pub(super) fn after(state: &mut State, fx: &mut Vec<Effect>) {
    ensure_loaded(state, fx);
    let (rows, partial) = schematree::flatten(state);
    let hint = state.tree.selected;
    let found = state
        .tree
        .selected_key
        .as_ref()
        .and_then(|key| schematree::find(state, &rows, key, hint));
    let mut selected = found.unwrap_or_else(|| hint.min(rows.len().saturating_sub(1)));
    if let Some(filter) = state.tree.filter.as_ref().filter(|f| f.jump) {
        let needle = filter.text.to_lowercase();
        let first = rows.iter().position(|row| {
            schematree::name(state, row.node).is_some_and(|n| n.to_lowercase().contains(&needle))
        });
        selected = first.unwrap_or(0);
    }
    let key = rows
        .get(selected)
        .map(|row| schematree::key_of(state, row.node));
    let tree = &mut state.tree;
    tree.rows = rows;
    tree.selected = selected;
    tree.selected_key = key;
    tree.filter_partial = partial;
    if let Some(filter) = tree.filter.as_mut() {
        filter.jump = false;
    }
    start_spinner(state, fx);
}

/// Asks for every open node that was never loaded. One that failed waits
/// for `r`.
fn ensure_loaded(state: &mut State, fx: &mut Vec<Effect>) {
    let mut wanted = Vec::new();
    for (i, node) in state.tree.connections.iter().enumerate() {
        if !node.expanded {
            continue;
        }
        let Some(session) = schematree::schema_session(state, i) else {
            continue;
        };
        let cache = &session.schema;
        let Some(catalogs) = cache.catalogs() else {
            if !cache.is_known(&SchemaPath::Catalogs) {
                wanted.push((session.id, SchemaPath::Catalogs));
            }
            continue;
        };
        for catalog in catalogs {
            let Some(view) = node.catalogs.get(&catalog.name) else {
                continue;
            };
            if !view.open || (catalog.is_system && !state.tree.show_system) {
                continue;
            }
            let path = SchemaPath::objects(&catalog.name);
            let Some(objects) = cache.objects(&catalog.name) else {
                if !cache.is_known(&path) {
                    wanted.push((session.id, path));
                }
                continue;
            };
            for object in objects {
                let shown = if object.kind.is_view() {
                    view.views
                } else {
                    view.tables
                };
                let open = view.objects.get(&object.name).is_some_and(|v| v.open);
                let path = SchemaPath::details(&object.catalog, &object.name);
                if shown && open && !cache.is_known(&path) {
                    wanted.push((session.id, path));
                }
            }
        }
    }
    for (session, path) in wanted.drain(..) {
        load(state, session, path, fx);
    }
    // What the inspector describes loads like an open node: again after a
    // reconnect or DDL cleared it. It waits for the catalogs or objects
    // loading above it (asked for just now, perhaps), whose answer would
    // drop it.
    for session in state.sessions.iter() {
        if let Some(describe) = &session.describe {
            let cache = &session.schema;
            let path = SchemaPath::details(&describe.catalog, &describe.object);
            let above = cache.is_loading(&SchemaPath::Catalogs)
                || cache.is_loading(&SchemaPath::objects(&describe.catalog));
            if session.is_open() && !above && !cache.is_known(&path) {
                wanted.push((session.id, path));
            }
        }
    }
    for (session, path) in wanted {
        load(state, session, path, fx);
    }
}

/// Whether something in the tree is waiting: a connection connecting, or a
/// load in flight.
fn waiting(state: &State) -> bool {
    let connecting = state.tree.connections.iter().any(|node| {
        state.sessions.connection_status(&node.record.id) == Some(&SessionStatus::Connecting)
    });
    connecting || state.sessions.iter().any(|s| s.schema.any_loading())
}

fn start_spinner(state: &mut State, fx: &mut Vec<Effect>) {
    if !state.tree.spinning && waiting(state) {
        state.tree.spinning = true;
        schedule_spinner(state, fx);
    }
}

fn schedule_spinner(state: &mut State, fx: &mut Vec<Effect>) {
    state.tree.spinner_timer += 1;
    fx.push(Effect::ScheduleTimer {
        timer: Timer::Tree(state.tree.spinner_timer),
        after: SPINNER_FRAME,
    });
}

/// The spinner's frame ran out: the next one, while something still
/// waits. A stale timer changes nothing.
pub(super) fn spin(state: &mut State, timer: u64, fx: &mut Vec<Effect>) {
    if timer != state.tree.spinner_timer || !state.tree.spinning {
        return;
    }
    if waiting(state) {
        state.tree.spinner = state.tree.spinner.wrapping_add(1);
        schedule_spinner(state, fx);
    } else {
        state.tree.spinning = false;
    }
}

/// A schema load of session `id` answered. One nobody waits for is
/// ignored. The first catalogs a connection gets open its own catalog and
/// that catalog's tables.
pub(super) fn schema_loaded(
    state: &mut State,
    id: SessionId,
    path: SchemaPath,
    generation: u64,
    outcome: Result<Children, String>,
) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    if !session.schema.finish(&path, generation, outcome) || path != SchemaPath::Catalogs {
        return;
    }
    let own = session.schema.catalogs().and_then(|catalogs| {
        catalogs
            .iter()
            .find(|c| c.is_default)
            .or_else(|| catalogs.iter().find(|c| !c.is_system))
            .map(|c| c.name.clone())
    });
    let Some(i) = connection_of(state, id) else {
        return;
    };
    let node = &mut state.tree.connections[i];
    if node.opened {
        return;
    }
    node.opened = true;
    if let Some(own) = own {
        let view = node.catalogs.entry(own).or_default();
        view.open = true;
        view.tables = true;
    }
}

/// Drops a connection. Its sessions stay open; what they answer about it
/// finds no connection and changes nothing.
pub(super) fn forget_node(state: &mut State, connection: usize) {
    let tree = &mut state.tree;
    tree.connections.remove(connection);
    // Rows name connections by index, so none may outlive this.
    tree.rows.clear();
    tree.selected_key = None;
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
    // What it held before is stale, or was never read; what is open
    // loads again once this event is handled.
    if let Some(session) = state.sessions.get_mut(id) {
        session.schema.clear();
    }
    let tree = &mut state.tree;
    tree.connections[i].label = Some(label);
    if expand {
        tree.connections[i].expanded = true;
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

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tree_tests;
