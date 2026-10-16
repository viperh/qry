//! Overlays through the reducer: the backdrop, the dialogs' accents and
//! footers, the pending spinner, and the confirm before a delete.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ratatui::style::Modifier;

use super::*;
use crate::{
    connections::{Driver, Secret, StoredConnection},
    testing::{AppHarness, buffer_lines, styled},
};

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.db(DbAnswer::Connected(":memory:".into()));
    h.take_effects();
    h
}

/// Fills in enough of the New Connection form to be submittable.
fn fill_connection(h: &mut AppHarness) {
    for (field, value) in [(0, "prod"), (2, "db.local"), (4, "app"), (6, "app")] {
        h.state_mut()
            .dialog_mut()
            .unwrap()
            .form_mut()
            .set_focus(field);
        h.type_str(value);
    }
    h.state_mut().dialog_mut().unwrap().form_mut().set_focus(2);
}

/// A saved connection `prod` whose password is in the keychain.
fn with_saved(h: &mut AppHarness) -> StoredConnection {
    let mut record = StoredConnection::with_id("prod", "prod", Driver::Sqlite);
    record.path = Some("prod.db".into());
    record.secret = Secret::Keyring;
    h.file(FileAnswer::ConnectionsLoaded(vec![record.clone()]));
    h.state_mut().tree.selected = 1;
    h.state_mut().tree.selected_key = None;
    record
}

#[test]
fn an_open_overlay_dims_the_screen_and_none_dims_nothing() {
    let mut h = harness();
    let buffer = h.render_buffer(80, 24);
    for y in 0..24 {
        for x in 0..80 {
            assert!(
                !buffer[(x, y)].modifier.contains(Modifier::DIM)
                    || buffer[(x, y)].fg == h.theme().fg_dim,
                "({x},{y}) dimmed without an overlay"
            );
        }
    }
    h.keys("ctrl-n");
    let buffer = h.render_buffer(80, 24);
    for (x, y) in [(0, 0), (79, 0), (0, 23), (79, 23)] {
        assert!(buffer[(x, y)].modifier.contains(Modifier::DIM), "({x},{y})");
        assert_eq!(buffer[(x, y)].fg, h.theme().fg_dim);
    }
}

#[test]
fn each_dialog_uses_its_accent() {
    let mut h = harness();
    let t = *h.theme();
    let corner = |h: &mut AppHarness| {
        let buffer = h.render_buffer(120, 40);
        let (x, y) = (0..40u16)
            .flat_map(|y| (0..120u16).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == "┌")
            .unwrap();
        buffer[(x, y)].fg
    };
    h.keys("ctrl-n");
    assert_eq!(corner(&mut h), t.teal, "New Connection");
    h.keys("esc ctrl-o");
    assert_eq!(corner(&mut h), t.teal, "Import");
    h.keys("esc");
    h.state_mut().focus = state::Focus::Results;
    h.keys("ctrl-s");
    assert_eq!(corner(&mut h), t.amber, "Export");
    h.keys("esc");
    with_saved(&mut h);
    h.state_mut().focus = state::Focus::Tree;
    h.keys("delete");
    assert_eq!(corner(&mut h), t.coral, "Delete");
}

#[test]
fn pending_spinner_advances_every_100ms() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    h.keys("enter");
    assert_eq!(h.state().spinner, 0);
    let text = h.render(120, 40);
    assert!(text.contains("⠋ Connecting…"), "{text}");
    h.advance(250);
    assert_eq!(h.state().spinner, 2);
    assert!(h.render(120, 40).contains("⠹ Connecting…"));

    // An answer stops it.
    let session = h.active_session();
    h.db_for(session, DbAnswer::ConnectFailed("refused".into()));
    h.advance(500);
    assert_eq!(h.state().spinner, 2);
}

#[test]
fn a_failure_shows_a_coral_error_and_keeps_every_value() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    h.keys("enter");
    let session = h.active_session();
    h.db_for(
        session,
        DbAnswer::ConnectFailed("password authentication failed".into()),
    );
    let buffer = h.render_buffer(120, 40);
    let lines = buffer_lines(&buffer);
    let y = lines
        .iter()
        .position(|l| l.contains("password authentication failed"))
        .unwrap();
    let x = lines[y][..lines[y].find("password").unwrap()]
        .chars()
        .count();
    assert_eq!(
        buffer[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())].fg,
        h.theme().coral
    );
    let text = lines.join("\n");
    assert!(text.contains("prod") && text.contains("db.local"), "{text}");
}

