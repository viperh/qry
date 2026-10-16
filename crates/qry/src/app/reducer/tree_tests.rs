//! Unit tests for `tree.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    style::{Color, Style},
};

use qry_core::schema::{Children, SchemaPath};

use super::*;
use crate::{
    app::{
        DbAnswer, SessionId,
        dialogs::{DialogState, connexportform::ConnExportForm, form::Form},
        state::Focus,
    },
    config::Config,
    testing::{buffer_lines, render_with},
    theme::Theme,
    views,
};

/// A saved SQLite connection, as the form would build it.
fn record(name: &str, path: &str) -> StoredConnection {
    let mut record = StoredConnection::new(name, crate::connections::Driver::Sqlite);
    record.path = Some(path.to_string());
    record.secret = Secret::None;
    record
}

/// The state with the tree focused, and the effects the tree asked for.
struct Tree {
    state: State,
    fx: Vec<Effect>,
    /// The session the last connection was opened in.
    session: SessionId,
}

impl Tree {
    fn new() -> Self {
        Self {
            state: State {
                config: Arc::new(Config::embedded()),
                focus: Focus::Tree,
                ..State::default()
            },
            fx: Vec::new(),
            session: SessionId::FIRST,
        }
    }

    /// The effects since the last call, oldest first, without the
    /// spinner's pacing.
    fn take(&mut self) -> Vec<Effect> {
        let mut fx = std::mem::take(&mut self.fx);
        fx.retain(|e| {
            !matches!(
                e,
                Effect::ScheduleTimer {
                    timer: crate::app::event::Timer::Tree(_),
                    ..
                }
            )
        });
        fx
    }

    /// What every event ends with in the app: loads asked for, rows worked
    /// out again.
    fn settle(&mut self) {
        after(&mut self.state, &mut self.fx);
    }

    /// A connection submitted from the New Connection modal, as the app
    /// opens it: in the session already on it, or a new one.
    fn attempt(&mut self, record: &StoredConnection) {
        self.session = super::super::connect(&mut self.state, record.clone(), &mut self.fx);
        self.settle();
    }

    /// An answer from the last session opened.
    fn answer(&mut self, answer: DbAnswer) {
        super::super::db::answer(&mut self.state, self.session, answer, &mut self.fx);
        self.settle();
    }

    fn connected(&mut self, label: &str) {
        self.answer(DbAnswer::Connected(label.into()));
    }

    fn failed(&mut self) {
        self.answer(DbAnswer::ConnectFailed("no such file".into()));
    }

    /// Answers every schema load asked for so far, from a `main` catalog
    /// holding `tables`, until nothing more is asked.
    fn tables(&mut self, tables: &[&str]) {
        loop {
            let (loads, rest): (Vec<Effect>, Vec<Effect>) = std::mem::take(&mut self.fx)
                .into_iter()
                .partition(|e| matches!(e, Effect::Db(_, DbCommand::LoadSchema { .. })));
            self.fx = rest;
            if loads.is_empty() {
                return;
            }
            for load in loads {
                let Effect::Db(id, DbCommand::LoadSchema { path, generation }) = load else {
                    continue;
                };
                let children = match &path {
                    SchemaPath::Catalogs => Children::Catalogs(vec![qry_core::schema::Catalog {
                        name: "main".into(),
                        is_default: true,
                        is_system: false,
                    }]),
                    SchemaPath::Objects { catalog } => Children::Objects(
                        tables
                            .iter()
                            .map(|t| qry_core::schema::DbObject {
                                catalog: catalog.clone(),
                                name: t.to_string(),
                                kind: qry_core::schema::ObjectKind::Table,
                            })
                            .collect(),
                    ),
                    SchemaPath::Details { .. } => Children::Details(Default::default()),
                };
                let answer = DbAnswer::Schema {
                    path,
                    generation,
                    outcome: Ok(children),
                };
                super::super::db::answer(&mut self.state, id, answer, &mut self.fx);
                self.settle();
            }
        }
    }

    /// The connection the active session is on, if the tree lists it.
    fn active(&self) -> Option<usize> {
        let session = self.state.sessions.active();
        self.state.tree.position(&session.connection_id)
    }

    fn loaded(&mut self, records: Vec<StoredConnection>) {
        connections_loaded(&mut self.state, records);
        self.settle();
    }

    fn press(&mut self, code: KeyCode) {
        super::super::keys::key(
            &mut self.state,
            KeyEvent::new(code, KeyModifiers::NONE),
            &mut self.fx,
        );
        self.settle();
    }

