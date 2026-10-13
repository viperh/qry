//! Unit tests for `conntree.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::*;
use crate::components::form_tests::key;

/// A saved SQLite connection, as the form would build it.
fn record(name: &str, path: &str) -> StoredConnection {
    let mut record = StoredConnection::new(name, crate::connections::Driver::Sqlite);
    record.path = Some(path.to_string());
    record.secret = Secret::None;
    record
}

fn tree() -> (ConnTree, UnboundedReceiver<Action>) {
    let (tx, rx) = unbounded_channel();
    let mut tree = ConnTree::default();
    tree.register_action_handler(tx).unwrap();
    tree.register_config_handler(Config::embedded()).unwrap();
    (tree, rx)
}

/// Adds a connection the way the app does: the action, then the answer.
#[allow(clippy::let_underscore_untyped)]
fn connect(tree: &mut ConnTree, name: &str, path: &str) -> StoredConnection {
    let record = record(name, path);
    tree.update(Action::Connect(Box::new(record.clone())))
        .unwrap();
    tree.update(Action::Connected(path.into())).unwrap();
    record
}

fn press(tree: &mut ConnTree, code: KeyCode) {
    tree.handle_key_event(key(code)).unwrap();
}

fn lines(tree: &mut ConnTree, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| tree.draw(frame, frame.area()).unwrap())
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

fn imported(connections: Vec<StoredConnection>) -> Action {
    Action::Imported(Box::new(Imported {
        format: crate::import::Format::DBeaver,
        connections,
        skipped: vec![crate::import::Skipped {
            name: "docs".into(),
            reason: "uses the DBeaver provider `mongodb`".into(),
        }],
        warnings: vec!["prod has no user name; edit it before connecting".into()],
    }))
}

#[test]
fn an_import_adds_what_is_new_saves_once_and_reports() {
    let (mut tree, mut rx) = tree();
    let saved = record("app", "app.db");
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();

    // The same file under another name and id is already here.
    let mut same = record("app copy", "app.db");
    same.id = "other-id".into();
    let new = record("new", "new.db");
    tree.update(imported(vec![same, new.clone(), saved.clone()]))
        .unwrap();

    let Ok(Action::SaveConnections(written)) = rx.try_recv() else {
        panic!("expected one save");
    };
    let ids: Vec<&str> = written.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [saved.id.as_str(), new.id.as_str()]);

    let Ok(Action::Status(StatusCode::Error(message))) = rx.try_recv() else {
        panic!("expected a status");
    };
    assert!(
        message.starts_with("Imported 1 connection from DBeaver"),
        "{message}"
    );
    assert!(message.contains("2 already here"), "{message}");
    assert!(message.contains("skipped docs"), "{message}");
    assert!(message.contains("no user name"), "{message}");
    assert!(rx.try_recv().is_err());

    // Importing it all again changes nothing and writes nothing.
    tree.update(imported(vec![new])).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::Status(_))));
    assert!(rx.try_recv().is_err(), "nothing new, so no save");
}

#[test]
fn an_export_hands_the_saved_list_to_the_app() {
    let (mut tree, mut rx) = tree();
    tree.update(Action::ExportConnections("b.json".into()))
        .unwrap();
    assert_eq!(
        rx.try_recv().unwrap(),
        Action::ConnectionsExportFailed("There are no saved connections to export".into())
    );

    let saved = record("app", "app.db");
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();
    let mut scratch = record("scratch", ":memory:");
    scratch.ephemeral = true;
    tree.update(Action::Connect(Box::new(scratch))).unwrap();
    tree.update(Action::Connected(":memory:".into())).unwrap();

    tree.update(Action::ExportConnections("b.json".into()))
        .unwrap();
    assert_eq!(
        rx.try_recv().unwrap(),
        Action::WriteConnections {
            path: "b.json".into(),
            connections: vec![saved]
        },
        "everything but the scratch database"
    );
}

