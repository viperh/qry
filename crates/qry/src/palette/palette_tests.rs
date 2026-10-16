//! Unit tests for the palette's model, and for running items through the
//! reducer the way its keys run them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use strum::IntoEnumIterator;

use super::*;
use crate::{
    app::{DbAnswer, Effect, FileAnswer, UiEvent, state::Overlay},
    config::Config,
    connections::{Driver, StoredConnection},
    testing::{AppHarness, grid},
};

/// The actions that only make sense on a key: moving about, editing text,
/// answering a dialog, and the numbered sessions (listed by name instead).
const KEY_ONLY: &[Action] = &[
    Action::Palette,
    Action::FocusNext,
    Action::FocusPrev,
    Action::Up,
    Action::Down,
    Action::Left,
    Action::Right,
    Action::PageUp,
    Action::PageDown,
    Action::Top,
    Action::Bottom,
    Action::LineStart,
    Action::LineEnd,
    Action::Toggle,
    Action::Open,
    Action::Cancel,
    Action::Submit,
    Action::TestConnection,
    Action::NextField,
    Action::PrevField,
    Action::NextChoice,
    Action::NewLine,
    Action::Indent,
    Action::Dedent,
    Action::WordLeft,
    Action::WordRight,
    Action::SelectLeft,
    Action::SelectRight,
    Action::SelectUp,
    Action::SelectDown,
    Action::SelectWordLeft,
    Action::SelectWordRight,
    Action::SelectLineStart,
    Action::SelectLineEnd,
    Action::SelectAll,
    Action::DeleteBack,
    Action::DeleteForward,
    Action::DeleteWordBack,
    Action::DeleteWordForward,
    Action::DeleteToLineEnd,
    Action::Undo,
    Action::Redo,
    Action::Cut,
    Action::Copy,
    Action::Paste,
    Action::DismissToast,
];

fn harness() -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    let mut prod = StoredConnection::with_id("prod", "prod", Driver::Sqlite);
    prod.path = Some("prod.db".into());
    h.file(FileAnswer::ConnectionsLoaded(vec![prod]));
    h.take_effects();
    h
}

fn palette(h: &AppHarness) -> &PaletteState {
    match h.state().overlays.last() {
        Some(Overlay::Palette(palette)) => palette,
        other => panic!("no palette: {other:?}"),
    }
}

fn labels(h: &AppHarness) -> Vec<String> {
    let p = palette(h);
    p.matches
        .iter()
        .map(|(i, _)| p.items[*i].label.clone())
        .collect()
}

#[test]
fn every_user_action_has_a_palette_label() {
    for action in Action::iter() {
        if matches!(action, Action::SwitchSession(_)) || KEY_ONLY.contains(&action) {
            continue;
        }
        assert!(
            action.palette_label().is_some(),
            "{action:?} has no palette label"
        );
    }
    for action in KEY_ONLY {
        assert!(action.palette_label().is_none(), "{action:?} is key-only");
    }
}

#[test]
fn dynamic_items_include_connections_and_sessions() {
    let mut h = harness();
    h.keys("ctrl-p");
    let all = labels(&h);
    assert!(
        all.contains(&"Open connection: prod".to_string()),
        "{all:?}"
    );
    assert!(all.contains(&"Open connection: scratch".to_string()));
    assert!(all.contains(&"Switch to session 1: scratch".to_string()));
    assert!(all.contains(&"export csv <path>".to_string()));
}

#[test]
fn empty_query_lists_recent_first() {
    let mut h = harness();
    h.keys("ctrl-p").type_str("show mess").keys("enter");
    h.keys("esc ctrl-p").type_str("toggle hints").keys("enter");
    h.keys("ctrl-p");
    let all = labels(&h);
    assert_eq!(&all[..2], ["Toggle hints", "Show messages"], "newest first");
    assert_eq!(all[2], "Quit", "then the commands in order");
}

#[test]
fn ranking_is_deterministic_and_prefers_word_starts() {
    let mut h = harness();
    h.keys("ctrl-p").type_str("ec");
    let first = labels(&h);
    // Two word starts each; equal scores keep the commands' order.
    assert_eq!(
        &first[..2],
        ["Edit connection", "Export connections…"],
        "{first:?}"
    );
    h.keys("esc ctrl-p").type_str("ec");
    assert_eq!(labels(&h), first);
    h.keys("esc ctrl-p").type_str("exc");
    assert_eq!(labels(&h)[0], "Export connections…");
}

