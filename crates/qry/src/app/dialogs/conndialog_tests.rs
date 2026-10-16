#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{DatabaseType, testing::FakeDatabase};

use super::{test_support::*, *};
use crate::{
    app::{
        DbAnswer, Effect, SessionStatus, UiEvent,
        dialogs::form_tests::{press, render, type_str},
        event::{FileAnswer, Tested},
    },
    config::Config,
    connections::Secret,
    testing::{AppHarness, buffer_lines, buffer_to_string, styled},
};

const CANARY: &str = "CANARY-PW-7f3a";

fn set(dialog: &mut ConnDialog, key: Key, value: &str) {
    dialog.set_text(key, value);
    dialog.relayout();
}

fn choose(dialog: &mut ConnDialog, key: Key, option: &str) {
    let Some(Field::Choice { options, selected }) = dialog.field_mut(key) else {
        panic!("{key:?} is not a choice");
    };
    *selected = options.iter().position(|o| *o == option).unwrap();
    dialog.relayout();
}

fn filled() -> ConnDialog {
    let mut dialog = ConnDialog::new("c0ffee42");
    set(&mut dialog, Key::Name, " prod ");
    set(&mut dialog, Key::Conn(FieldKind::Host), "db.local");
    set(&mut dialog, Key::Conn(FieldKind::User), "alice");
    set(&mut dialog, Key::Conn(FieldKind::Password), " s3cret ");
    set(&mut dialog, Key::Conn(FieldKind::Database), "app");
    set(&mut dialog, Key::Conn(FieldKind::Schema), "reporting");
    dialog
}

/// The record the dialog submits, and the password typed beside it.
fn submitted(dialog: &ConnDialog) -> (StoredConnection, Option<String>) {
    match dialog.submit() {
        Ok(Submission::Connect { record, password }) => {
            (*record, password.map(|p| p.as_str().to_string()))
        }
        other => panic!("expected a connection, got {other:?}"),
    }
}

fn record(dialog: &ConnDialog) -> StoredConnection {
    submitted(dialog).0
}

fn rows(dialog: &ConnDialog) -> Vec<&'static str> {
    dialog.labels().to_vec()
}

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded())
        .with_database(FakeDatabase::new(DatabaseType::Postgres).version("PostgreSQL 16.4"));
    h.send(UiEvent::Started);
    h.take_effects();
    h
}

/// A harness with no fake, so nothing answers on its own.
fn quiet() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.db(DbAnswer::Connected(":memory:".into()));
    h.take_effects();
    h
}

/// A saved Postgres connection `prod`, selected in the tree.
fn saved(h: &mut AppHarness, secret: Secret) -> StoredConnection {
    let mut record = StoredConnection::with_id("p1", "prod", Driver::Postgres);
    record.host = Some("db.local".into());
    record.user = Some("alice".into());
    record.database = Some("app".into());
    record.secret = secret;
    h.file(FileAnswer::ConnectionsLoaded(vec![record.clone()]));
    let i = h.state().tree.position("p1").unwrap();
    h.state_mut().tree.selected = i;
    h.state_mut().focus = crate::app::state::Focus::Tree;
    record
}

// ---- the form itself ----------------------------------------------------

#[test]
fn the_dialog_builds_a_record_with_no_password_in_it() {
    let (built, password) = submitted(&filled());
    assert_eq!(built.name, "prod");
    assert_eq!(built.driver, Driver::Postgres);
    assert_eq!(built.host.as_deref(), Some("db.local"));
    assert_eq!(built.user.as_deref(), Some("alice"));
    assert_eq!(built.database.as_deref(), Some("app"));
    assert_eq!(built.schema.as_deref(), Some("reporting"));
    assert_eq!(built.tls.as_deref(), Some("prefer"));
    assert_eq!(
        built.port, None,
        "5432 is the default, so it is not written"
    );
    assert_eq!(built.secret, Secret::Keyring);
    let written = serde_json::to_string(&built).unwrap();
    assert!(!written.contains("s3cret"), "{written}");
    assert_eq!(password.as_deref(), Some(" s3cret "), "spaces and all");
}