    /// Draws the tree into a `width`x`height` terminal.
    fn drawn(&self, width: u16, height: u16) -> Buffer {
        render_with(width, height, |frame| {
            views::tree::draw(&self.state, &Theme::DARK, frame, frame.area());
        })
    }

    /// The rows of the tree drawn at `width`x`height`.
    fn lines(&self, width: u16, height: u16) -> String {
        buffer_lines(&self.drawn(width, height)).join("\n")
    }
}

/// Adds a connection the way the app does: the attempt, then the answer.
fn connect(tree: &mut Tree, name: &str, path: &str) -> StoredConnection {
    let record = record(name, path);
    tree.attempt(&record);
    tree.connected(path);
    record
}

fn imported(connections: Vec<StoredConnection>) -> Imported {
    Imported {
        format: crate::import::Format::DBeaver,
        connections,
        skipped: vec![crate::import::Skipped {
            name: "docs".into(),
            reason: "uses the DBeaver provider `mongodb`".into(),
        }],
        warnings: vec!["prod has no user name; edit it before connecting".into()],
    }
}

/// The records of the one save among `effects`.
fn saved(effects: &[Effect]) -> Vec<StoredConnection> {
    let saves: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SaveConnections(list) => Some(list.clone()),
            _ => None,
        })
        .collect();
    let [list] = saves.as_slice() else {
        panic!("expected one save, got {effects:?}");
    };
    list.clone()
}

#[test]
fn an_import_adds_what_is_new_saves_once_and_reports() {
    let mut tree = Tree::new();
    let saved_one = record("app", "app.db");
    tree.loaded(vec![saved_one.clone()]);

    // The same file under another name and id is already here.
    let mut same = record("app copy", "app.db");
    same.id = "other-id".into();
    let new = record("new", "new.db");
    import(
        &mut tree.state,
        imported(vec![same, new.clone(), saved_one.clone()]),
        &mut tree.fx,
    );

    let written = saved(&tree.take());
    let ids: Vec<&str> = written.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [saved_one.id.as_str(), new.id.as_str()]);

    let (_, notice) = tree.state.messages.newest_first().next().unwrap();
    assert_eq!(
        notice.level,
        crate::notice::Level::Warning,
        "some were skipped"
    );
    let message = &notice.text;
    assert!(
        message.starts_with("Imported 1 connection from DBeaver"),
        "{message}"
    );
    assert!(message.contains("2 already here"), "{message}");
    assert!(message.contains("skipped docs"), "{message}");
    assert!(message.contains("no user name"), "{message}");

    // Importing it all again changes nothing and writes nothing.
    import(&mut tree.state, imported(vec![new]), &mut tree.fx);
    assert_eq!(tree.state.messages.len(), 2, "it still says so");
    assert!(tree.take().is_empty(), "nothing new, so no save");
}

#[test]
fn an_export_hands_the_saved_list_to_the_runtime() {
    let mut tree = Tree::new();
    // The modal waits for the answer, which here is a refusal.
    tree.state
        .open_dialog(DialogState::ConnExport(ConnExportForm::default()));
    tree.state.awaiting = Some(Awaiting::ExportConnections);
    export_connections(&mut tree.state, "b.json".into(), &mut tree.fx);
    assert!(tree.take().is_empty());
    assert_eq!(
        tree.state.dialog().unwrap().form().error(),
        Some("There are no saved connections to export")
    );

    let saved_one = record("app", "app.db");
    tree.loaded(vec![saved_one.clone()]);
    let mut scratch = record("scratch", ":memory:");
    scratch.ephemeral = true;
    tree.attempt(&scratch);
    tree.connected(":memory:");
    tree.take();

    export_connections(&mut tree.state, "b.json".into(), &mut tree.fx);
    assert_eq!(
        tree.take(),
        [Effect::ExportConnections {
            path: "b.json".into(),
            connections: vec![saved_one]
        }],
        "everything but the scratch database"
    );
}

#[test]
fn a_connection_joins_the_tree_once_it_opens() {
    let mut tree = Tree::new();
    assert!(tree.lines(30, 8).contains("No connections yet"));

    connect(&mut tree, "prod", "app.db");
    let shown = tree.lines(30, 8);
    assert!(shown.contains("▸ ● sq  prod"), "{shown}");
    assert_eq!(tree.active(), Some(0));

    // The same connection again is not added twice: the id is what
    // counts, not the name.
    let again = tree.state.tree.connections[0].record.clone();
    tree.attempt(&again);
    tree.connected("app.db");
    assert_eq!(tree.state.tree.connections.len(), 1);
}

