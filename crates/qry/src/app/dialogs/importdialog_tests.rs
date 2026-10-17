#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    app::{DbAnswer, Effect, UiEvent, dialogs::DialogState, event::FileAnswer},
    config::Config,
    connections::Driver,
    import::Skipped,
    testing::{AppHarness, buffer_to_string, styled},
};

fn pg(id: &str, name: &str, host: &str) -> StoredConnection {
    let mut record = StoredConnection::with_id(id, name, Driver::Postgres);
    record.host = Some(host.into());
    record.user = Some("alice".into());
    record.database = Some("app".into());
    record
}

fn file() -> Imported {
    Imported {
        format: Format::DBeaver,
        connections: vec![
            pg("n1", "local-app", "localhost"),
            pg("other-id", "prod again", "db.prod"),
            pg("n2", "analytics", "warehouse"),
        ],
        skipped: vec![Skipped {
            name: "legacy".into(),
            reason: "unsupported driver: mssql".into(),
        }],
        warnings: vec!["analytics has no schema; edit it before connecting".into()],
    }
}

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.db(DbAnswer::Connected(":memory:".into()));
    h.file(FileAnswer::ConnectionsLoaded(vec![pg(
        "p1", "prod", "db.prod",
    )]));
    h.take_effects();
    h
}

/// Picks a file and has it read as `imported`.
fn previewed(imported: Imported) -> AppHarness {
    let mut h = harness();
    h.keys("ctrl-o");
    h.type_str("~/data-sources.json");
    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Import { .. }]
    ));
    h.file(FileAnswer::Imported(Box::new(imported)));
    h
}

fn preview(h: &AppHarness) -> &ImportPreview {
    match h.state().dialog() {
        Some(DialogState::ImportPreview(preview)) => preview,
        other => panic!("no preview: {other:?}"),
    }
}

fn picked(h: &AppHarness) -> Vec<&str> {
    preview(h)
        .entries
        .iter()
        .filter(|e| e.selected)
        .map(|e| e.name.as_str())
        .collect()
}

#[test]
fn preview_classifies_new_existing_skipped() {
    let h = previewed(file());
    let kinds: Vec<(&str, &EntryKind)> = preview(&h)
        .entries
        .iter()
        .map(|e| (e.name.as_str(), &e.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("local-app", &EntryKind::New),
            ("analytics", &EntryKind::New),
            ("prod again", &EntryKind::Existing),
            (
                "legacy",
                &EntryKind::Skipped("unsupported driver: mssql".into())
            ),
        ]
    );
    assert_eq!(
        picked(&h),
        ["local-app", "analytics"],
        "new ones are picked"
    );
    assert!(h.state().tree.position("n1").is_none(), "nothing added yet");
}

#[test]
fn space_toggles_selection() {
    let mut h = previewed(file());
    h.keys("space");
    assert_eq!(picked(&h), ["analytics"]);
    h.keys("space");
    assert_eq!(picked(&h), ["local-app", "analytics"]);
    // An entry already here cannot be picked.
    h.keys("down down space");
    assert_eq!(picked(&h), ["local-app", "analytics"]);
}

#[test]
fn a_selects_all() {
    let mut h = previewed(file());
    h.keys("A");
    assert!(picked(&h).is_empty());
    h.keys("a");
    assert_eq!(picked(&h), ["local-app", "analytics"]);
}

#[test]
fn confirm_adds_selected_and_saves_once() {
    let mut h = previewed(file());
    h.keys("down space enter");
    assert_eq!(h.dialog_title(), None);
    let effects = h.take_effects();
    let saves: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SaveConnections(list) => Some(list),
            _ => None,
        })
        .collect();
    assert_eq!(saves.len(), 1, "{effects:?}");
    let ids: Vec<&str> = saves[0].iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["p1", "n1"]);
    let (toast, _) = h.state().toasts.visible().unwrap();
    assert_eq!(toast.notice.text, "imported 1 · 1 already here · 1 skipped");
}

#[test]
fn parse_error_keeps_path() {
    let mut h = harness();
    h.keys("ctrl-o");
    h.type_str("~/broken.json");
    h.keys("enter");
    h.file(FileAnswer::ImportFailed("~/broken.json is not JSON".into()));
    assert_eq!(h.dialog_title(), Some("Import Connections"));
    let shown = h.render(120, 40);
    assert!(shown.contains("~/broken.json is not JSON"), "{shown}");
    assert!(shown.contains("~/broken.json"), "{shown}");
}

#[test]
fn an_import_waits_for_the_file_and_keeps_the_path_on_failure() {
    let mut h = harness();
    h.keys("ctrl-o");
    h.type_str("~/data-sources.json");
    h.keys("enter");
    assert!(h.render(120, 40).contains("Importing"));
    // A second Enter while reading does nothing.
    h.keys("enter");
    assert!(
        h.take_effects()
            .iter()
            .filter(|e| matches!(e, Effect::Import { .. }))
            .count()
            == 1
    );
    h.file(FileAnswer::ImportFailed(
        "cannot read ~/data-sources.json".into(),
    ));
    let shown = h.render(120, 40);
    assert!(!shown.contains("Importing"), "{shown}");
    assert!(shown.contains("~/data-sources.json"), "{shown}");
}

#[test]
fn a_long_preview_scrolls_with_the_cursor() {
    let many = Imported {
        format: Format::DBeaver,
        connections: (0..500)
            .map(|i| pg(&format!("id{i}"), &format!("conn-{i:03}"), &format!("h{i}")))
            .collect(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    };
    let mut h = previewed(many);
    h.render(120, 40);
    h.keys("pagedown pagedown down");
    let shown = h.render(120, 40);
    let cursor = preview(&h).cursor;
    assert!(cursor > 40, "{cursor}");
    assert!(shown.contains(&format!("conn-{cursor:03}")), "{shown}");
    assert!(!shown.contains("conn-000"), "{shown}");
}

#[test]
fn import_preview_120x40() {
    let h = &mut previewed(file());
    let buffer = h.render_buffer(120, 40);
    insta::assert_snapshot!("import_preview_120x40", buffer_to_string(&buffer));
    insta::assert_snapshot!("import_preview_120x40_styled", styled(&buffer));
}