#[test]
fn rows_follow_the_driver() {
    let mut dialog = ConnDialog::new("c1");
    assert_eq!(
        rows(&dialog),
        [
            "Driver",
            "Name",
            "",
            "Host",
            "Port",
            "User",
            "Password",
            "Password in",
            "Database",
            "Schema",
            "SSL",
            "Label",
            ""
        ]
    );
    choose(&mut dialog, Key::Driver, "SQLite");
    assert_eq!(
        rows(&dialog),
        ["Driver", "Name", "", "File", "Mode", "Label", ""]
    );
    choose(&mut dialog, Key::Driver, "Oracle");
    assert!(rows(&dialog).contains(&"Service/SID"));
    assert!(rows(&dialog).contains(&"Wallet dir"), "SSL prefer is on");
    choose(&mut dialog, Key::Conn(FieldKind::Ssl), "disable");
    assert!(!rows(&dialog).contains(&"Wallet dir"));
}

#[test]
fn driver_change_keeps_shared_values_and_moves_an_untouched_port() {
    let mut dialog = filled();
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::Port)), "5432");
    choose(&mut dialog, Key::Driver, "MySQL");
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::Host)), "db.local");
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::Port)), "3306");
    set(&mut dialog, Key::Conn(FieldKind::Port), "13306");
    choose(&mut dialog, Key::Driver, "PostgreSQL");
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::Port)), "13306");
    assert_eq!(dialog.subtitle().as_deref(), Some("postgres"));
}

#[test]
fn a_dsn_replaces_the_fields_and_turning_it_off_brings_them_back() {
    let mut dialog = filled();
    dialog.set_checked(Key::UseDsn, true);
    assert!(rows(&dialog).contains(&"DSN"));
    assert!(!rows(&dialog).contains(&"Host"));
    set(
        &mut dialog,
        Key::Conn(FieldKind::Dsn),
        "postgres://alice@db.local/app",
    );
    let built = record(&dialog);
    assert_eq!(built.dsn.as_deref(), Some("postgres://alice@db.local/app"));
    assert_eq!(built.host, None);

    dialog.set_checked(Key::UseDsn, false);
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::Host)), "db.local");
}

#[test]
fn environment_swaps_the_password_for_a_variable_named_after_the_connection() {
    let mut dialog = ConnDialog::new("c1");
    set(&mut dialog, Key::Name, "local-app");
    choose(&mut dialog, Key::Store, "environment");
    assert!(rows(&dialog).contains(&"Variable"));
    assert!(!rows(&dialog).contains(&"Password"));
    assert_eq!(
        dialog.text_of(Key::Conn(FieldKind::EnvVar)),
        "QRY_PW_LOCAL_APP"
    );
    // It follows the name while it is still the default.
    set(&mut dialog, Key::Name, "app2");
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::EnvVar)), "QRY_PW_APP2");
    set(&mut dialog, Key::Conn(FieldKind::EnvVar), "PGPASSWORD");
    set(&mut dialog, Key::Name, "app3");
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::EnvVar)), "PGPASSWORD");
}

#[test]
fn validation_names_the_field_and_moves_to_it() {
    let mut dialog = ConnDialog::new("c1");
    dialog.set_taken(vec!["prod".into()]);
    set(&mut dialog, Key::Name, "Prod");
    set(&mut dialog, Key::Conn(FieldKind::Port), "70000");
    assert!(!dialog.check());
    assert_eq!(
        dialog.error(),
        Some("Name: \"Prod\" is already a saved connection")
    );
    assert_eq!(dialog.focus(), dialog.position(Key::Name).unwrap());
    let port = dialog.position(Key::Conn(FieldKind::Port)).unwrap();
    assert_eq!(
        dialog.field_error(port),
        Some("must be a number from 1 to 65535")
    );
    assert!(dialog.submit().is_err());

    // Typing in a field takes its error away.
    dialog.set_focus(port);
    press(
        &mut dialog,
        crate::app::dialogs::form_tests::key(crossterm::event::KeyCode::Backspace),
    );
    assert_eq!(dialog.field_error(port), None);
}

#[test]
fn the_id_survives_a_retry() {
    let dialog = filled();
    assert_eq!(record(&dialog).id, record(&dialog).id);
}

