//! Unit tests for the reducer, through [`AppHarness`]: keys go through the
//! keymap as they do in the app, and the I/O the runtime would carry out
//! shows up as effects.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{state::Focus, *};
use crate::{
    config::KeySection,
    connections::{Driver, StoredConnection},
    db::DbCommand,
    import::{Format, Imported},
    testing::AppHarness,
};

/// The main screen with the built-in config, as the runtime sets it up.
fn harness() -> AppHarness {
    AppHarness::new(Config::embedded())
}

#[test]
fn startup_opens_scratch_and_loads_connections() {
    let mut h = harness();
    h.send(UiEvent::Started);
    let effects = h.take_effects();
    let [Effect::LoadConnections, Effect::OpenSession(id, scratch)] = effects.as_slice() else {
        panic!("expected a load and the scratch session, got {effects:?}");
    };
    assert_eq!(*id, SessionId::FIRST);
    assert_eq!(scratch.path.as_deref(), Some(":memory:"));
    assert_eq!(scratch.driver, Driver::Sqlite);
    assert!(scratch.ephemeral);
    assert!(h.render(80, 24).contains("scratch"));

    // Once it opens, it stays in the tree but is never written.
    h.db(DbAnswer::Connected(":memory:".into()));
    assert!(
        !h.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::SaveConnections(_))),
        "the scratch database is never saved"
    );
    let mut saved = StoredConnection::new("app", Driver::Sqlite);
    saved.path = Some("app.db".into());
    h.file(FileAnswer::ConnectionsLoaded(vec![saved]));
    let text = h.render(80, 24);
    assert!(text.contains("scratch") && text.contains("app"), "{text}");
}

#[test]
fn each_mode_opens_its_own_modal() {
    let mut h = harness();
    assert!(h.state().dialog().is_none());

    h.keys("ctrl-n");
    assert!(h.render(80, 50).contains("New Connection"));

    h.keys("esc ctrl-e");
    let text = h.render(80, 50);
    assert!(
        text.contains("Export") && !text.contains("New Connection"),
        "{text}"
    );

    h.keys("esc");
    assert!(h.state().dialog().is_none());
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
    let [Effect::Db(_, DbCommand::Export(config))] = effects.as_slice() else {
        panic!("expected an export, got {effects:?}");
    };
    assert_eq!(config.path, "no/such/dir/rows.csv");
    assert_eq!(h.section(), KeySection::ExpoModal, "still open");
    assert!(h.render(80, 50).contains("Exporting"));

    h.db(DbAnswer::ExportFailed(
        "could not write no/such/dir/rows.csv".into(),
    ));
    let text = h.render(80, 50);
    assert!(text.contains("could not write"), "{text}");
    assert!(text.contains("no/such/dir/rows.csv"), "{text}");
}

#[test]
fn import_waits_and_keeps_path_on_failure() {
    let mut h = harness();
    // Ctrl-o opens it from the main screen.
    h.keys("ctrl-o");
    assert_eq!(h.section(), KeySection::ImportModal);
    h.type_str("~/nope.json").keys("enter");

    let effects = h.take_effects();
    let [Effect::Import { path, .. }] = effects.as_slice() else {
        panic!("expected an import, got {effects:?}");
    };
    assert_eq!(path, "~/nope.json");
    assert!(h.render(80, 50).contains("Importing"));

    h.file(FileAnswer::ImportFailed("cannot read nope.json".into()));
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
    h.file(FileAnswer::Imported(Box::new(Imported {
        format: Format::Qry,
        connections: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    })));
    assert_eq!(h.section(), KeySection::Home);
}

#[test]
fn exporting_connections_waits_for_the_file() {
    let mut h = harness();
    let mut saved = StoredConnection::new("app", Driver::Sqlite);
    saved.path = Some("app.db".into());
    h.file(FileAnswer::ConnectionsLoaded(vec![saved]));
    h.keys("ctrl-s");
    assert_eq!(h.section(), KeySection::ConnExportModal);
    assert!(h.render(80, 50).contains("Export Connections"));
    h.type_str("/root/b.json").keys("enter");

    // The tree holds the list, and the runtime writes it.
    let effects = h.take_effects();
    let [Effect::ExportConnections { path, connections }] = effects.as_slice() else {
        panic!("expected a write, got {effects:?}");
    };
    assert_eq!(path, "/root/b.json");
    assert_eq!(connections.len(), 1);
    assert!(h.render(80, 50).contains("Exporting"));

    h.file(FileAnswer::ConnectionsExportFailed(
        "cannot write /root/b.json".into(),
    ));
    assert!(h.render(80, 50).contains("cannot write /root/b.json"));

    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::ExportConnections { .. }]
    ));
    h.file(FileAnswer::ConnectionsExported {
        path: "/tmp/b.json".into(),
        count: 1,
    });
    assert_eq!(h.section(), KeySection::Home);
    assert!(
        h.render(80, 24)
            .contains("Exported 1 connection to /tmp/b.json")
    );
}

