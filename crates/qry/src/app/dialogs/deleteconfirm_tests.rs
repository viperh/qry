#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    app::{
        DbAnswer, Effect, UiEvent,
        dialogs::{confirm::Check, form::Form},
        event::FileAnswer,
        state::Focus,
    },
    config::Config,
    connections::Driver,
    testing::{AppHarness, buffer_to_string, styled},
};

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.db(DbAnswer::Connected(":memory:".into()));
    h.take_effects();
    h
}

/// A saved `prod-db` keeping its password in `secret`, selected in the
/// tree.
fn saved(h: &mut AppHarness, secret: Secret) -> StoredConnection {
    let mut record = StoredConnection::with_id("p1", "prod-db", Driver::Sqlite);
    record.path = Some("prod.db".into());
    record.secret = secret;
    h.file(FileAnswer::ConnectionsLoaded(vec![record.clone()]));
    h.state_mut().focus = Focus::Tree;
    // The second row, under the scratch connection.
    h.keys("g g j");
    h.take_effects();
    record
}

fn confirm_delete(h: &mut AppHarness) -> Vec<Effect> {
    h.keys("delete");
    assert_eq!(h.dialog_title(), Some("Delete Connection"));
    // Destructive: it starts on Cancel.
    h.keys("right enter");
    h.take_effects()
}

#[test]
fn the_keychain_option_is_only_there_for_a_keychain_password() {
    let mut record = StoredConnection::with_id("p1", "prod-db", Driver::Postgres);
    record.secret = Secret::Keyring;
    let confirm = delete_connection(&record);
    assert_eq!(confirm.fields().len(), 2);
    assert!(confirm.is_checked(Check::ForgetSecret));
    assert!(confirm.is_checked(Check::DeleteHistory));
    record.secret = Secret::Prompt;
    assert_eq!(delete_connection(&record).fields().len(), 1);
}

#[test]
fn cancel_is_noop() {
    let mut h = harness();
    saved(&mut h, Secret::Keyring);
    let before = h.state().tree.clone();
    for keys in ["delete esc", "delete enter", "delete n"] {
        h.keys(keys);
        assert_eq!(h.dialog_title(), None, "{keys}");
        assert!(h.take_effects().is_empty(), "{keys}");
        assert_eq!(h.state().tree, before);
    }
}

#[test]
fn confirm_forgets_keychain_secret_and_deletes_history_when_checked() {
    let mut h = harness();
    saved(&mut h, Secret::Keyring);
    let effects = confirm_delete(&mut h);
    assert!(h.state().tree.position("p1").is_none());
    assert_eq!(
        effects,
        [
            Effect::ForgetSecret("p1".into()),
            Effect::DeleteHistory("p1".into()),
            Effect::SaveConnections(Vec::new()),
        ]
    );
}

#[test]
fn unchecked_keeps_history() {
    let mut h = harness();
    saved(&mut h, Secret::Keyring);
    h.keys("delete");
    // Down to the history box and untick it, then over to Delete.
    h.keys("down space right enter");
    let effects = h.take_effects();
    assert!(effects.contains(&Effect::ForgetSecret("p1".into())));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::DeleteHistory(_))),
        "{effects:?}"
    );

    // And unticking the keychain keeps the password.
    let mut h = harness();
    saved(&mut h, Secret::Keyring);
    h.keys("delete space right enter");
    let effects = h.take_effects();
    assert!(!effects.iter().any(|e| matches!(e, Effect::ForgetSecret(_))));
}

#[test]
fn open_session_disconnected_first() {
    let mut h = harness();
    saved(&mut h, Secret::None);
    h.keys("enter");
    let session = h.active_session();
    h.db(DbAnswer::Connected("prod.db".into()));
    h.take_effects();
    h.state_mut().tree.selected = 1;

    let effects = confirm_delete(&mut h);
    assert_eq!(effects.first(), Some(&Effect::CloseSession(session)));
    assert!(!h.sessions().contains(&session));
    assert!(effects.contains(&Effect::SaveConnections(Vec::new())));
}

#[test]
fn delete_confirm_80x24() {
    let mut h = harness();
    saved(&mut h, Secret::Keyring);
    h.keys("delete");
    let buffer = h.render_buffer(80, 24);
    insta::assert_snapshot!("delete_confirm_80x24", buffer_to_string(&buffer));
    insta::assert_snapshot!("delete_confirm_80x24_styled", styled(&buffer));
}
