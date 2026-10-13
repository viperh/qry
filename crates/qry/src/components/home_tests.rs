//! Unit tests for `home.rs`.
//!
//! These drive the main screen through [`AppHarness`], so keys go through the
//! global keymap as they do in the app, and the I/O `App` would start shows
//! up as effects.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    action::StatusCode,
    connections::{Driver, StoredConnection},
    testing::{AppHarness, Effect},
};

/// The main screen with the built-in config, as `App` sets it up.
fn harness() -> AppHarness {
    AppHarness::new(Config::embedded())
}

#[test]
fn home_empty_snapshots() {
    let mut h = harness();
    insta::assert_snapshot!("home_empty_80x24", h.render(80, 24));
    insta::assert_snapshot!("home_empty_160x48", h.render(160, 48));
}

#[test]
fn connection_form_new_snapshot() {
    let mut h = harness();
    h.keys("ctrl-n");
    insta::assert_snapshot!("connection_form_new_80x24", h.render(80, 24));
}

#[test]
fn export_form_snapshot() {
    let mut h = harness();
    h.keys("ctrl-e");
    insta::assert_snapshot!("export_form_80x24", h.render(80, 24));
}

#[test]
fn password_prompt_snapshot() {
    let mut h = harness();
    h.send(Action::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    insta::assert_snapshot!("password_prompt_80x24", h.render(80, 24));
}

#[test]
fn help_snapshot() {
    let mut h = harness();
    h.keys("ctrl-h");
    insta::assert_snapshot!("help_80x24", h.render(80, 24));
}

#[test]
fn each_mode_opens_its_own_modal() {
    let mut h = harness();
    assert!(h.home().modal.is_none());

    h.keys("ctrl-n");
    assert!(h.render(80, 50).contains("New Connection"));

    h.keys("esc ctrl-e");
    let text = h.render(80, 50);
    assert!(
        text.contains("Export") && !text.contains("New Connection"),
        "{text}"
    );

    h.keys("esc");
    assert!(h.home().modal.is_none());
    assert!(!h.render(80, 50).contains("Export"));
}

#[test]
fn keys_reach_the_open_modal_and_it_reopens_empty() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv");
    assert!(h.render(80, 50).contains("rows.csv"));

    // Reopening builds a fresh form.
    h.keys("esc ctrl-e");
    assert!(!h.render(80, 50).contains("rows.csv"));
}

#[test]
fn a_failed_export_keeps_the_modal_and_the_path() {
    let mut h = harness();
    h.keys("ctrl-e")
        .type_str("no/such/dir/rows.csv")
        .keys("enter");

    let effects = h.take_effects();
    let [Effect::Export(config)] = effects.as_slice() else {
        panic!("expected an export, got {effects:?}");
    };
    assert_eq!(config.path, "no/such/dir/rows.csv");
    assert_eq!(h.mode(), Mode::ExpoModal, "the mode must not change yet");
    assert!(h.render(80, 50).contains("Exporting"));

    h.send(Action::ExportFailed(
        "could not write no/such/dir/rows.csv".into(),
    ));
    let text = h.render(80, 50);
    assert!(text.contains("could not write"), "{text}");
    assert!(text.contains("no/such/dir/rows.csv"), "{text}");
}

#[test]
fn an_import_waits_for_the_file_and_keeps_the_path_on_failure() {
    let mut h = harness();
    // Ctrl-o opens it from the main screen.
    h.keys("ctrl-o");
    assert_eq!(h.mode(), Mode::ImportModal);
    h.type_str("~/nope.json").keys("enter");

    let effects = h.take_effects();
    let [Effect::Import { path, .. }] = effects.as_slice() else {
        panic!("expected an import, got {effects:?}");
    };
    assert_eq!(path, "~/nope.json");
    assert!(h.render(80, 50).contains("Importing"));

    h.send(Action::ImportFailed("cannot read nope.json".into()));
    let text = h.render(80, 50);
    assert!(
        text.contains("cannot read nope.json") && text.contains("~/nope.json"),
        "{text}"
    );

    // A second try that works closes it.
    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Import { .. }]
    ));
    h.send(Action::Imported(Box::new(crate::import::Imported {
        format: crate::import::Format::Qry,
        connections: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    })));
    assert_eq!(h.mode(), Mode::Home);
}

