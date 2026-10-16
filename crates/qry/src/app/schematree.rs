//! The schema browser's model: what is expanded, and the flat list of rows
//! the tree draws.
//!
//! The loaded schema lives in each session's
//! [`SchemaCache`]. The tree keeps only view
//! state (which nodes are open, the selection, the filter) and a flattened
//! [`VisibleRow`] list, worked out again after each event that can change
//! it ([`flatten`]) rather than on every frame. A row names its node by
//! indices into the tree's connections and the cache's lists, so building
//! the list copies no names, and the view draws only the rows in sight.

use std::collections::BTreeMap;

use qry_core::schema::{Catalog, DbObject, Details, Entry, SchemaCache, SchemaPath};

use super::state::{SessionState, State};

/// Which children of a catalog are open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CatalogView {
    pub(crate) open: bool,
    pub(crate) tables: bool,
    pub(crate) views: bool,
    /// By object name.
    pub(crate) objects: BTreeMap<String, ObjectView>,
}

/// Which children of a table or view are open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ObjectView {
    pub(crate) open: bool,
    pub(crate) columns: bool,
    pub(crate) indexes: bool,
}

/// A catalog's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Folder {
    Tables,
    Views,
}

impl Folder {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Folder::Tables => "Tables",
            Folder::Views => "Views",
        }
    }

    fn holds(self, object: &DbObject) -> bool {
        object.kind.is_view() == (self == Folder::Views)
    }
}

/// An object's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Section {
    Columns,
    Indexes,
}

impl Section {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Section::Columns => "Columns",
            Section::Indexes => "Indexes",
        }
    }
}

/// Where a load happens, by index: the connection's catalogs, a catalog's
/// objects, an object's details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LoadAt {
    Catalogs,
    Objects { cat: usize },
    Details { cat: usize, obj: usize },
}

/// What a row shows. `conn` indexes `TreeState::connections`; `cat`, `obj`,
/// `col` and `idx` index the session cache's lists of catalogs, objects,
/// columns and indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TreeNode {
    Connection(usize),
    Catalog {
        conn: usize,
        cat: usize,
    },
    Folder {
        conn: usize,
        cat: usize,
        folder: Folder,
    },
    Object {
        conn: usize,
        cat: usize,
        obj: usize,
    },
    Section {
        conn: usize,
        cat: usize,
        obj: usize,
        section: Section,
    },
    Column {
        conn: usize,
        cat: usize,
        obj: usize,
        col: usize,
    },
    Index {
        conn: usize,
        cat: usize,
        obj: usize,
        idx: usize,
    },
    /// The load at `at` failed; `r` on it tries again.
    Error {
        conn: usize,
        at: LoadAt,
    },
    /// A word under a connection, such as `(not connected)`.
    Note {
        conn: usize,
        text: &'static str,
    },
}

impl TreeNode {
    /// The connection the row is under.
    pub(crate) fn conn(self) -> usize {
        match self {
            TreeNode::Connection(conn)
            | TreeNode::Catalog { conn, .. }
            | TreeNode::Folder { conn, .. }
            | TreeNode::Object { conn, .. }
            | TreeNode::Section { conn, .. }
            | TreeNode::Column { conn, .. }
            | TreeNode::Index { conn, .. }
            | TreeNode::Error { conn, .. }
            | TreeNode::Note { conn, .. } => conn,
        }
    }

    /// Whether it has children it can show.
    pub(crate) fn expandable(self) -> bool {
        matches!(
            self,
            TreeNode::Connection(_)
                | TreeNode::Catalog { .. }
                | TreeNode::Folder { .. }
                | TreeNode::Object { .. }
                | TreeNode::Section { .. }
        )
    }
}

/// One drawn line of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibleRow {
    /// Indentation level: 0 for a connection.
    pub(crate) depth: u8,
    pub(crate) node: TreeNode,
}