#[test]
fn a_connection_joins_the_tree_once_it_opens() {
    let (mut tree, _rx) = tree();
    assert!(
        lines(&mut tree, 30, 8)
            .iter()
            .any(|l| l.contains("No connections yet"))
    );

    connect(&mut tree, "prod", "app.db");
    let shown = lines(&mut tree, 30, 8).join("\n");
    assert!(shown.contains("▸ prod"), "{shown}");
    assert_eq!(tree.active, Some(0));

    // The same connection again is not added twice: the id is what
    // counts, not the name.
    let again = tree.connections[0].record.clone();
    tree.update(Action::Connect(Box::new(again))).unwrap();
    tree.update(Action::Connected("app.db".into())).unwrap();
    assert_eq!(tree.connections.len(), 1);
}

#[test]
fn a_connection_that_never_opens_is_not_kept() {
    let (mut tree, _rx) = tree();
    connect(&mut tree, "prod", "app.db");

    tree.update(Action::Connect(Box::new(record("typo", "nope.db"))))
        .unwrap();
    assert_eq!(tree.connections.len(), 2, "added while it is being tried");
    tree.update(Action::ConnectFailed("no such file".into()))
        .unwrap();

    assert_eq!(tree.connections.len(), 1);
    assert_eq!(tree.active, Some(0), "the working one is still active");
    assert!(!lines(&mut tree, 30, 8).join("\n").contains("typo"));
}

#[test]
fn the_saved_connections_join_the_tree_at_startup() {
    let (mut tree, _rx) = tree();
    connect(&mut tree, "scratch", ":memory:");
    let saved = record("prod", "app.db");
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();

    let shown = lines(&mut tree, 30, 8).join("\n");
    assert!(
        shown.contains("scratch") && shown.contains("prod"),
        "{shown}"
    );

    // Loading again does not double anything up.
    tree.update(Action::ConnectionsLoaded(vec![saved])).unwrap();
    assert_eq!(tree.connections.len(), 2);
}

#[test]
fn a_connection_that_opens_is_saved_but_the_scratch_one_is_not() {
    let (mut tree, mut rx) = tree();

    let mut scratch = record("scratch", ":memory:");
    scratch.ephemeral = true;
    tree.update(Action::Connect(Box::new(scratch))).unwrap();
    tree.update(Action::Connected(":memory:".into())).unwrap();
    assert!(
        rx.try_recv().is_err(),
        "the scratch database is never written"
    );

    connect(&mut tree, "prod", "app.db");
    let Ok(Action::SaveConnections(saved)) = rx.try_recv() else {
        panic!("expected a save");
    };
    assert_eq!(saved.len(), 1, "only the real connection");
    assert_eq!(saved[0].name, "prod");
}

#[test]
fn deleting_a_connection_saves_and_forgets_its_password() {
    let (mut tree, mut rx) = tree();
    let mut kept = record("prod", "app.db");
    kept.secret = Secret::Keyring;
    let id = kept.id.clone();
    tree.update(Action::ConnectionsLoaded(vec![kept])).unwrap();
    while rx.try_recv().is_ok() {}

    press(&mut tree, KeyCode::Delete);
    assert_eq!(rx.try_recv().unwrap(), Action::ForgetSecret(id));
    let Ok(Action::SaveConnections(saved)) = rx.try_recv() else {
        panic!("expected a save");
    };
    assert!(saved.is_empty());
    assert!(
        lines(&mut tree, 30, 8)
            .join("\n")
            .contains("No connections yet")
    );
}

#[test]
fn a_keychain_that_refused_downgrades_the_record_and_saves() {
    let (mut tree, mut rx) = tree();
    let mut record = record("prod", "app.db");
    record.secret = Secret::Keyring;
    let id = record.id.clone();
    tree.update(Action::ConnectionsLoaded(vec![record]))
        .unwrap();
    while rx.try_recv().is_ok() {}

    tree.update(Action::SecretNotStored(id)).unwrap();
    let Ok(Action::SaveConnections(saved)) = rx.try_recv() else {
        panic!("expected a save");
    };
    assert_eq!(saved[0].secret, Secret::Prompt, "it asks each time now");
}

