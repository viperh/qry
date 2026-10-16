//! The connection file across versions: what qry 0.1 wrote still loads,
//! and what this build does not know survives a save.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry::{
    connections::{self, ConnLabel, Driver, Secret},
    testing::fixture,
};
use serde_json::Value;

fn v0_1() -> (String, std::path::PathBuf) {
    let path = fixture("connections/v0_1.json");
    (std::fs::read_to_string(&path).unwrap(), path)
}

#[test]
fn old_files_load() {
    let (text, path) = v0_1();
    let records = connections::parse(&text, &path).unwrap();
    let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["local-app", "erp", "notes", "shop"]);
    assert_eq!(records[0].driver, Driver::Postgres);
    assert_eq!(records[0].secret, Secret::Keyring);
    assert_eq!(
        records[1].secret,
        Secret::Env {
            var: "ERP_PASSWORD".into()
        }
    );
    assert_eq!(records[1].port, Some(2484));
    assert_eq!(records[2].mode.as_deref(), Some("ro"));
    assert!(records.iter().all(|r| r.label.is_none()), "no labels then");
}

#[test]
fn unknown_keys_round_trip() {
    let (text, path) = v0_1();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("connections.json"), &text).unwrap();

    let records = connections::load_from(dir.path()).unwrap();
    connections::save_to(dir.path(), &records).unwrap();

    let before: Value = serde_json::from_str(&text).unwrap();
    let after: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("connections.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(after["written_by"], before["written_by"], "file-level key");
    assert_eq!(
        after["connections"][2]["colour"], "teal",
        "a record's own unknown key"
    );
    assert_eq!(after, before, "nothing else changed either");
    let _ = path;
}

#[test]
fn label_round_trip() {
    let (text, path) = v0_1();
    let mut records = connections::parse(&text, &path).unwrap();
    records[0].label = Some(ConnLabel::Prod);
    records[3].label = Some(ConnLabel::Dev);

    let dir = tempfile::tempdir().unwrap();
    connections::save_to(dir.path(), &records).unwrap();
    let written = std::fs::read_to_string(dir.path().join("connections.json")).unwrap();
    let raw: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(raw["connections"][0]["label"], "prod");
    assert!(
        raw["connections"][1].get("label").is_none(),
        "none is not written"
    );

    let back = connections::load_from(dir.path()).unwrap();
    assert_eq!(back, records);
}