/// A row by name rather than by index, so the selection stays on its node
/// when rows come and go above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowKey {
    kind: KeyKind,
    names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Connection,
    Catalog,
    Folder(Folder),
    Object,
    Section(Section),
    Column,
    Index,
    Error,
    Note,
}

impl RowKey {
    /// The key one level up, if there is one: an object's catalog, a
    /// column's object.
    fn parent(&self) -> Option<RowKey> {
        let (kind, keep) = match self.kind {
            KeyKind::Connection => return None,
            KeyKind::Column => (KeyKind::Section(Section::Columns), 3),
            KeyKind::Index => (KeyKind::Section(Section::Indexes), 3),
            KeyKind::Section(_) => (KeyKind::Object, 3),
            KeyKind::Error => match self.names.len() {
                3 => (KeyKind::Object, 3),
                2 => (KeyKind::Catalog, 2),
                _ => (KeyKind::Connection, 1),
            },
            KeyKind::Object | KeyKind::Folder(_) => (KeyKind::Catalog, 2),
            KeyKind::Catalog | KeyKind::Note => (KeyKind::Connection, 1),
        };
        let mut names = self.names.clone();
        names.truncate(keep);
        Some(RowKey { kind, names })
    }
}

/// The session whose cache the tree shows for connection `conn`: an open
/// one on it, preferring the active session.
pub(crate) fn schema_session(state: &State, conn: usize) -> Option<&SessionState> {
    let id = &state.tree.connections.get(conn)?.record.id;
    let preferred = state
        .sessions
        .for_connection(id)
        .and_then(|s| state.sessions.get(s));
    match preferred {
        Some(session) if session.is_open() => Some(session),
        _ => state
            .sessions
            .iter()
            .find(|s| s.connection_id == *id && s.is_open()),
    }
}

fn cache(state: &State, conn: usize) -> Option<&SchemaCache> {
    schema_session(state, conn).map(|s| &s.schema)
}

/// The catalog a row names.
pub(crate) fn catalog(state: &State, conn: usize, cat: usize) -> Option<&Catalog> {
    cache(state, conn)?.catalogs()?.get(cat)
}

/// The object a row names, with its catalog.
pub(crate) fn object(state: &State, conn: usize, cat: usize, obj: usize) -> Option<&DbObject> {
    let catalog = catalog(state, conn, cat)?;
    cache(state, conn)?.objects(&catalog.name)?.get(obj)
}

/// The columns and indexes of the object a row names.
pub(crate) fn details(state: &State, conn: usize, cat: usize, obj: usize) -> Option<&Details> {
    let object = object(state, conn, cat, obj)?;
    cache(state, conn)?.details(&object.catalog, &object.name)
}

/// What can be loaded at `at`, by name.
pub(crate) fn load_path(state: &State, conn: usize, at: LoadAt) -> Option<SchemaPath> {
    Some(match at {
        LoadAt::Catalogs => SchemaPath::Catalogs,
        LoadAt::Objects { cat } => SchemaPath::objects(&catalog(state, conn, cat)?.name),
        LoadAt::Details { cat, obj } => {
            let object = object(state, conn, cat, obj)?;
            SchemaPath::details(&object.catalog, &object.name)
        }
    })
}

/// The load a row's node depends on: its own children's, or that of the
/// nearest node above it that loads.
pub(crate) fn load_of(node: TreeNode) -> LoadAt {
    match node {
        TreeNode::Connection(_) | TreeNode::Note { .. } => LoadAt::Catalogs,
        TreeNode::Catalog { cat, .. } | TreeNode::Folder { cat, .. } => LoadAt::Objects { cat },
        TreeNode::Object { cat, obj, .. }
        | TreeNode::Section { cat, obj, .. }
        | TreeNode::Column { cat, obj, .. }
        | TreeNode::Index { cat, obj, .. } => LoadAt::Details { cat, obj },
        TreeNode::Error { at, .. } => at,
    }
}