#[test]
fn delete_from_tree_asks_first() {
    let mut h = harness();
    with_saved(&mut h);
    h.keys("delete");
    assert!(h.take_effects().is_empty());
    assert_eq!(h.dialog_title(), Some("Delete Connection"));
    let text = h.render(80, 24);
    assert!(text.contains("Delete \"prod\""), "{text}");
    assert!(text.contains("[ Cancel ]"), "Cancel is the default: {text}");

    for keys in ["esc", "n", "enter"] {
        if h.state().dialog().is_none() {
            h.keys("delete");
        }
        h.keys(keys);
        assert!(h.state().dialog().is_none(), "{keys} closes it");
        assert!(h.take_effects().is_empty(), "{keys} keeps the connection");
        assert_eq!(h.state().tree.connections.len(), 2);
    }
}

#[test]
fn confirmed_delete_removes_and_forgets_secret() {
    let mut h = harness();
    let record = with_saved(&mut h);
    h.keys("delete y");
    assert_eq!(
        h.take_effects(),
        [
            Effect::ForgetSecret(record.id.clone()),
            Effect::SaveConnections(Vec::new())
        ]
    );
    assert_eq!(h.state().tree.connections.len(), 1, "only scratch is left");

    // Choosing Delete with the arrow keys does the same.
    let mut h = harness();
    with_saved(&mut h);
    h.keys("delete right enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::ForgetSecret(_), _]
    ));
}

#[test]
fn dialog_snapshots() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    insta::assert_snapshot!("dialog_new_connection_120x40", h.render(120, 40));
    insta::assert_snapshot!(
        "dialog_new_connection_120x40_styled",
        styled(&h.render_buffer(120, 40))
    );
    h.state_mut().dialog_mut().unwrap().form_mut().set_focus(10);
    h.resize(80, 24);
    insta::assert_snapshot!("dialog_new_connection_80x24_scrolled", h.render(80, 24));
    h.resize(120, 40);
    h.state_mut().dialog_mut().unwrap().form_mut().set_focus(2);
    h.keys("enter");
    h.advance(200);
    insta::assert_snapshot!("dialog_new_connection_pending_120x40", h.render(120, 40));
    let session = h.active_session();
    h.db_for(
        session,
        DbAnswer::ConnectFailed("could not connect to db.local: connection refused".into()),
    );
    insta::assert_snapshot!("dialog_new_connection_error_120x40", h.render(120, 40));

    let mut h = harness();
    h.state_mut().focus = state::Focus::Results;
    h.keys("ctrl-s");
    insta::assert_snapshot!("dialog_export_120x40", h.render(120, 40));
    insta::assert_snapshot!(
        "dialog_export_120x40_styled",
        styled(&h.render_buffer(120, 40))
    );
    h.keys("esc ctrl-o");
    insta::assert_snapshot!("dialog_import_120x40", h.render(120, 40));
    h.keys("esc");
    h.db(DbAnswer::NeedPassword {
        id: "prod".into(),
        name: "prod".into(),
    });
    h.type_str("hunter2");
    let text = h.render(80, 24);
    assert!(!text.contains("hunter2"));
    insta::assert_snapshot!("dialog_password_80x24", text);

    let mut h = harness();
    with_saved(&mut h);
    h.keys("delete");
    insta::assert_snapshot!("confirm_delete_80x24", h.render(80, 24));
    insta::assert_snapshot!(
        "confirm_delete_80x24_styled",
        styled(&h.render_buffer(80, 24))
    );
}

#[test]
fn masked_field_never_shows_plaintext() {
    let mut h = harness();
    h.db(DbAnswer::NeedPassword {
        id: "prod".into(),
        name: "prod".into(),
    });
    h.type_str("hunter2");
    let buffer = h.render_buffer(80, 24);
    let lines = buffer_lines(&buffer);
    let row = lines.iter().find(|l| l.contains("› Password")).unwrap();
    assert!(row.contains("•••••••"), "{row}");
    assert!(!row.contains('h'), "{row}");
}