#[test]
fn exporting_connections_waits_for_the_file() {
    let mut h = harness();
    let mut saved = StoredConnection::new("app", Driver::Sqlite);
    saved.path = Some("app.db".into());
    h.send(Action::ConnectionsLoaded(vec![saved]));
    h.keys("ctrl-s");
    assert_eq!(h.mode(), Mode::ConnExportModal);
    assert!(h.render(80, 50).contains("Export Connections"));
    h.type_str("/root/b.json").keys("enter");

    // The modal asks the tree, which holds the list, and the tree asks `App`
    // to write it.
    let effects = h.take_effects();
    let [Effect::WriteConnections { path, connections }] = effects.as_slice() else {
        panic!("expected a write, got {effects:?}");
    };
    assert_eq!(path, "/root/b.json");
    assert_eq!(connections.len(), 1);
    assert!(h.render(80, 50).contains("Exporting"));

    h.send(Action::ConnectionsExportFailed(
        "cannot write /root/b.json".into(),
    ));
    assert!(h.render(80, 50).contains("cannot write /root/b.json"));

    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::WriteConnections { .. }]
    ));
    h.send(Action::ConnectionsExported("/tmp/b.json".into()));
    assert_eq!(h.mode(), Mode::Home);
}

#[test]
fn a_written_export_closes_the_modal() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv").keys("enter");
    assert!(matches!(h.take_effects().as_slice(), [Effect::Export(_)]));

    h.send(Action::Exported("rows.csv".into()));
    assert!(h.home().modal.is_none());
    assert_eq!(h.mode(), Mode::Home);
}

#[test]
fn an_answer_meant_for_another_modal_is_ignored() {
    let mut h = harness();
    // An export is waiting, and a connection's answer arrives late.
    h.keys("ctrl-e").type_str("rows.csv").keys("enter");
    h.send(Action::Connected(":memory:".into()));
    h.send(Action::ConnectFailed("nope".into()));

    let text = h.render(80, 50);
    assert!(
        h.home().modal.is_some(),
        "the export modal was closed by a connection"
    );
    assert!(
        text.contains("Exporting") && !text.contains("nope"),
        "{text}"
    );
}

#[test]
fn a_refused_modal_stays_open_with_its_error() {
    let mut h = harness();
    h.keys("ctrl-e enter");
    assert!(h.take_effects().is_empty());
    assert!(h.render(80, 50).contains("Path is required"));
}

/// Fills in enough of the New Connection form to be submittable.
fn fill_connection(h: &mut AppHarness) {
    for (field, value) in [(2, "db.local"), (4, "alice"), (6, "app")] {
        h.home_mut().modal.as_mut().unwrap().set_focus(field);
        h.type_str(value);
    }
}

#[test]
fn a_failed_connection_keeps_the_modal_and_everything_typed() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);

    // Submitting sends the connection but leaves the modal waiting.
    h.keys("enter");
    assert!(matches!(h.take_effects().as_slice(), [Effect::Connect(..)]));
    assert_eq!(h.mode(), Mode::AddConnModal, "the mode must not change yet");
    assert!(h.render(80, 50).contains("Connecting"));

    // A second Enter while waiting does not try again.
    h.keys("enter");
    assert!(h.take_effects().is_empty());

    h.send(Action::ConnectFailed(
        "password authentication failed".into(),
    ));
    let text = h.render(80, 50);
    assert!(text.contains("password authentication failed"), "{text}");
    assert!(
        text.contains("db.local") && text.contains("alice"),
        "{text}"
    );
    assert!(!text.contains("Connecting"), "{text}");

    // Correcting and submitting again sends a second attempt.
    h.keys("x enter");
    assert!(matches!(h.take_effects().as_slice(), [Effect::Connect(..)]));
}