#[test]
fn editing_opens_filled_in_and_keeps_id_password_and_unknown_keys() {
    let mut saved = record(&filled());
    saved.port = Some(6543);
    saved.secret = Secret::Keyring;
    saved.label = Some(ConnLabel::Staging);
    saved
        .extra
        .insert("from_the_future".into(), serde_json::json!(1));

    let dialog = ConnDialog::editing(&saved);
    assert_eq!(dialog.title(), "Edit Connection");
    assert_eq!(dialog.text_of(Key::Name), "prod");
    assert_eq!(dialog.text_of(Key::Conn(FieldKind::Port)), "6543");
    assert_eq!(dialog.choice_of(Key::Label), 2);
    let shown = buffer_lines(&render(&dialog, 100, 40)).join("\n");
    assert!(shown.contains("(unchanged)"), "{shown}");

    let (edited, password) = submitted(&dialog);
    assert_eq!(password, None);
    assert_eq!(edited.id, saved.id);
    assert_eq!(edited.secret, Secret::Keyring);
    assert_eq!(edited.port, Some(6543));
    assert_eq!(edited.label, Some(ConnLabel::Staging));
    assert_eq!(edited.extra, saved.extra);
}

#[test]
fn oracle_round_trips_ssl_and_wallet_and_reads_old_records() {
    let mut dialog = filled();
    choose(&mut dialog, Key::Driver, "Oracle");
    set(&mut dialog, Key::Conn(FieldKind::Service), "ORCLPDB1");
    choose(&mut dialog, Key::Conn(FieldKind::Ssl), "verify-full");
    set(
        &mut dialog,
        Key::Conn(FieldKind::Wallet),
        " /etc/oracle/wallet ",
    );
    let built = record(&dialog);
    assert_eq!(built.service.as_deref(), Some("ORCLPDB1"));
    assert_eq!(built.tls.as_deref(), Some("verify-full"));
    assert_eq!(built.wallet.as_deref(), Some("/etc/oracle/wallet"));

    let mut old = built.clone();
    old.tls = Some("/etc/oracle/wallet".into());
    old.wallet = None;
    let again = record(&ConnDialog::editing(&old));
    assert_eq!(again.tls.as_deref(), Some("verify-full"));
    assert_eq!(again.wallet.as_deref(), Some("/etc/oracle/wallet"));
}

#[test]
fn sqlite_needs_a_file_and_no_password() {
    let mut dialog = ConnDialog::new("c1");
    choose(&mut dialog, Key::Driver, "SQLite");
    set(&mut dialog, Key::Name, "local");
    assert!(!dialog.check());
    assert_eq!(dialog.error(), Some("File: a file is required"));
    set(&mut dialog, Key::Conn(FieldKind::Path), "~/app.db");
    choose(&mut dialog, Key::Conn(FieldKind::Mode), "read-only");
    let built = record(&dialog);
    assert_eq!(built.path.as_deref(), Some("~/app.db"));
    assert_eq!(built.mode.as_deref(), Some("ro"));
    assert_eq!(built.secret, Secret::None);
    assert_eq!(built.tls, None);
}

#[test]
fn typing_reaches_the_focused_field_and_the_password_is_masked() {
    let mut dialog = ConnDialog::new("c1");
    type_str(&mut dialog, "prod");
    dialog.set_focus(dialog.position(Key::Conn(FieldKind::Password)).unwrap());
    type_str(&mut dialog, "hunter2");
    assert_eq!(dialog.text_of(Key::Name), "prod");
    let shown = buffer_lines(&render(&dialog, 100, 40)).join("\n");
    assert!(shown.contains("•••••••"), "{shown}");
    assert!(!shown.contains("hunter2"), "{shown}");
}

// ---- guarantees, through the app --------------------------------------