/// The name a row is matched and copied by: the connection's title, the
/// catalog's, object's, column's or index's name. Folders, errors and notes
/// have none.
pub(crate) fn name(state: &State, node: TreeNode) -> Option<&str> {
    match node {
        TreeNode::Connection(conn) => Some(state.tree.connections.get(conn)?.title()),
        TreeNode::Catalog { conn, cat } => Some(&catalog(state, conn, cat)?.name),
        TreeNode::Object { conn, cat, obj } => Some(&object(state, conn, cat, obj)?.name),
        TreeNode::Column {
            conn,
            cat,
            obj,
            col,
        } => Some(&details(state, conn, cat, obj)?.columns.get(col)?.name),
        TreeNode::Index {
            conn,
            cat,
            obj,
            idx,
        } => Some(&details(state, conn, cat, obj)?.indexes.get(idx)?.name),
        TreeNode::Folder { .. }
        | TreeNode::Section { .. }
        | TreeNode::Error { .. }
        | TreeNode::Note { .. } => None,
    }
}

/// The names that identify `node`, outermost first.
fn names(state: &State, node: TreeNode) -> Vec<&str> {
    let conn_id = |conn: usize| {
        state
            .tree
            .connections
            .get(conn)
            .map_or("", |n| n.record.id.as_str())
    };
    let cat_name = |conn, cat| catalog(state, conn, cat).map_or("", |c| c.name.as_str());
    let obj_name = |conn, cat, obj| object(state, conn, cat, obj).map_or("", |o| o.name.as_str());
    match node {
        TreeNode::Connection(conn) | TreeNode::Note { conn, .. } => vec![conn_id(conn)],
        TreeNode::Catalog { conn, cat } | TreeNode::Folder { conn, cat, .. } => {
            vec![conn_id(conn), cat_name(conn, cat)]
        }
        TreeNode::Object { conn, cat, obj } | TreeNode::Section { conn, cat, obj, .. } => {
            vec![conn_id(conn), cat_name(conn, cat), obj_name(conn, cat, obj)]
        }
        TreeNode::Column { conn, cat, obj, .. } | TreeNode::Index { conn, cat, obj, .. } => {
            let mut names = vec![conn_id(conn), cat_name(conn, cat), obj_name(conn, cat, obj)];
            names.extend(name(state, node));
            names
        }
        TreeNode::Error { conn, at } => match at {
            LoadAt::Catalogs => vec![conn_id(conn)],
            LoadAt::Objects { cat } => vec![conn_id(conn), cat_name(conn, cat)],
            LoadAt::Details { cat, obj } => {
                vec![conn_id(conn), cat_name(conn, cat), obj_name(conn, cat, obj)]
            }
        },
    }
}

fn kind(node: TreeNode) -> KeyKind {
    match node {
        TreeNode::Connection(_) => KeyKind::Connection,
        TreeNode::Catalog { .. } => KeyKind::Catalog,
        TreeNode::Folder { folder, .. } => KeyKind::Folder(folder),
        TreeNode::Object { .. } => KeyKind::Object,
        TreeNode::Section { section, .. } => KeyKind::Section(section),
        TreeNode::Column { .. } => KeyKind::Column,
        TreeNode::Index { .. } => KeyKind::Index,
        TreeNode::Error { .. } => KeyKind::Error,
        TreeNode::Note { .. } => KeyKind::Note,
    }
}

/// The key of `node`, by name.
pub(crate) fn key_of(state: &State, node: TreeNode) -> RowKey {
    RowKey {
        kind: kind(node),
        names: names(state, node).into_iter().map(str::to_string).collect(),
    }
}

fn matches_key(state: &State, node: TreeNode, key: &RowKey) -> bool {
    kind(node) == key.kind && names(state, node) == key.names
}