#[test]
fn exporting_no_saved_connections_is_refused_in_the_modal() {
    let mut h = harness();
    h.keys("ctrl-s").type_str("/tmp/b.json").keys("enter");
    assert!(h.take_effects().is_empty());
    let text = h.render(80, 50);
    assert!(
        text.contains("There are no saved connections to export"),
        "{text}"
    );
    assert_eq!(h.section(), KeySection::ConnExportModal);
}

#[test]
fn a_written_export_closes_the_modal() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv").keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Db(_, DbCommand::Export(_))]
    ));

    h.db(DbAnswer::Exported("rows.csv".into()));
    assert!(h.state().dialog().is_none());
    assert_eq!(h.section(), KeySection::Home);
}

#[test]
fn late_answer_for_closed_modal_is_ignored() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv").keys("enter esc");
    assert!(h.state().dialog().is_none());

    let before = h.state().clone();
    h.db(DbAnswer::Exported("rows.csv".into()));
    h.db(DbAnswer::ExportFailed("too late".into()));
    let mut after = h.state().clone();
    after.dirty = before.dirty;
    assert_eq!(after, before, "a late answer changed the state");
    assert!(h.state().overlays.is_empty());
}

#[test]
fn answer_for_other_modal_is_ignored() {
    let mut h = harness();
    // An export is waiting, and a connection's answer arrives late.
    h.keys("ctrl-e").type_str("rows.csv").keys("enter");
    h.db(DbAnswer::Connected(":memory:".into()));
    h.db(DbAnswer::ConnectFailed("nope".into()));

    let text = h.render(80, 50);
    assert!(
        h.state().dialog().is_some(),
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
        let dialog = h.state_mut().dialog_mut().unwrap();
        dialog.form_mut().set_focus(field);
        h.type_str(value);
    }
}

#[test]
fn connect_modal_waits_for_answer() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    let focus = h.state().focus;

    // Submitting sends the connection but leaves the modal waiting.
    h.keys("enter");
    let effects = h.take_effects();
    let [Effect::Db(_, DbCommand::Connect(record))] = effects.as_slice() else {
        panic!("expected a connection and nothing else, got {effects:?}");
    };
    assert_eq!(record.host.as_deref(), Some("db.local"));
    assert_eq!(h.section(), KeySection::AddConnModal, "still open");
    assert_eq!(h.state().focus, focus);
    assert!(h.render(80, 50).contains("Connecting"));
}

#[test]
fn second_submit_while_awaiting_is_ignored() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    h.keys("enter");
    assert_eq!(h.take_effects().len(), 1);

    // A second Enter while waiting does not try again.
    h.keys("enter");
    assert!(h.take_effects().is_empty());
}

#[test]
fn connect_failure_keeps_fields_and_shows_error() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    h.keys("enter");
    h.take_effects();

    h.db(DbAnswer::ConnectFailed(
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
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Db(_, DbCommand::Connect(..))]
    ));
}

#[test]
fn a_working_connection_closes_the_modal() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);

    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Db(_, DbCommand::Connect(..))]
    ));

    h.db(DbAnswer::Connected("alice@db.local:5432/app".into()));
    assert!(h.state().dialog().is_none());
    assert_eq!(h.section(), KeySection::Home);
    assert!(!h.render(80, 50).contains("New Connection"));
}

#[test]
fn a_connect_answer_without_a_waiting_modal_is_ignored() {
    let mut h = harness();
    // The in-memory connection at startup answers with nobody waiting.
    h.db(DbAnswer::Connected(":memory:".into()));
    h.db(DbAnswer::ConnectFailed("nope".into()));
    assert!(h.state().dialog().is_none());

    // An export modal is not touched by a connection's answer either.
    h.keys("ctrl-e");
    h.db(DbAnswer::ConnectFailed("nope".into()));
    let text = h.render(80, 50);
    assert!(text.contains("Export") && !text.contains("nope"), "{text}");
}