#[test]
fn password_never_in_emitted_actions() {
    let mut h = quiet();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    type_into(&mut h, Key::Conn(FieldKind::Password), CANARY);
    h.keys("ctrl-t");
    h.keys("enter");
    h.db(DbAnswer::ConnectFailed("nope".into()));
    h.keys("enter");
    let effects = h.take_effects();
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::StashPassword { .. })),
        "{effects:?}"
    );
    for effect in &effects {
        let shown = format!("{effect:?}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
    let state = format!("{:?}", h.state());
    assert!(!state.contains(CANARY), "the state's Debug leaks it");
}

#[test]
fn id_minted_once_across_retries() {
    let mut h = quiet();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    h.keys("enter");
    h.db(DbAnswer::ConnectFailed("refused".into()));
    h.keys("enter");
    let ids: Vec<String> = h
        .take_effects()
        .iter()
        .filter_map(|e| match e {
            Effect::OpenSession(_, record) => Some(record.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(ids[0], ids[1]);
}

#[test]
fn failed_connect_keeps_fields() {
    let mut h = quiet();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    h.keys("enter");
    h.db(DbAnswer::ConnectFailed(
        "password authentication failed".into(),
    ));
    assert_eq!(h.dialog_title(), Some("New Connection"));
    let shown = h.render(120, 40);
    assert!(shown.contains("password authentication failed"), "{shown}");
    assert!(
        shown.contains("db.local") && shown.contains("alice"),
        "{shown}"
    );
}

#[test]
fn secret_not_stored_falls_back_to_prompt_with_toast() {
    let mut h = harness();
    let record = saved(&mut h, Secret::Keyring);
    h.db(DbAnswer::SecretNotStored(record.id.clone()));
    let node = &h.state().tree.connections[h.state().tree.position("p1").unwrap()];
    assert_eq!(node.record.secret, Secret::Prompt);
    let effects = h.take_effects();
    assert!(
        effects.iter().any(
            |e| matches!(e, Effect::SaveConnections(list) if list[0].secret == Secret::Prompt)
        ),
        "{effects:?}"
    );
    let (_, notice) = h.state().messages.newest_first().next().unwrap();
    assert_eq!(notice.text, "keychain unavailable; qry will ask each time");
    assert_eq!(notice.level, crate::notice::Level::Info);
}

#[test]
fn switch_keychain_to_prompt_forgets_secret() {
    let mut h = quiet();
    saved(&mut h, Secret::Keyring);
    h.keys("e");
    assert_eq!(h.dialog_title(), Some("Edit Connection"));
    let at = dialog(&mut h).position(Key::Store).unwrap();
    dialog(&mut h).set_focus(at);
    h.keys("right");
    h.keys("enter");
    let effects = h.take_effects();
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::ForgetSecret(id) if id == "p1")),
        "{effects:?}"
    );
}

#[test]
fn ctrl_t_emits_test_effect_not_connect() {
    let mut h = quiet();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    type_into(&mut h, Key::Conn(FieldKind::Password), "pw");
    h.keys("ctrl-t");
    let effects = h.take_effects();
    let [
        Effect::StashPassword { connection, .. },
        Effect::TestConnection { test, record },
    ] = effects.as_slice()
    else {
        panic!("expected a stash and a test, got {effects:?}");
    };
    assert_eq!(connection, &format!("test:{}", record.id));
    assert_eq!(record.host.as_deref(), Some("db.local"));
    assert!(
        h.state().tree.position(&record.id).is_none(),
        "not in the tree"
    );
    assert_eq!(h.sessions().len(), 1, "no session opened");
    assert!(h.render(120, 40).contains("testing the connection"));

    h.send(UiEvent::ConnectionTested {
        test: *test,
        outcome: Err("connection refused".into()),
    });
    assert!(h.render(120, 40).contains("✗ connection refused"));
}

#[test]
fn a_test_with_nothing_to_go_on_says_so_without_trying() {
    let mut h = quiet();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    let at = dialog(&mut h).position(Key::Store).unwrap();
    dialog(&mut h).set_focus(at);
    h.keys("right");
    h.keys("ctrl-t");
    assert!(h.take_effects().is_empty());
    assert!(h.render(120, 40).contains("type the password"));
}

#[test]
fn enter_on_the_test_button_tests() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    type_into(&mut h, Key::Conn(FieldKind::Password), "pw");
    let at = dialog(&mut h).position(Key::Test).unwrap();
    dialog(&mut h).set_focus(at);
    h.keys("enter");
    assert!(
        h.render(120, 40)
            .contains("✓ connected in 42 ms · PostgreSQL 16.4")
    );
    assert_eq!(h.dialog_title(), Some("New Connection"), "not saved");
}