#[test]
fn a_connection_that_never_opens_is_not_kept() {
    let mut tree = Tree::new();
    connect(&mut tree, "prod", "app.db");

    tree.attempt(&record("typo", "nope.db"));
    assert_eq!(
        tree.state.tree.connections.len(),
        2,
        "added while it is being tried"
    );
    let typo = tree.session;
    tree.failed();

    assert_eq!(tree.state.tree.connections.len(), 1);
    assert!(
        tree.state.sessions.get(typo).is_none(),
        "its session is closed too"
    );
    assert_eq!(tree.active(), Some(0), "the working one is active again");
    assert!(!tree.lines(30, 8).contains("typo"));
}

#[test]
fn the_saved_connections_join_the_tree_at_startup() {
    let mut tree = Tree::new();
    connect(&mut tree, "scratch", ":memory:");
    let saved_one = record("prod", "app.db");
    tree.loaded(vec![saved_one.clone()]);

    let shown = tree.lines(30, 8);
    assert!(
        shown.contains("scratch") && shown.contains("prod"),
        "{shown}"
    );

    // Loading again does not double anything up.
    tree.loaded(vec![saved_one]);
    assert_eq!(tree.state.tree.connections.len(), 2);
}

#[test]
fn a_connection_that_opens_is_saved_but_the_scratch_one_is_not() {
    let mut tree = Tree::new();

    let mut scratch = record("scratch", ":memory:");
    scratch.ephemeral = true;
    tree.attempt(&scratch);
    tree.connected(":memory:");
    assert!(
        !tree
            .take()
            .iter()
            .any(|e| matches!(e, Effect::SaveConnections(_))),
        "the scratch database is never written"
    );

    connect(&mut tree, "prod", "app.db");
    let written = saved(&tree.take());
    assert_eq!(written.len(), 1, "only the real connection");
    assert_eq!(written[0].name, "prod");
}

#[test]
fn deleting_a_connection_saves_and_forgets_its_password() {
    let mut tree = Tree::new();
    let mut kept = record("prod", "app.db");
    kept.secret = Secret::Keyring;
    let id = kept.id.clone();
    tree.loaded(vec![kept]);

    // It asks first.
    tree.press(KeyCode::Delete);
    assert!(tree.take().is_empty());
    assert!(matches!(tree.state.dialog(), Some(DialogState::Confirm(_))));
    tree.press(KeyCode::Char('y'));
    assert_eq!(
        tree.take(),
        [
            Effect::ForgetSecret(id),
            Effect::SaveConnections(Vec::new())
        ]
    );
    assert!(tree.state.dialog().is_none());
    assert!(tree.lines(30, 8).contains("No connections yet"));
}

#[test]
fn a_keychain_that_refused_downgrades_the_record_and_saves() {
    let mut tree = Tree::new();
    let mut kept = record("prod", "app.db");
    kept.secret = Secret::Keyring;
    let id = kept.id.clone();
    tree.loaded(vec![kept]);

    secret_not_stored(&mut tree.state, &id, &mut tree.fx);
    let written = saved(&tree.take());
    assert_eq!(written[0].secret, Secret::Prompt, "it asks each time now");
}

/// The background of the first cell of one rendered row.
fn row_style(tree: &Tree, width: u16, height: u16, y: u16) -> Color {
    tree.drawn(width, height)[(1, y)].bg
}

/// The background a theme style draws with (an unset colour draws as the
/// default, `Reset`).
fn colours(style: Style) -> Color {
    style.bg.unwrap_or_default()
}

#[test]
fn the_selection_takes_the_selected_style_and_the_rest_the_plain_one() {
    let mut tree = Tree::new();
    connect(&mut tree, "one", "one.db");
    connect(&mut tree, "two", "two.db");
    let t = Theme::DARK;
    let (selected, plain) = (
        colours(t.tree_connection_selected()),
        colours(t.tree_connection()),
    );
    assert_ne!(selected, plain);

    // Row 0 is the title, so the connections are on rows 1 and 2.
    assert_eq!(row_style(&tree, 30, 8, 1), selected);
    assert_eq!(row_style(&tree, 30, 8, 2), plain);

    tree.press(KeyCode::Down);
    assert_eq!(row_style(&tree, 30, 8, 1), plain);
    assert_eq!(row_style(&tree, 30, 8, 2), selected);
}

#[test]
fn e_opens_the_selected_connection_for_editing() {
    let mut tree = Tree::new();
    let saved_one = record("prod", "app.db");
    tree.loaded(vec![saved_one.clone()]);

    tree.press(KeyCode::Char('e'));
    let Some(DialogState::Connection(form)) = tree.state.dialog() else {
        panic!("expected the connection form");
    };
    assert_eq!(form.title(), "Edit Connection");
    assert_eq!(form.text(0), "prod");
    assert!(tree.take().is_empty());
}