#[test]
fn a_working_connection_closes_the_modal() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);

    h.keys("enter");
    assert!(matches!(h.take_effects().as_slice(), [Effect::Connect(..)]));

    h.send(Action::Connected("alice@db.local:5432/app".into()));
    assert!(h.home().modal.is_none());
    assert_eq!(h.mode(), Mode::Home);
    assert!(!h.render(80, 50).contains("New Connection"));
}

#[test]
fn a_connect_answer_without_a_waiting_modal_is_ignored() {
    let mut h = harness();
    // The in-memory connection at startup answers with nobody waiting.
    h.send(Action::Connected(":memory:".into()));
    h.send(Action::ConnectFailed("nope".into()));
    assert!(h.home().modal.is_none());

    // An export modal is not touched by a connection's answer either.
    h.keys("ctrl-e");
    h.send(Action::ConnectFailed("nope".into()));
    let text = h.render(80, 50);
    assert!(text.contains("Export") && !text.contains("nope"), "{text}");
}

#[test]
fn a_request_for_a_password_opens_the_prompt() {
    let mut h = harness();
    h.send(Action::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    // It asks `App` to switch modes, so Esc closes the prompt.
    assert_eq!(h.mode(), Mode::PasswordModal);
    let text = h.render(80, 30);
    assert!(
        text.contains("Password") && text.contains("for prod"),
        "{text}"
    );

    // Submitting waits for the worker, as a connection does.
    crate::secrets::stash(crate::secrets::Password::new("x"));
    h.keys("enter");
    assert_eq!(h.take_effects(), [Action::PasswordEntered]);
    assert!(h.render(80, 30).contains("Connecting"));

    h.send(Action::Connected(":memory:".into()));
    assert!(h.home().modal.is_none());
}

#[test]
fn help_opens_scrolls_and_closes_with_configured_keys() {
    let mut h = harness();
    h.keys("ctrl-h");
    assert!(h.home().helpvisible);
    assert!(h.render(80, 20).contains("Global:"));

    // While help is open, keys go to it, not to the focused pane.
    h.home_mut().focus = Pane::Results;
    h.keys("pagedown");
    assert!(!h.render(80, 20).contains("Global:"));

    h.keys("esc");
    assert!(!h.home().helpvisible);

    h.keys("ctrl-h");
    assert!(h.render(80, 20).contains("Global:"));
    h.keys("ctrl-h");
    assert!(!h.home().helpvisible);
}

#[test]
fn update_is_forwarded_to_children() {
    let mut h = harness();
    assert!(h.home().statuspanel.is_empty());
    h.send(Action::Status(StatusCode::Error("e".into())));
    assert!(!h.home().statuspanel.is_empty());
}

#[test]
fn tree_is_focused_and_highlighted_at_startup() {
    let mut h = harness();
    assert!(h.home().focus == Pane::Tree);

    // The top-left corners of the tree and the editor.
    let focused = h.theme().focused_border().fg.unwrap();
    let buf = h.render_buffer(80, 24);
    assert_eq!(buf[(0, 0)].fg, focused);
    assert_ne!(buf[(20, 0)].fg, focused);

    h.keys("tab");
    assert!(h.home().focus == Pane::Editor);
    let buf = h.render_buffer(80, 24);
    assert_ne!(buf[(0, 0)].fg, focused);
    assert_eq!(buf[(20, 0)].fg, focused);

    h.keys("tab");
    assert!(h.home().focus == Pane::Results);
    h.keys("tab");
    assert!(h.home().focus == Pane::Tree);
    h.keys("shift-tab");
    assert!(h.home().focus == Pane::Results);
}

#[test]
fn keys_reach_the_focused_pane_when_nothing_is_on_top() {
    let mut h = harness();
    h.keys("tab").type_str("select 1").keys("f8");
    assert_eq!(h.take_effects(), [Action::Execute("select 1".into())]);
}