/// The cells of one rendered row, for checking its colours.
fn row_style(tree: &mut ConnTree, width: u16, height: u16, y: u16) -> (Color, Color) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| tree.draw(frame, frame.area()).unwrap())
        .unwrap();
    let cell = &terminal.backend().buffer()[(1, y)];
    (cell.fg, cell.bg)
}

#[test]
fn the_selection_is_green_and_the_rest_grey_with_no_background() {
    let (mut tree, _rx) = tree();
    connect(&mut tree, "one", "one.db");
    connect(&mut tree, "two", "two.db");

    // Row 0 is the border, so the connections are on rows 1 and 2.
    assert_eq!(row_style(&mut tree, 30, 8, 1), (Color::Green, Color::Reset));
    assert_eq!(row_style(&mut tree, 30, 8, 2), (Color::Gray, Color::Reset));

    press(&mut tree, KeyCode::Down);
    assert_eq!(row_style(&mut tree, 30, 8, 1), (Color::Gray, Color::Reset));
    assert_eq!(row_style(&mut tree, 30, 8, 2), (Color::Green, Color::Reset));
}

#[test]
fn e_opens_the_selected_connection_for_editing() {
    let (mut tree, mut rx) = tree();
    let saved = record("prod", "app.db");
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();
    while rx.try_recv().is_ok() {}

    press(&mut tree, KeyCode::Char('e'));
    let Ok(Action::EditConnection(asked)) = rx.try_recv() else {
        panic!("expected an edit");
    };
    assert_eq!(asked.id, saved.id);
}

#[test]
fn an_edited_connection_replaces_the_old_one_and_is_saved() {
    let (mut tree, mut rx) = tree();
    let saved = record("prod", "app.db");
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();
    while rx.try_recv().is_ok() {}

    // Same id, new name: the edit, coming back through the app.
    let mut edited = saved.clone();
    edited.name = "production".into();
    tree.update(Action::Connect(Box::new(edited))).unwrap();
    tree.update(Action::Connected("app.db".into())).unwrap();

    assert_eq!(tree.connections.len(), 1, "not a second connection");
    assert!(lines(&mut tree, 30, 8).join("\n").contains("production"));
    let Ok(Action::SaveConnections(list)) = rx.try_recv() else {
        panic!("expected a save");
    };
    assert_eq!(list[0].name, "production");
}

#[test]
fn an_edit_that_stops_using_the_keychain_forgets_the_password() {
    let (mut tree, mut rx) = tree();
    let mut saved = record("prod", "app.db");
    saved.secret = Secret::Keyring;
    let id = saved.id.clone();
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();
    while rx.try_recv().is_ok() {}

    let mut edited = saved.clone();
    edited.secret = Secret::Prompt;
    tree.update(Action::Connect(Box::new(edited))).unwrap();
    assert_eq!(rx.try_recv().unwrap(), Action::ForgetSecret(id));
}

#[test]
fn a_saved_connection_survives_a_failed_connect() {
    let (mut tree, _rx) = tree();
    let saved = record("prod", "app.db");
    tree.update(Action::ConnectionsLoaded(vec![saved.clone()]))
        .unwrap();

    // It has never opened in this session, which is not the same as
    // never having existed.
    tree.update(Action::Connect(Box::new(saved))).unwrap();
    tree.update(Action::ConnectFailed(
        "password authentication failed".into(),
    ))
    .unwrap();

    assert_eq!(tree.connections.len(), 1);
    assert!(lines(&mut tree, 30, 8).join("\n").contains("prod"));
}

#[test]
fn arrows_move_the_selection_and_stop_at_the_ends() {
    let (mut tree, _rx) = tree();
    connect(&mut tree, "one", "one.db");
    connect(&mut tree, "two", "two.db");

    press(&mut tree, KeyCode::Up);
    assert_eq!(tree.selected, 0);
    press(&mut tree, KeyCode::Down);
    assert_eq!(tree.selected, 1);
    press(&mut tree, KeyCode::Down);
    assert_eq!(tree.selected, 1, "stops at the last row");
    press(&mut tree, KeyCode::Home);
    assert_eq!(tree.selected, 0);
    press(&mut tree, KeyCode::End);
    assert_eq!(tree.selected, 1);
}

