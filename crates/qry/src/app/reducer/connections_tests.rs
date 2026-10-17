#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    app::{DbAnswer, Effect, UiEvent, event::FileAnswer},
    config::Config,
    connections::{Driver, StoredConnection},
    testing::AppHarness,
};

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.db(DbAnswer::Connected(":memory:".into()));
    let records = ["a", "b", "c"].map(|id| {
        let mut record = StoredConnection::with_id(id, format!("conn-{id}"), Driver::Sqlite);
        record.path = Some(format!("{id}.db"));
        record
    });
    h.file(FileAnswer::ConnectionsLoaded(records.to_vec()));
    h.take_effects();
    h
}

fn open_export(h: &mut AppHarness) {
    h.keys("ctrl-p")
        .type_str("Export connections")
        .keys("enter");
    assert_eq!(h.dialog_title(), Some("Export Connections"));
}

fn written(effects: &[Effect]) -> (String, Vec<String>, bool) {
    match effects {
        [
            Effect::ExportConnections {
                path,
                connections,
                overwrite,
            },
        ] => (
            path.clone(),
            connections.iter().map(|r| r.id.clone()).collect(),
            *overwrite,
        ),
        other => panic!("expected one export, got {other:?}"),
    }
}

#[test]
fn export_writes_only_the_ticked_connections() {
    let mut h = harness();
    open_export(&mut h);
    h.type_str("~/backup.json");
    // Down to the second connection, and leave it out.
    h.keys("down down space enter");
    let (path, ids, overwrite) = written(&h.take_effects());
    assert_eq!(path, "~/backup.json");
    assert_eq!(ids, ["a", "c"]);
    assert!(!overwrite, "an existing file is not replaced unasked");
}

#[test]
fn an_existing_file_asks_before_it_is_replaced() {
    let mut h = harness();
    open_export(&mut h);
    h.type_str("~/backup.json");
    h.keys("down space enter");
    h.take_effects();
    h.file(FileAnswer::ConnectionsExportExists(
        "/home/me/backup.json".into(),
    ));
    assert_eq!(h.dialog_title(), Some("Replace File"));

    // No keeps the file; nothing is written.
    h.keys("n");
    assert_eq!(h.dialog_title(), None);
    assert!(h.take_effects().is_empty());

    // Yes writes the same selection over it.
    open_export(&mut h);
    h.type_str("~/backup.json");
    h.keys("down space enter");
    h.take_effects();
    h.file(FileAnswer::ConnectionsExportExists(
        "/home/me/backup.json".into(),
    ));
    h.keys("enter");
    let (path, ids, overwrite) = written(&h.take_effects());
    assert_eq!(path, "~/backup.json");
    assert_eq!(ids, ["b", "c"]);
    assert!(overwrite);
    h.file(FileAnswer::ConnectionsExported {
        path: "/home/me/backup.json".into(),
        count: 2,
    });
    let (_, notice) = h.state().messages.newest_first().next().unwrap();
    assert_eq!(
        notice.text,
        "Exported 2 connections to /home/me/backup.json"
    );
}

#[test]
fn a_late_exists_answer_for_a_closed_dialog_changes_nothing() {
    let mut h = harness();
    h.file(FileAnswer::ConnectionsExportExists("/x.json".into()));
    assert_eq!(h.dialog_title(), None);
}