#[test]
fn need_password_opens_prompt_and_submit_awaits_connect() {
    let mut h = harness();
    h.db(DbAnswer::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    // It is the password modal's keys now, so Esc closes the prompt.
    assert_eq!(h.section(), KeySection::PasswordModal);
    let text = h.render(80, 30);
    assert!(
        text.contains("Password") && text.contains("for prod"),
        "{text}"
    );

    // Submitting leaves the password for the worker and waits for it, as a
    // connection does.
    h.type_str("x").keys("enter");
    let effects = h.take_effects();
    let [
        Effect::StashPassword(password),
        Effect::Db(_, DbCommand::PasswordEntered),
    ] = effects.as_slice()
    else {
        panic!("expected the stash and then the worker, got {effects:?}");
    };
    assert_eq!(password.as_str(), "x");
    assert!(h.render(80, 30).contains("Connecting"));

    h.db(DbAnswer::Connected(":memory:".into()));
    assert!(h.state().dialog().is_none());
}

#[test]
fn password_text_never_in_effects() {
    const CANARY: &str = "CANARY-PW-x1";
    let mut h = harness();
    h.db(DbAnswer::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    h.type_str(CANARY).keys("enter");
    let effects = h.take_effects();
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::StashPassword(_)))
    );
    for effect in &effects {
        let shown = format!("{effect:?}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
    let state = format!("{:?}", h.state());
    assert!(!state.contains(CANARY), "the state printed it: {state}");

    // The same in the New Connection form's password field.
    h.keys("esc ctrl-n");
    fill_connection(&mut h);
    h.state_mut().dialog_mut().unwrap().form_mut().set_focus(5);
    h.type_str(CANARY).keys("enter");
    let effects = h.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::StashPassword(_),
            Effect::Db(_, DbCommand::Connect(_))
        ]
    ));
    for effect in &effects {
        let shown = format!("{effect:?}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
}

#[test]
fn help_opens_scrolls_and_closes_with_configured_keys() {
    let mut h = harness();
    h.keys("ctrl-h");
    assert!(h.state().help().is_some());
    assert!(h.render(80, 20).contains("Global:"));

    // While help is open, keys go to it, not to the focused pane.
    h.state_mut().focus = Focus::Results;
    h.keys("pagedown");
    assert!(!h.render(80, 20).contains("Global:"));

    h.keys("esc");
    assert!(h.state().help().is_none());

    h.keys("ctrl-h");
    assert!(h.render(80, 20).contains("Global:"));
    h.keys("ctrl-h");
    assert!(h.state().help().is_none());
}

#[test]
fn a_status_answer_reaches_the_status_bar() {
    let mut h = harness();
    assert_eq!(h.state().status, StatusCode::None);
    h.db(DbAnswer::Status(StatusCode::Error("e".into())));
    assert_eq!(h.state().status, StatusCode::Error("e".into()));
    assert!(h.render(80, 24).contains("Error: e"));
}

#[test]
fn a_stopped_worker_is_reported_in_the_status_bar() {
    let mut h = harness();
    h.db(DbAnswer::WorkerStopped);
    assert!(h.render(80, 24).contains("database worker has stopped"));
}

#[test]
fn keys_reach_the_focused_pane_when_nothing_is_on_top() {
    let mut h = harness();
    h.keys("tab").type_str("select 1").keys("f8");
    assert_eq!(
        h.take_effects(),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 1".into())
        )]
    );
}

#[test]
fn query_done_is_never_printed_in_full() {
    let mut h = harness();
    let answer = DbAnswer::QueryDone(std::sync::Arc::new(crate::testing::grid(2, 2)));
    let shown = format!("{answer:?}");
    assert!(!shown.contains("r1c1"), "{shown}");
    h.db(answer);
    assert!(h.render(80, 24).contains("r1c1"), "but it is shown");
}

#[test]
fn quit_and_suspend_are_effects() {
    let mut h = harness();
    h.keys("ctrl-q");
    assert_eq!(h.take_effects(), [Effect::Quit]);
}
