//! Unit tests for `connform.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::KeyCode;

use super::*;
use crate::{
    app::dialogs::form_tests::{key, press, render, type_str},
    secrets::Password,
    testing::buffer_lines,
};

fn set(form: &mut ConnForm, field: usize, value: &str) {
    match &mut form.fields[field] {
        Field::Text(input) => {
            input.value = value.into();
            input.cursor = input.len();
        }
        Field::Secret(input) => {
            input.value = Password::new(value);
            input.cursor = input.len();
        }
        Field::Choice { .. } | Field::Checkbox { .. } => {
            panic!("{} is not a text field", LABELS[field])
        }
    }
}

fn choose(form: &mut ConnForm, field: usize, option: &str) {
    let Field::Choice { options, selected } = &mut form.fields[field] else {
        panic!("{} is not a choice", LABELS[field]);
    };
    *selected = options.iter().position(|o| *o == option).unwrap();
}

fn filled() -> ConnForm {
    let mut form = ConnForm::new("c0ffee42");
    set(&mut form, NAME, " prod ");
    set(&mut form, HOST, "db.local");
    set(&mut form, USER, "alice");
    set(&mut form, PASSWORD, " s3cret ");
    set(&mut form, DATABASE, "app");
    set(&mut form, SCHEMA, "reporting");
    form
}

/// The record the form submits, and the password typed beside it.
fn submitted(form: &ConnForm) -> (StoredConnection, Option<String>) {
    match form.submit() {
        Ok(Submission::Connect { record, password }) => {
            (*record, password.map(|p| p.as_str().to_string()))
        }
        other => panic!("expected a connection, got {other:?}"),
    }
}

fn record(form: &ConnForm) -> StoredConnection {
    submitted(form).0
}

#[test]
fn the_form_builds_a_record_with_no_password_in_it() {
    let (built, password) = submitted(&filled());

    assert_eq!(built.name, "prod");
    assert_eq!(built.driver, Driver::Postgres);
    assert_eq!(built.host.as_deref(), Some("db.local"));
    assert_eq!(built.user.as_deref(), Some("alice"));
    assert_eq!(built.database.as_deref(), Some("app"));
    assert_eq!(built.schema.as_deref(), Some("reporting"));
    assert_eq!(built.tls.as_deref(), Some("prefer"));
    assert_eq!(built.port, None, "left to the driver's default");
    assert_eq!(built.secret, Secret::Keyring);

    // The record can be written without leaking anything.
    let written = serde_json::to_string(&built).unwrap();
    assert!(!written.contains("s3cret"), "{written}");

    // The typed password rides beside it instead, spaces and all.
    assert_eq!(password.as_deref(), Some(" s3cret "));
}

#[test]
fn the_id_survives_a_retry() {
    let form = filled();
    assert_eq!(record(&form).id, record(&form).id);
}

#[test]
fn where_the_password_lives_follows_the_choice() {
    let mut form = filled();
    choose(&mut form, STORE, "ASK EACH TIME");
    assert_eq!(record(&form).secret, Secret::Prompt);

    // Nothing typed: there is nothing to keep.
    let mut form = filled();
    set(&mut form, PASSWORD, "");
    choose(&mut form, STORE, "KEYCHAIN");
    assert_eq!(record(&form).secret, Secret::Prompt);

    // SQLite needs no password at all.
    let mut form = ConnForm::new("c0ffee42");
    choose(&mut form, TYPE, "SQLite");
    set(&mut form, DATABASE, "app.db");
    let built = record(&form);
    assert_eq!(built.secret, Secret::None);
    assert_eq!(
        built.path.as_deref(),
        Some("app.db"),
        "the file, not a database name"
    );
    assert_eq!(built.tls, None);
}

#[test]
fn an_environment_variable_needs_its_name() {
    let mut form = filled();
    choose(&mut form, STORE, "ENVIRONMENT");
    form.after_change();
    assert_eq!(form.labels(), LABELS_VAR);
    assert_eq!(form.submit().unwrap_err(), "Variable is required");

    set(&mut form, VAR, "PGPASSWORD");
    assert_eq!(
        record(&form).secret,
        Secret::Env {
            var: "PGPASSWORD".into()
        }
    );

    // Choosing another store takes the field away again.
    choose(&mut form, STORE, "KEYCHAIN");
    form.after_change();
    assert_eq!(form.labels(), LABELS);
}

#[test]
fn oracle_stores_the_database_field_as_a_service() {
    let mut form = filled();
    choose(&mut form, TYPE, "Oracle");
    let built = record(&form);
    assert_eq!(built.service.as_deref(), Some("app"));
    assert_eq!(built.database, None);
}