/// Where `key` is in `rows`, or else its nearest ancestor that is. `hint` is
/// where it was, tried first.
pub(crate) fn find(state: &State, rows: &[VisibleRow], key: &RowKey, hint: usize) -> Option<usize> {
    let mut key = Some(key.clone());
    while let Some(k) = key {
        if rows
            .get(hint)
            .is_some_and(|row| matches_key(state, row.node, &k))
        {
            return Some(hint);
        }
        if let Some(i) = rows.iter().position(|row| matches_key(state, row.node, &k)) {
            return Some(i);
        }
        key = k.parent();
    }
    None
}

/// A filter's needle, lower-cased, matched against names.
struct Needle(String);

impl Needle {
    fn hits(&self, name: &str) -> bool {
        name.to_lowercase().contains(&self.0)
    }
}

/// Builds the rows. Without a filter a node's children show while it is
/// open. With one, every loaded node whose name contains the filter shows,
/// with the nodes above it, whether open or not; nothing is loaded for it.
/// Also says whether some nodes the filter could reach are not loaded.
pub(crate) fn flatten(state: &State) -> (Vec<VisibleRow>, bool) {
    let needle = state
        .tree
        .filter
        .as_ref()
        .filter(|f| !f.text.is_empty())
        .map(|f| Needle(f.text.to_lowercase()));
    let mut out = Vec::new();
    let mut partial = false;
    for conn in 0..state.tree.connections.len() {
        connection(state, conn, needle.as_ref(), &mut out, &mut partial);
    }
    (out, partial)
}

/// Pushes `node`, lets `children` push what is under it, and takes it all
/// back if a filter is on and neither the node nor anything under it
/// matched.
fn keep(
    out: &mut Vec<VisibleRow>,
    row: VisibleRow,
    self_hit: bool,
    filtering: bool,
    children: impl FnOnce(&mut Vec<VisibleRow>),
) {
    let start = out.len();
    out.push(row);
    children(out);
    if filtering && !self_hit && out.len() == start + 1 {
        out.truncate(start);
    }
}

fn error_row<T>(
    entry: Option<&Entry<T>>,
    conn: usize,
    at: LoadAt,
    depth: u8,
    filtering: bool,
    out: &mut Vec<VisibleRow>,
) {
    if !filtering && entry.is_some_and(|e| e.error.is_some()) {
        out.push(VisibleRow {
            depth,
            node: TreeNode::Error { conn, at },
        });
    }
}