#[test]
fn enter_connects_to_the_selected_connection() {
    let (mut tree, mut rx) = tree();
    connect(&mut tree, "one", "one.db");
    connect(&mut tree, "two", "two.db");
    while rx.try_recv().is_ok() {}

    press(&mut tree, KeyCode::Down);
    press(&mut tree, KeyCode::Enter);
    let Ok(Action::Connect(sent)) = rx.try_recv() else {
        panic!("expected a connection");
    };
    assert_eq!(sent.name, "two");
}

#[test]
fn space_expands_the_active_connection_and_shows_its_tables() {
    let (mut tree, mut rx) = tree();
    connect(&mut tree, "prod", "app.db");
    while rx.try_recv().is_ok() {}

    press(&mut tree, KeyCode::Char(' '));
    assert_eq!(rx.try_recv().unwrap(), Action::ListTables);
    let shown = lines(&mut tree, 30, 8).join("\n");
    assert!(
        shown.contains("▾ prod") && shown.contains("loading"),
        "{shown}"
    );

    tree.update(Action::TablesLoaded(vec!["users".into(), "orders".into()]))
        .unwrap();
    let shown = lines(&mut tree, 30, 8).join("\n");
    assert!(
        shown.contains("users") && shown.contains("orders"),
        "{shown}"
    );

    // Tables are rows of their own, so the selection walks through them.
    press(&mut tree, KeyCode::End);
    assert_eq!(tree.selected, 2);

    // Collapsing hides them again.
    press(&mut tree, KeyCode::Home);
    press(&mut tree, KeyCode::Char(' '));
    assert!(!lines(&mut tree, 30, 8).join("\n").contains("users"));
}

#[test]
fn expanding_another_connection_opens_it_first() {
    let (mut tree, mut rx) = tree();
    connect(&mut tree, "one", "one.db");
    connect(&mut tree, "two", "two.db");
    tree.update(Action::TablesLoaded(vec!["t".into()])).unwrap();
    while rx.try_recv().is_ok() {}

    // "one" is not the active connection, so Space connects to it first.
    press(&mut tree, KeyCode::Home);
    press(&mut tree, KeyCode::Char(' '));
    let sent = rx.try_recv().unwrap();
    let Action::Connect(ref asked) = sent else {
        panic!("expected a connection")
    };
    assert_eq!(asked.name, "one");
    assert!(rx.try_recv().is_err(), "tables come after it is connected");
    // `App` hands every action back to the components, this one included.
    tree.update(sent).unwrap();

    tree.update(Action::Connected("one.db".into())).unwrap();
    assert_eq!(rx.try_recv().unwrap(), Action::ListTables);
    assert_eq!(tree.active, Some(0));

    tree.update(Action::TablesLoaded(vec!["people".into()]))
        .unwrap();
    assert!(lines(&mut tree, 30, 8).join("\n").contains("people"));
}

#[test]
fn a_failed_table_list_closes_the_branch_so_it_can_be_tried_again() {
    let (mut tree, mut rx) = tree();
    connect(&mut tree, "prod", "app.db");
    press(&mut tree, KeyCode::Char(' '));
    while rx.try_recv().is_ok() {}

    tree.update(Action::TablesFailed("connection lost".into()))
        .unwrap();
    let shown = lines(&mut tree, 30, 8).join("\n");
    assert!(shown.contains("▸ prod"), "{shown}");
    assert!(!shown.contains("loading"), "{shown}");

    press(&mut tree, KeyCode::Char(' '));
    assert_eq!(rx.try_recv().unwrap(), Action::ListTables);
}

#[test]
fn long_trees_scroll_to_keep_the_selection_in_view() {
    let (mut tree, _rx) = tree();
    for i in 0..20 {
        connect(&mut tree, &format!("c{i}"), &format!("{i}.db"));
    }
    press(&mut tree, KeyCode::End);

    // 8 rows tall: 2 border rows leave 6 connections in view.
    let shown = lines(&mut tree, 30, 8).join("\n");
    assert!(shown.contains("c19"), "{shown}");
    assert!(!shown.contains("c0 ") && !shown.contains("▸ c0"), "{shown}");
}