#[test]
fn oracle_takes_an_ssl_mode_and_a_wallet() {
    let mut form = filled();
    choose(&mut form, TYPE, "Oracle");
    choose(&mut form, SSL_MODE, "verify-full");
    set(&mut form, WALLET, " /etc/oracle/wallet ");
    let built = record(&form);
    assert_eq!(built.tls.as_deref(), Some("verify-full"));
    assert_eq!(built.wallet.as_deref(), Some("/etc/oracle/wallet"));

    // And opening it again shows both.
    let form = ConnForm::editing(&built);
    assert_eq!(
        form.selected(SSL_MODE),
        SSL_MODES.iter().position(|m| *m == "verify-full").unwrap()
    );
    assert_eq!(form.text(WALLET), "/etc/oracle/wallet");
}

#[test]
fn a_wallet_is_refused_for_anything_but_oracle() {
    let mut form = filled();
    set(&mut form, WALLET, "/etc/oracle/wallet");
    assert_eq!(form.submit().unwrap_err(), "Wallet is only used by Oracle");
}

#[test]
fn editing_an_old_oracle_record_moves_its_wallet_out_of_tls() {
    let mut saved = StoredConnection::new("erp", Driver::Oracle);
    saved.host = Some("db.local".into());
    saved.user = Some("alice".into());
    saved.service = Some("ORCLPDB1".into());
    saved.tls = Some("/etc/oracle/wallet".into());

    let form = ConnForm::editing(&saved);
    assert_eq!(form.text(WALLET), "/etc/oracle/wallet");
    let built = record(&form);
    assert_eq!(built.tls.as_deref(), Some("verify-full"));
    assert_eq!(built.wallet.as_deref(), Some("/etc/oracle/wallet"));
}

#[test]
fn editing_opens_the_form_filled_in_and_keeps_the_id() {
    let mut saved = record(&filled());
    saved.port = Some(6543);
    saved.secret = Secret::Keyring;

    let form = ConnForm::editing(&saved);
    assert_eq!(form.title(), "Edit Connection");
    assert_eq!(form.text(NAME), "prod");
    assert_eq!(form.text(HOST), "db.local");
    assert_eq!(form.text(PORT), "6543");
    assert_eq!(form.text(DATABASE), "app");
    assert_eq!(form.text(SCHEMA), "reporting");
    assert_eq!(
        form.secret(PASSWORD).unwrap().as_str(),
        "",
        "the password is never shown back"
    );

    let shown = buffer_lines(&render(&form, 80, 60)).join("\n");
    assert!(shown.contains("(unchanged)"), "{shown}");

    // Saving it again is the same connection, not a second one.
    let (edited, password) = submitted(&form);
    assert_eq!(password, None, "nothing was typed");
    assert_eq!(edited.id, saved.id);
    assert_eq!(edited.secret, Secret::Keyring, "its password is untouched");
    assert_eq!(edited.port, Some(6543));
}

#[test]
fn editing_can_change_where_the_password_lives() {
    let mut saved = record(&filled());
    saved.secret = Secret::Keyring;

    // Asking each time from now on.
    let mut form = ConnForm::editing(&saved);
    choose(&mut form, STORE, "ASK EACH TIME");
    assert_eq!(record(&form).secret, Secret::Prompt);

    // Or typing a new one, which replaces what the keychain holds.
    let mut form = ConnForm::editing(&saved);
    set(&mut form, PASSWORD, "newer");
    let (edited, password) = submitted(&form);
    assert_eq!(edited.secret, Secret::Keyring);
    assert_eq!(password.as_deref(), Some("newer"));
}

#[test]
fn editing_an_environment_connection_shows_its_variable() {
    let mut saved = record(&filled());
    saved.secret = Secret::Env {
        var: "PGPASSWORD".into(),
    };

    let form = ConnForm::editing(&saved);
    assert_eq!(form.labels(), LABELS_VAR);
    assert_eq!(form.text(VAR), "PGPASSWORD");
}

#[test]
fn invalid_forms_explain_what_is_wrong() {
    let mut form = filled();
    set(&mut form, HOST, "   ");
    assert!(form.submit().unwrap_err().contains("needs a host"));

    let mut form = filled();
    for port in ["abc", "0", "70000"] {
        set(&mut form, PORT, port);
        assert_eq!(
            form.submit().unwrap_err(),
            "Port must be a number from 1 to 65535"
        );
    }
}

#[test]
fn ssl_mode_labels_line_up_with_core() {
    let core: Vec<String> = qry_core::SslMode::ALL
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(core, SSL_MODES);
}

#[test]
fn typing_reaches_the_focused_field_and_the_password_is_masked() {
    let mut form = ConnForm::new("c0ffee42");
    type_str(&mut form, "prod");
    press(&mut form, key(KeyCode::Tab));
    press(&mut form, key(KeyCode::Tab));
    type_str(&mut form, "db.local");
    form.set_focus(PASSWORD);
    type_str(&mut form, "hunter2");

    assert_eq!(form.text(NAME), "prod");
    assert_eq!(form.text(HOST), "db.local");

    let shown = buffer_lines(&render(&form, 80, 60)).join("\n");
    for label in LABELS {
        assert!(shown.contains(label), "missing {label}: {shown}");
    }
    assert!(shown.contains("•••••••"), "{shown}");
    assert!(!shown.contains("hunter2"), "{shown}");
}