#[test]
fn late_test_answer_after_cancel_ignored() {
    let mut h = quiet();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    type_into(&mut h, Key::Conn(FieldKind::Password), "pw");
    h.keys("ctrl-t esc ctrl-n");
    let before = h.state().clone();
    h.send(UiEvent::ConnectionTested {
        test: 1,
        outcome: Ok(Tested {
            millis: 1,
            version: "late".into(),
        }),
    });
    let Some(crate::app::dialogs::DialogState::Connection(now)) = h.state().dialog() else {
        panic!("the new dialog is open");
    };
    assert_eq!(now.test, TestState::Idle);
    assert_eq!(h.state().dialog(), before.dialog());
}

#[test]
fn prod_label_confirms_first_write_once() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill(&mut h, "prod");
    let at = dialog(&mut h).position(Key::Label).unwrap();
    dialog(&mut h).set_focus(at);
    h.keys("left");
    h.keys("enter");
    assert_eq!(h.dialog_title(), None, "connected");
    assert_eq!(h.state().sessions.active().label, Some(ConnLabel::Prod));
    h.take_effects();

    // A read runs straight away.
    h.state_mut().focus = crate::app::state::Focus::Editor;
    h.type_str("SELECT 1");
    h.keys("f5");
    assert!(h.state().dialog().is_none());
    h.take_effects();

    // The first write asks; Esc drops it.
    h.state_mut().editor_mut().textarea = ratatui_textarea::TextArea::default();
    h.type_str("DELETE FROM t");
    h.keys("f5");
    assert_eq!(h.dialog_title(), Some("Write to Production"));
    assert!(!h.take_effects().iter().any(|e| matches!(e, Effect::Db(..))));
    h.keys("esc");
    assert!(h.take_effects().is_empty());

    // Asked again, and this time run.
    h.keys("f5");
    assert_eq!(h.dialog_title(), Some("Write to Production"));
    h.keys("enter");
    assert!(h.take_effects().iter().any(
        |e| matches!(e, Effect::Db(_, crate::db::DbCommand::Query(sql)) if sql == "DELETE FROM t")
    ));

    // Not again in this session.
    h.keys("f5");
    assert_eq!(h.dialog_title(), None);
}

#[test]
fn prod_writes_run_without_asking_when_configured_off() {
    let config =
        Config::with_user_file("config.json", r#"{ "confirm_writes_on_prod": false }"#).unwrap();
    let mut h = AppHarness::new(config).with_database(FakeDatabase::new(DatabaseType::Postgres));
    h.send(UiEvent::Started);
    let mut record = StoredConnection::with_id("p1", "prod", Driver::Postgres);
    record.label = Some(ConnLabel::Prod);
    h.file(FileAnswer::ConnectionsLoaded(vec![record]));
    h.state_mut().tree.selected = 1;
    h.keys("enter");
    h.state_mut().focus = crate::app::state::Focus::Editor;
    h.type_str("DELETE FROM t");
    h.keys("f5");
    assert_eq!(h.dialog_title(), None);
}

#[test]
fn duplicate_gets_new_id_and_prompt_secret() {
    let mut h = quiet();
    let original = saved(&mut h, Secret::Keyring);
    h.keys("c");
    assert_eq!(h.dialog_title(), Some("Duplicate Connection"));
    assert_eq!(dialog(&mut h).text_of(Key::Name), "prod (copy)");
    h.keys("enter");
    let effects = h.take_effects();
    let Some(Effect::OpenSession(_, copy)) = effects
        .iter()
        .find(|e| matches!(e, Effect::OpenSession(..)))
    else {
        panic!("{effects:?}");
    };
    assert_ne!(copy.id, original.id);
    assert_eq!(copy.secret, Secret::Prompt);
    assert_eq!(copy.host, original.host);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::StashPassword { .. })),
        "no password copied"
    );
}