#[test]
fn an_edited_connection_replaces_the_old_one_and_is_saved() {
    let mut tree = Tree::new();
    let saved_one = record("prod", "app.db");
    tree.loaded(vec![saved_one.clone()]);

    // Same id, new name: the edit, coming back through the app.
    let mut edited = saved_one.clone();
    edited.name = "production".into();
    tree.attempt(&edited);
    tree.connected("app.db");

    assert_eq!(
        tree.state.tree.connections.len(),
        1,
        "not a second connection"
    );
    assert!(tree.lines(30, 8).contains("production"));
    assert_eq!(saved(&tree.take())[0].name, "production");
}

#[test]
fn an_edit_that_stops_using_the_keychain_forgets_the_password() {
    let mut tree = Tree::new();
    let mut saved_one = record("prod", "app.db");
    saved_one.secret = Secret::Keyring;
    let id = saved_one.id.clone();
    tree.loaded(vec![saved_one.clone()]);

    let mut edited = saved_one.clone();
    edited.secret = Secret::Prompt;
    attempt(&mut tree.state, &edited, &mut tree.fx);
    assert_eq!(tree.take(), [Effect::ForgetSecret(id)]);
}

#[test]
fn a_saved_connection_survives_a_failed_connect() {
    let mut tree = Tree::new();
    let saved_one = record("prod", "app.db");
    tree.loaded(vec![saved_one.clone()]);

    // It has never opened in this session, which is not the same as
    // never having existed.
    tree.attempt(&saved_one);
    tree.failed();

    assert_eq!(tree.state.tree.connections.len(), 1);
    assert!(tree.lines(30, 8).contains("▸ ✗ sq  prod"));
}

#[test]
fn arrows_move_the_selection_and_stop_at_the_ends() {
    let mut tree = Tree::new();
    connect(&mut tree, "one", "one.db");
    connect(&mut tree, "two", "two.db");

    tree.press(KeyCode::Up);
    assert_eq!(tree.state.tree.selected, 0);
    tree.press(KeyCode::Down);
    assert_eq!(tree.state.tree.selected, 1);
    tree.press(KeyCode::Down);
    assert_eq!(tree.state.tree.selected, 1, "stops at the last row");
    tree.press(KeyCode::Home);
    assert_eq!(tree.state.tree.selected, 0);
    tree.press(KeyCode::End);
    assert_eq!(tree.state.tree.selected, 1);
}

#[test]
fn enter_opens_a_session_on_the_selected_connection() {
    let mut tree = Tree::new();
    connect(&mut tree, "one", "one.db");
    tree.loaded(vec![record("two", "two.db")]);
    tree.take();

    tree.press(KeyCode::Down);
    tree.press(KeyCode::Enter);
    let effects = tree.take();
    let [Effect::OpenSession(id, sent)] = effects.as_slice() else {
        panic!("expected a session, got {effects:?}");
    };
    assert_eq!(sent.name, "two");
    assert_eq!(tree.state.sessions.active, *id);
    assert!(tree.lines(30, 8).contains("▸ ⠋ sq  two"), "connecting");
}

#[test]
fn space_expands_the_active_connection_and_shows_its_tables() {
    let mut tree = Tree::new();
    connect(&mut tree, "prod", "app.db");
    tree.take();

    tree.press(KeyCode::Char(' '));
    let effects = tree.take();
    let [
        Effect::Db(
            _,
            DbCommand::LoadSchema {
                path: SchemaPath::Catalogs,
                generation,
            },
        ),
    ] = effects.as_slice()
    else {
        panic!("expected the catalogs to load, got {effects:?}");
    };
    tree.fx.push(Effect::Db(
        tree.session,
        DbCommand::LoadSchema {
            path: SchemaPath::Catalogs,
            generation: *generation,
        },
    ));
    let shown = tree.lines(30, 8);
    assert!(
        shown.contains("▾ ● sq  prod ⠋"),
        "a spinner while it loads: {shown}"
    );

    tree.tables(&["users", "orders"]);
    let shown = tree.lines(30, 8);
    assert!(shown.contains("▾ main"), "its own catalog opens: {shown}");
    assert!(shown.contains("▾ Tables  2"), "{shown}");
    assert!(
        shown.contains("users") && shown.contains("orders"),
        "{shown}"
    );

    // Tables are rows of their own, so the selection walks through them:
    // prod, main, Tables, orders, users, Views.
    tree.press(KeyCode::End);
    assert_eq!(tree.state.tree.selected, 5);

    // Collapsing hides them again.
    tree.press(KeyCode::Home);
    tree.press(KeyCode::Char(' '));
    assert!(!tree.lines(30, 8).contains("users"));
}