fn connection(
    state: &State,
    conn: usize,
    needle: Option<&Needle>,
    out: &mut Vec<VisibleRow>,
    partial: &mut bool,
) {
    let node = &state.tree.connections[conn];
    let filtering = needle.is_some();
    let self_hit = needle.is_some_and(|n| n.hits(node.title()));
    let row = VisibleRow {
        depth: 0,
        node: TreeNode::Connection(conn),
    };
    keep(out, row, self_hit, filtering, |out| {
        if !filtering && !node.expanded {
            return;
        }
        let Some(session) = schema_session(state, conn) else {
            *partial |= filtering;
            if node.expanded && !filtering {
                let connecting = state
                    .sessions
                    .connection_status(&node.record.id)
                    .is_some_and(|s| *s == super::state::SessionStatus::Connecting);
                if !connecting {
                    out.push(VisibleRow {
                        depth: 1,
                        node: TreeNode::Note {
                            conn,
                            text: "(not connected)",
                        },
                    });
                }
            }
            return;
        };
        let cache = &session.schema;
        error_row(
            cache.catalogs_entry(),
            conn,
            LoadAt::Catalogs,
            1,
            filtering,
            out,
        );
        let Some(catalogs) = cache.catalogs() else {
            *partial |= filtering;
            return;
        };
        let shown = catalogs
            .iter()
            .enumerate()
            .filter(|(_, c)| state.tree.show_system || !c.is_system);
        let mut any = false;
        for (cat, c) in shown {
            any = true;
            catalog_rows(state, conn, cat, c, cache, needle, out, partial);
        }
        if !any && !filtering && cache.catalogs_entry().is_some_and(|e| e.error.is_none()) {
            out.push(VisibleRow {
                depth: 1,
                node: TreeNode::Note {
                    conn,
                    text: "(no schemas)",
                },
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn catalog_rows(
    state: &State,
    conn: usize,
    cat: usize,
    catalog: &Catalog,
    cache: &SchemaCache,
    needle: Option<&Needle>,
    out: &mut Vec<VisibleRow>,
    partial: &mut bool,
) {
    let filtering = needle.is_some();
    let view = state.tree.connections[conn].catalogs.get(&catalog.name);
    let open = view.is_some_and(|v| v.open);
    let self_hit = needle.is_some_and(|n| n.hits(&catalog.name));
    let row = VisibleRow {
        depth: 1,
        node: TreeNode::Catalog { conn, cat },
    };
    keep(out, row, self_hit, filtering, |out| {
        if !filtering && !open {
            return;
        }
        let entry = cache.objects_entry(&catalog.name);
        error_row(entry, conn, LoadAt::Objects { cat }, 2, filtering, out);
        let Some(objects) = cache.objects(&catalog.name) else {
            *partial |= filtering;
            return;
        };
        for folder in [Folder::Tables, Folder::Views] {
            let folder_open = view.is_some_and(|v| match folder {
                Folder::Tables => v.tables,
                Folder::Views => v.views,
            });
            let row = VisibleRow {
                depth: 2,
                node: TreeNode::Folder { conn, cat, folder },
            };
            keep(out, row, false, filtering, |out| {
                let partial = &mut *partial;
                if !filtering && !folder_open {
                    return;
                }
                for (obj, object) in objects.iter().enumerate() {
                    if folder.holds(object) {
                        object_rows(state, conn, cat, obj, object, cache, needle, out, partial);
                    }
                }
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn object_rows(
    state: &State,
    conn: usize,
    cat: usize,
    obj: usize,
    object: &DbObject,
    cache: &SchemaCache,
    needle: Option<&Needle>,
    out: &mut Vec<VisibleRow>,
    partial: &mut bool,
) {
    let filtering = needle.is_some();
    let self_hit = needle.is_some_and(|n| n.hits(&object.name));
    let row = VisibleRow {
        depth: 3,
        node: TreeNode::Object { conn, cat, obj },
    };
    // Most objects are closed and unfiltered: nothing to look up.
    let view = state.tree.connections[conn]
        .catalogs
        .get(&object.catalog)
        .and_then(|c| c.objects.get(&object.name));
    let open = view.is_some_and(|v| v.open);
    if !filtering && !open {
        out.push(row);
        return;
    }
    keep(out, row, self_hit, filtering, |out| {
        let entry = cache.details_entry(&object.catalog, &object.name);
        error_row(entry, conn, LoadAt::Details { cat, obj }, 4, filtering, out);
        let Some(details) = entry.and_then(|e| e.value.as_ref()) else {
            // Its columns could match, but are not loaded.
            *partial |= filtering;
            return;
        };
        for section in [Section::Columns, Section::Indexes] {
            let section_open = view.is_some_and(|v| match section {
                Section::Columns => v.columns,
                Section::Indexes => v.indexes,
            });
            let row = VisibleRow {
                depth: 4,
                node: TreeNode::Section {
                    conn,
                    cat,
                    obj,
                    section,
                },
            };
            keep(out, row, false, filtering, |out| {
                if !filtering && !section_open {
                    return;
                }
                let names: Vec<&str> = match section {
                    Section::Columns => details.columns.iter().map(|c| c.name.as_str()).collect(),
                    Section::Indexes => details.indexes.iter().map(|i| i.name.as_str()).collect(),
                };
                for (i, name) in names.into_iter().enumerate() {
                    if needle.is_some_and(|n| !n.hits(name)) {
                        continue;
                    }
                    let node = match section {
                        Section::Columns => TreeNode::Column {
                            conn,
                            cat,
                            obj,
                            col: i,
                        },
                        Section::Indexes => TreeNode::Index {
                            conn,
                            cat,
                            obj,
                            idx: i,
                        },
                    };
                    out.push(VisibleRow { depth: 5, node });
                }
            });
        }
    });
}

/// Whether the row's own node is open: drawn `▾` rather than `▸`.
pub(crate) fn is_open(state: &State, node: TreeNode) -> bool {
    let tree = &state.tree;
    match node {
        TreeNode::Connection(conn) => tree.connections.get(conn).is_some_and(|n| n.expanded),
        TreeNode::Catalog { conn, cat } => catalog(state, conn, cat)
            .and_then(|c| tree.connections.get(conn)?.catalogs.get(&c.name))
            .is_some_and(|v| v.open),
        TreeNode::Folder { conn, cat, folder } => catalog(state, conn, cat)
            .and_then(|c| tree.connections.get(conn)?.catalogs.get(&c.name))
            .is_some_and(|v| match folder {
                Folder::Tables => v.tables,
                Folder::Views => v.views,
            }),
        TreeNode::Object { conn, cat, obj } => {
            object_view(state, conn, cat, obj).is_some_and(|v| v.open)
        }
        TreeNode::Section {
            conn,
            cat,
            obj,
            section,
        } => object_view(state, conn, cat, obj).is_some_and(|v| match section {
            Section::Columns => v.columns,
            Section::Indexes => v.indexes,
        }),
        _ => false,
    }
}

fn object_view(state: &State, conn: usize, cat: usize, obj: usize) -> Option<&ObjectView> {
    let object = object(state, conn, cat, obj)?;
    state
        .tree
        .connections
        .get(conn)?
        .catalogs
        .get(&object.catalog)?
        .objects
        .get(&object.name)
}

/// How many children a folder or section has, once loaded.
pub(crate) fn count(state: &State, node: TreeNode) -> Option<usize> {
    match node {
        TreeNode::Folder { conn, cat, folder } => {
            let c = catalog(state, conn, cat)?;
            let objects = cache(state, conn)?.objects(&c.name)?;
            Some(objects.iter().filter(|o| folder.holds(o)).count())
        }
        TreeNode::Section {
            conn,
            cat,
            obj,
            section,
        } => {
            let d = details(state, conn, cat, obj)?;
            Some(match section {
                Section::Columns => d.columns.len(),
                Section::Indexes => d.indexes.len(),
            })
        }
        _ => None,
    }
}

/// Whether the load behind a row's own children is in flight, for its
/// spinner.
pub(crate) fn is_loading(state: &State, node: TreeNode) -> bool {
    let at = match node {
        TreeNode::Connection(_) => LoadAt::Catalogs,
        TreeNode::Catalog { cat, .. } => LoadAt::Objects { cat },
        TreeNode::Object { cat, obj, .. } => LoadAt::Details { cat, obj },
        _ => return false,
    };
    let conn = node.conn();
    let Some(path) = load_path(state, conn, at) else {
        return false;
    };
    cache(state, conn).is_some_and(|c| c.is_loading(&path))
}

/// The error of the load a row stands for.
pub(crate) fn error(state: &State, conn: usize, at: LoadAt) -> Option<&str> {
    let cache = cache(state, conn)?;
    match load_path(state, conn, at)? {
        SchemaPath::Catalogs => cache.catalogs_entry()?.error.as_deref(),
        SchemaPath::Objects { catalog } => cache.objects_entry(&catalog)?.error.as_deref(),
        SchemaPath::Details { catalog, object } => {
            cache.details_entry(&catalog, &object)?.error.as_deref()
        }
    }
}

#[cfg(test)]
#[path = "schematree_tests.rs"]
mod schematree_tests;