#[test]
fn harness_flow_tests_then_saves_and_connects() {
    let mut h = harness();
    h.keys("ctrl-n");
    h.type_str("local-app");
    h.keys("tab tab");
    h.type_str("localhost");
    h.keys("tab tab");
    h.type_str("app");
    h.keys("tab");
    h.type_str("pw");
    h.keys("tab tab");
    h.type_str("app");
    h.keys("ctrl-t");
    assert!(
        h.render(120, 40)
            .contains("✓ connected in 42 ms · PostgreSQL 16.4")
    );
    h.keys("enter");
    assert_eq!(h.dialog_title(), None);
    let tree = &h.state().tree;
    let i = tree
        .connections
        .iter()
        .position(|n| n.record.name == "local-app")
        .unwrap();
    let id = tree.connections[i].record.id.clone();
    assert!(matches!(
        h.state().sessions.connection_status(&id),
        Some(SessionStatus::Open { .. })
    ));
    // Its row shows the open state and the driver's badge.
    let text = h.render(120, 40);
    assert!(
        text.lines()
            .any(|l| l.contains("● ") && l.contains("local-app")),
        "{text}"
    );
}

// ---- snapshots ----------------------------------------------------------

fn snap(name: &str, dialog: &ConnDialog) {
    let buffer = render(dialog, 100, 30);
    insta::assert_snapshot!(name, buffer_to_string(&buffer));
    insta::assert_snapshot!(format!("{name}_styled"), styled(&buffer));
}

#[test]
fn conndialog_snapshots() {
    let mut postgres = filled();
    set(&mut postgres, Key::Conn(FieldKind::Password), "");
    snap("conndialog_postgres_100x30", &postgres);

    let mut sqlite = ConnDialog::new("c1");
    choose(&mut sqlite, Key::Driver, "SQLite");
    set(&mut sqlite, Key::Name, "local");
    set(&mut sqlite, Key::Conn(FieldKind::Path), "~/app.db");
    snap("conndialog_sqlite_100x30", &sqlite);

    let mut oracle = filled();
    choose(&mut oracle, Key::Driver, "Oracle");
    set(&mut oracle, Key::Conn(FieldKind::Service), "ORCLPDB1");
    choose(&mut oracle, Key::Conn(FieldKind::Ssl), "verify-full");
    set(
        &mut oracle,
        Key::Conn(FieldKind::Wallet),
        "/etc/oracle/wallet",
    );
    snap("conndialog_oracle_tls_100x30", &oracle);

    let mut env = filled();
    choose(&mut env, Key::Store, "environment");
    snap("conndialog_env_password_100x30", &env);

    let mut invalid = ConnDialog::new("c1");
    set(&mut invalid, Key::Conn(FieldKind::Port), "0");
    invalid.check();
    snap("conndialog_validation_errors_100x30", &invalid);

    let mut ok = filled();
    ok.test = TestState::Passed {
        millis: 42,
        version: "PostgreSQL 16.4".into(),
    };
    snap("conndialog_test_ok_100x30", &ok);

    let mut failed = filled();
    failed.test = TestState::Failed("password authentication failed for user \"alice\"".into());
    snap("conndialog_test_failed_100x30", &failed);
}

#[test]
fn tree_and_tab_prod_label_120x40() {
    let mut h = harness();
    let labelled = |id: &str, name: &str, label: ConnLabel| {
        let mut record = StoredConnection::with_id(id, name, Driver::Sqlite);
        record.path = Some(format!("{name}.db"));
        record.label = Some(label);
        record
    };
    h.file(FileAnswer::ConnectionsLoaded(vec![
        labelled("p1", "prod-db", ConnLabel::Prod),
        labelled("s1", "staging-db", ConnLabel::Staging),
        labelled("d1", "dev-db", ConnLabel::Dev),
    ]));
    h.state_mut().tree.selected = 1;
    h.keys("enter");
    let buffer = h.render_buffer(120, 40);
    let text = buffer_to_string(&buffer);
    assert!(text.contains("prod-db [prod]"), "{text}");
    assert!(text.contains("2 prod-db [prod]"), "{text}");
    insta::assert_snapshot!("tree_and_tab_prod_label_120x40", text);
    insta::assert_snapshot!("tree_and_tab_prod_label_120x40_styled", styled(&buffer));
}