#[test]
fn expanding_another_connection_opens_it_first() {
    let mut tree = Tree::new();
    tree.loaded(vec![record("one", "one.db")]);
    connect(&mut tree, "two", "two.db");
    tree.take();

    // "one" has no session, so Space opens one first.
    tree.press(KeyCode::Home);
    tree.press(KeyCode::Char(' '));
    let effects = tree.take();
    let [Effect::OpenSession(id, asked)] = effects.as_slice() else {
        panic!("expected only a session, got {effects:?}: tables come after it is connected");
    };
    assert_eq!(asked.name, "one");
    tree.session = *id;

    tree.connected("one.db");
    assert!(
        tree.fx.iter().any(|e| matches!(
            e,
            Effect::Db(s, DbCommand::LoadSchema { path: SchemaPath::Catalogs, .. }) if s == id
        )),
        "{:?}",
        tree.fx
    );
    assert_eq!(tree.active(), Some(0));

    tree.tables(&["people"]);
    assert!(tree.lines(30, 8).contains("people"));
}

#[test]
fn a_failed_load_shows_why_and_r_tries_again() {
    let mut tree = Tree::new();
    connect(&mut tree, "prod", "app.db");
    tree.take();
    tree.press(KeyCode::Char(' '));
    let effects = tree.take();
    let [Effect::Db(_, DbCommand::LoadSchema { generation, .. })] = effects.as_slice() else {
        panic!("{effects:?}");
    };

    tree.answer(DbAnswer::Schema {
        path: SchemaPath::Catalogs,
        generation: *generation,
        outcome: Err("gone".into()),
    });
    let shown = tree.lines(30, 8);
    assert!(shown.contains("▾ ● sq  prod"), "{shown}");
    assert!(shown.contains("! gone"), "{shown}");
    assert!(tree.take().is_empty(), "no retry on its own");

    tree.press(KeyCode::Down);
    tree.press(KeyCode::Char('r'));
    assert!(matches!(
        tree.take().as_slice(),
        [Effect::Db(
            _,
            DbCommand::LoadSchema {
                path: SchemaPath::Catalogs,
                ..
            }
        )]
    ));
}

#[test]
fn long_trees_scroll_to_keep_the_selection_in_view() {
    let mut tree = Tree::new();
    for i in 0..20 {
        connect(&mut tree, &format!("c{i}"), &format!("{i}.db"));
    }
    tree.press(KeyCode::End);

    // 8 rows tall: the title leaves 7 connections in view.
    let shown = tree.lines(30, 8);
    assert!(shown.contains("c19"), "{shown}");
    assert!(!shown.contains("c0 ") && !shown.contains("▸ c0"), "{shown}");
}

#[test]
fn deleting_a_connection_being_opened_drops_its_late_answer() {
    let mut tree = Tree::new();
    connect(&mut tree, "one", "one.db");
    let two = record("two", "two.db");
    tree.attempt(&two);
    tree.take();

    // Deleted while it opens, and then the answers arrive: they find no
    // connection and change nothing.
    tree.press(KeyCode::End);
    tree.press(KeyCode::Delete);
    tree.press(KeyCode::Char('y'));
    tree.connected("two.db");
    tree.tables(&["t"]);
    assert_eq!(tree.state.tree.connections.len(), 1);
    assert!(!tree.lines(30, 8).contains("two"));
}

#[test]
fn the_tree_marks_each_connection_by_its_sessions() {
    let mut tree = Tree::new();
    tree.loaded(vec![record("idle", "idle.db")]);
    connect(&mut tree, "open", "open.db");
    let mut down = record("down", "down.db");
    down.id = "down".into();
    tree.loaded(vec![down.clone()]);
    tree.attempt(&down);
    let shown = tree.lines(30, 8);
    assert!(shown.contains("▸ ○ sq  idle"), "no session: {shown}");
    assert!(shown.contains("▸ ● sq  open"), "{shown}");
    assert!(shown.contains("▸ ⠋ sq  down"), "{shown}");

    tree.failed();
    assert!(tree.lines(30, 8).contains("▸ ✗ sq  down"));
    let t = Theme::DARK;
    let row = tree.drawn(30, 8);
    let dot = (0..30).find(|x| row[(*x, 3)].symbol() == "✗").unwrap();
    assert_eq!(row[(dot, 3)].fg, t.value_error().fg.unwrap());
}