#[test]
fn tab_fills_parametrised_command() {
    let mut h = harness();
    h.keys("ctrl-p").type_str("exp csv");
    assert_eq!(
        palette(&h).selected_item().unwrap().label,
        "export csv <path>"
    );
    h.keys("tab");
    assert_eq!(palette(&h).query(), "export csv ");
    assert_eq!(labels(&h), ["export csv <path>"]);
    h.type_str("/tmp/out.csv").keys("enter");
    let effects = h.take_effects();
    let [Effect::Db(_, crate::db::DbCommand::Export(config))] = effects.as_slice() else {
        panic!("expected an export, got {effects:?}");
    };
    assert_eq!(config.path, "/tmp/out.csv");
    assert!(h.state().overlays.is_empty());
}

#[test]
fn missing_argument_keeps_palette_open_with_error() {
    let mut h = harness();
    h.keys("ctrl-p").type_str("export json").keys("enter");
    assert_eq!(palette(&h).error.as_deref(), Some("needs <path>"));
    assert!(h.take_effects().is_empty());
    assert!(h.render(120, 40).contains("needs <path>"));
}

#[test]
fn invalid_argument_shows_parse_error() {
    let mut h = harness();
    h.keys("ctrl-p").type_str("goto abc").keys("enter");
    assert_eq!(palette(&h).error.as_deref(), Some("not a row number: abc"));
    h.keys("esc ctrl-p").type_str("theme neon").keys("enter");
    assert!(
        palette(&h)
            .error
            .as_deref()
            .unwrap()
            .starts_with("no theme neon")
    );
}

#[test]
fn goto_selects_the_row() {
    let mut h = harness();
    h.db(DbAnswer::QueryDone(Arc::new(grid(50, 2))));
    h.keys("ctrl-p").type_str("goto 30").keys("enter");
    assert_eq!(h.state().results().row, 29);
}

#[test]
fn palette_run_equals_keybound_effects() {
    let by_key = {
        let mut h = harness();
        h.keys("tab").type_str("select 1").keys("f5");
        (h.take_effects(), h.state().clone())
    };
    let by_palette = {
        let mut h = harness();
        h.keys("tab")
            .type_str("select 1")
            .keys("ctrl-p")
            .type_str("run statement")
            .keys("enter");
        (h.take_effects(), h.state().clone())
    };
    assert_eq!(by_key.0, by_palette.0);
    assert!(
        matches!(by_key.0.as_slice(), [Effect::Db(_, crate::db::DbCommand::Query(q))] if q == "select 1")
    );

    // Export results opens the same dialog either way.
    let mut h = harness();
    h.keys("ctrl-p").type_str("export results").keys("enter");
    assert_eq!(h.dialog_title(), Some("Export"));
}

#[test]
fn key_column_follows_remap() {
    let mut h = harness();
    h.keys("ctrl-p").type_str("new conn");
    assert_eq!(
        palette(&h).selected_item().unwrap().key.as_deref(),
        Some("ctrl-n")
    );
    let json = r#"{ "keybindings": { "Global": { "<ctrl-n>": null, "<f2>": "NewConnection" } } }"#;
    let mut h = AppHarness::new(Config::with_user_file("config.json", json).unwrap());
    h.keys("ctrl-p").type_str("new conn");
    assert_eq!(
        palette(&h).selected_item().unwrap().key.as_deref(),
        Some("F2")
    );
}

#[test]
fn opening_a_connection_from_the_palette_opens_its_session() {
    let mut h = harness();
    h.keys("ctrl-p")
        .type_str("open connection prod")
        .keys("enter");
    assert!(matches!(h.take_effects().as_slice(), [Effect::OpenSession(_, r)] if r.id == "prod"));
}

#[test]
fn remember_keeps_ten_newest_first() {
    let mut recent = Vec::new();
    for i in 0..12 {
        remember(&mut recent, &format!("c{i}"));
    }
    remember(&mut recent, "c5");
    assert_eq!(recent.len(), RECENT);
    assert_eq!(recent[0], "c5");
    assert_eq!(recent[1], "c11");
}
