//! The history dialog, recall, recording and snippets, through the harness.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{
    history::{Entry, History},
    snippets::Snippet,
    testing::{FakeDatabase, Reply, result},
};

use crate::{
    app::{Effect, FileAnswer, UiEvent, WallClock, state::Overlay},
    config::Config,
    history_store::{HistoryWrite, Loaded},
    testing::AppHarness,
};

/// 2026-10-13T12:00:00Z.
pub(crate) const NOW: i64 = 1_791_892_800;

fn entry(sql: &str, ts: i64, ok: bool) -> Entry {
    Entry {
        ts: Some(ts),
        sql: sql.into(),
        ok,
        rows: ok.then_some(1),
        ms: 4,
        error: (!ok).then(|| "no such table".to_string()),
    }
}

/// `connection`'s history as if read from disk, oldest first.
pub(crate) fn loaded(connection: &str, entries: &[(&str, i64, bool)]) -> Loaded {
    let mut history = History::default();
    for (sql, ts, ok) in entries {
        history.push(entry(sql, *ts, *ok));
    }
    Loaded {
        connection: connection.into(),
        lines: history.len(),
        history,
    }
}

fn harness_with(config: Config, db: FakeDatabase) -> AppHarness {
    let mut h = AppHarness::new(config).with_database(db);
    h.send(UiEvent::WallClock(WallClock {
        unix: NOW,
        offset: 0,
    }));
    h
}

/// The scratch session's history holds three statements; the editor is
/// focused.
fn harness() -> AppHarness {
    let mut h = harness_with(Config::embedded(), FakeDatabase::default());
    h.file(FileAnswer::HistoryLoaded(vec![
        loaded(
            "scratch",
            &[
                ("SELECT * FROM user_s", NOW - 86_400, false),
                ("SELECT u.name FROM users u", NOW - 600, true),
                ("SELECT * FROM users WHERE id = 3", NOW - 60, true),
            ],
        ),
        loaded("prod", &[("DELETE FROM sessions", NOW - 30, true)]),
    ]));
    h.keys("tab");
    h
}

fn dialog(h: &AppHarness) -> &crate::app::history::HistoryDialog {
    match h.state().overlays.last() {
        Some(Overlay::History(dialog)) => dialog,
        other => panic!("no history dialog: {other:?}"),
    }
}

fn query(h: &AppHarness) -> String {
    h.state().editor().query()
}

fn history_writes(h: &mut AppHarness) -> Vec<HistoryWrite> {
    h.take_effects()
        .into_iter()
        .filter_map(|e| match e {
            Effect::History(write) => Some(write),
            _ => None,
        })
        .collect()
}

fn config(edit: impl FnOnce(&mut crate::config::HistoryConfig)) -> Config {
    let mut config = Config::embedded();
    edit(&mut config.history);
    config
}

#[test]
fn ctrl_r_opens_and_esc_closes() {
    let mut h = harness();
    h.keys("ctrl-r");
    let open = dialog(&h);
    assert!(!open.all);
    assert_eq!(open.rows.len(), 3, "only this connection's");
    assert_eq!(open.rows[0].index, 2, "newest first");
    h.type_str("usr");
    assert_eq!(dialog(&h).input.value, "usr");
    h.keys("esc");
    assert!(h.state().overlays.is_empty());
    assert_eq!(query(&h), "", "typing went to the query, not the editor");
}

#[test]
fn tab_toggles_scope() {
    let mut h = harness();
    h.keys("ctrl-r tab");
    assert!(dialog(&h).all);
    assert_eq!(dialog(&h).rows.len(), 4);
    assert_eq!(dialog(&h).rows[0].connection, "prod", "the newest of all");
    h.keys("tab");
    assert_eq!(dialog(&h).rows.len(), 3);
}

#[test]
fn search_filters_with_match_positions() {
    let mut h = harness();
    h.keys("ctrl-r").type_str("sel usr");
    let rows = &dialog(&h).rows;
    assert_eq!(rows.len(), 3, "`user_s` has a `usr` too: {rows:?}");
    assert!(rows.iter().all(|r| r.positions.len() == 7));
    h.type_str(" wh");
    let rows = &dialog(&h).rows;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].index, 2);
}

#[test]
fn enter_inserts_as_new_statement() {
    let mut h = harness();
    h.type_str("SELECT 1");
    h.keys("ctrl-r enter");
    assert!(h.state().overlays.is_empty());
    assert_eq!(query(&h), "SELECT 1;\nSELECT * FROM users WHERE id = 3;");
    assert!(
        !h.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Db(_, crate::db::DbCommand::Query(_)))),
        "Enter only inserts"
    );
    // Into an empty editor it goes alone.
    let mut h = harness();
    h.keys("ctrl-r down enter");
    assert_eq!(query(&h), "SELECT u.name FROM users u;");
}

#[test]
fn ctrl_enter_inserts_and_runs() {
    let db = FakeDatabase::default()
        .reply(Reply::Rows(result(&["a"], &[&[Some("1")]])))
        .reply(Reply::Rows(result(&["id"], &[&[Some("3")]])));
    let mut h = harness_with(Config::embedded(), db.clone());
    h.file(FileAnswer::HistoryLoaded(vec![loaded(
        "scratch",
        &[("SELECT * FROM users WHERE id = 3", NOW - 60, true)],
    )]));
    h.keys("tab").type_str("SELECT 1").keys("f5");
    // `SELECT 1` is the newest entry now.
    h.keys("ctrl-r").type_str("users").keys("ctrl-enter");
    assert_eq!(db.calls(), ["SELECT 1", "SELECT * FROM users WHERE id = 3"]);
    assert_eq!(query(&h), "SELECT 1;\nSELECT * FROM users WHERE id = 3;");
}

#[test]
fn ctrl_d_twice_deletes() {
    let mut h = harness_with(
        config(|c| c.ephemeral_to_disk = true),
        FakeDatabase::default(),
    );
    h.file(FileAnswer::HistoryLoaded(vec![loaded(
        "scratch",
        &[("SELECT 1", NOW - 60, true), ("SELECT 2", NOW, true)],
    )]));
    h.keys("tab ctrl-r ctrl-d");
    assert!(dialog(&h).deleting);
    assert!(h.render(100, 30).contains("press ctrl-d again"));
    assert_eq!(dialog(&h).rows.len(), 2, "one press only asks");
    // Moving away forgets the first press.
    h.keys("down");
    assert!(!dialog(&h).deleting);
    h.take_effects();
    h.keys("up ctrl-d ctrl-d");
    assert_eq!(dialog(&h).rows.len(), 1);
    let left = h.state().history.get("scratch").unwrap();
    assert_eq!(left.get(0).unwrap().sql, "SELECT 1");
    let writes = history_writes(&mut h);
    assert!(
        matches!(&writes[..], [HistoryWrite::Rewrite { connection, content }]
            if connection == "scratch" && content.contains("SELECT 1") && !content.contains("SELECT 2")),
        "{writes:?}"
    );
}

#[test]
fn alt_up_recall_only_on_empty_buffer() {
    let mut h = harness();
    h.keys("alt-up");
    assert_eq!(query(&h), "SELECT * FROM users WHERE id = 3");
    h.keys("alt-up");
    assert_eq!(query(&h), "SELECT u.name FROM users u");
    h.keys("alt-down");
    assert_eq!(query(&h), "SELECT * FROM users WHERE id = 3");
    h.keys("alt-down");
    assert_eq!(query(&h), "", "past the newest the editor is empty again");
    // Typed text is not replaced.
    h.type_str("SELECT 9");
    h.keys("alt-up");
    assert_eq!(query(&h), "SELECT 9");
    // Nor is a recalled statement once it is edited.
    let mut h = harness();
    h.keys("alt-up").type_str(" LIMIT 1").keys("alt-up");
    assert_eq!(query(&h), "SELECT * FROM users WHERE id = 3 LIMIT 1");
}

#[test]
fn executed_statements_are_recorded_and_secrets_redacted() {
    let db = FakeDatabase::default()
        .reply(Reply::Affected(0))
        .reply(Reply::Error("syntax error".into()));
    let mut h = harness_with(config(|c| c.ephemeral_to_disk = true), db);
    h.keys("tab")
        .type_str("ALTER USER x PASSWORD 'CANARY-PW-2'; SELEC 1")
        .keys("f8");
    let writes = history_writes(&mut h);
    let lines: Vec<&str> = writes
        .iter()
        .map(|w| match w {
            HistoryWrite::Append { connection, line } => {
                assert_eq!(connection, "scratch");
                line.as_str()
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(lines.len(), 2, "one entry per statement");
    assert!(!lines[0].contains("CANARY"), "{}", lines[0]);
    assert!(
        lines[0].contains(r#""sql":"ALTER USER x PASSWORD '•••'""#),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains(r#""ts":"2026-10-13T12:00:00Z""#));
    assert!(lines[1].contains(r#""ok":false"#) && lines[1].contains("syntax error"));
    let kept = h.state().history.get("scratch").unwrap();
    assert_eq!(kept.len(), 2);
}

#[test]
fn scratch_history_stays_in_memory_by_default() {
    let db = FakeDatabase::default().reply(Reply::Affected(1));
    let mut h = harness_with(Config::embedded(), db);
    h.keys("tab").type_str("UPDATE t SET a = 1").keys("f5");
    assert!(history_writes(&mut h).is_empty());
    assert_eq!(h.state().history.len("scratch"), 1);
}

#[test]
fn disabled_history_records_nothing() {
    let db = FakeDatabase::default().reply(Reply::Affected(1));
    let mut h = harness_with(
        config(|c| {
            c.enabled = false;
            c.ephemeral_to_disk = true;
        }),
        db,
    );
    h.keys("tab").type_str("UPDATE t SET a = 1").keys("f5");
    assert!(history_writes(&mut h).is_empty());
    assert_eq!(h.state().history.len("scratch"), 0);
}

#[test]
fn file_is_compacted_past_the_cap_plus_ten_percent() {
    let mut db = FakeDatabase::default();
    for _ in 0..12 {
        db = db.reply(Reply::Affected(1));
    }
    let mut h = harness_with(
        config(|c| {
            c.max_entries = 10;
            c.ephemeral_to_disk = true;
        }),
        db,
    );
    h.keys("tab");
    let mut rewrites = Vec::new();
    for i in 0..12 {
        h.type_str(&format!("UPDATE t SET a = {i}"))
            .keys("f5 ctrl-a delete");
        rewrites.extend(
            history_writes(&mut h)
                .into_iter()
                .filter(|w| matches!(w, HistoryWrite::Rewrite { .. })),
        );
    }
    assert_eq!(rewrites.len(), 1, "once the file had 12 lines");
    let HistoryWrite::Rewrite { content, .. } = &rewrites[0] else {
        unreachable!()
    };
    assert_eq!(content.lines().count(), 10);
    assert!(content.contains("a = 11") && !content.contains("a = 1\""));
}

#[test]
fn a_write_failure_is_toasted_once() {
    let mut h = harness();
    h.file(FileAnswer::HistoryWriteFailed("disk full".into()));
    h.file(FileAnswer::HistoryWriteFailed("disk full".into()));
    let toasts: Vec<_> = h
        .state()
        .messages
        .newest_first()
        .filter(|(_, n)| n.text.contains("disk full"))
        .collect();
    assert_eq!(toasts.len(), 1);
}

#[test]
fn loading_keeps_what_was_recorded_before() {
    let db = FakeDatabase::default().reply(Reply::Affected(1));
    let mut h = harness_with(Config::embedded(), db);
    h.keys("tab").type_str("UPDATE t SET a = 1").keys("f5");
    h.file(FileAnswer::HistoryLoaded(vec![loaded(
        "scratch",
        &[("SELECT old", NOW - 100, true)],
    )]));
    let kept: Vec<_> = h
        .state()
        .history
        .get("scratch")
        .unwrap()
        .entries()
        .map(|e| e.sql.clone())
        .collect();
    assert_eq!(kept, ["SELECT old", "UPDATE t SET a = 1"]);
}

// ---- snippets ---------------------------------------------------------

fn saved(h: &mut AppHarness) -> Vec<Snippet> {
    h.take_effects()
        .into_iter()
        .rev()
        .find_map(|e| match e {
            Effect::SaveSnippets(list) => Some(list),
            _ => None,
        })
        .expect("the snippets were saved")
}

#[test]
fn ctrl_s_saves_the_statement_as_a_snippet() {
    let mut h = harness();
    h.type_str("SELECT 1;\nSELECT * FROM users WHERE active");
    h.keys("ctrl-s");
    assert_eq!(h.dialog_title(), Some("Save Snippet"));
    h.type_str("active users").keys("enter");
    assert_eq!(h.dialog_title(), None);
    let list = saved(&mut h);
    assert_eq!(
        list,
        [Snippet {
            name: "active users".into(),
            sql: "SELECT * FROM users WHERE active".into(),
            connection: Some("scratch".into()),
            tags: Vec::new(),
        }]
    );
    // The same name in the same scope is refused; global is another scope.
    h.keys("ctrl-s").type_str("active users").keys("enter");
    assert_eq!(h.dialog_title(), Some("Save Snippet"));
    assert!(h.render(80, 24).contains("already saved"));
    h.keys("tab right enter");
    assert_eq!(h.dialog_title(), None);
    let list = saved(&mut h);
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].connection, None);
}

#[test]
fn palette_lists_snippets_and_inserts_or_runs_them() {
    let db = FakeDatabase::default().reply(Reply::Rows(result(&["a"], &[&[Some("1")]])));
    let mut h = harness_with(Config::embedded(), db.clone());
    h.file(FileAnswer::SnippetsLoaded(vec![
        Snippet {
            name: "active users".into(),
            sql: "SELECT * FROM users WHERE active".into(),
            connection: None,
            tags: Vec::new(),
        },
        Snippet {
            name: "elsewhere".into(),
            sql: "SELECT 2".into(),
            connection: Some("prod".into()),
            tags: Vec::new(),
        },
    ]));
    h.keys("tab ctrl-p");
    let Some(Overlay::Palette(palette)) = h.state().overlays.last() else {
        panic!("no palette");
    };
    let labels: Vec<_> = palette.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"snippet: active users"), "{labels:?}");
    assert!(
        !labels.contains(&"snippet: elsewhere"),
        "another connection's"
    );
    h.type_str("snippet: active").keys("enter");
    assert_eq!(query(&h), "SELECT * FROM users WHERE active;");
    assert!(db.calls().is_empty());
    h.keys("ctrl-p")
        .type_str("snippet: active")
        .keys("ctrl-enter");
    assert_eq!(db.calls(), ["SELECT * FROM users WHERE active"]);
}

#[test]
fn manage_snippets_renames_deletes_and_edits() {
    let mut h = harness();
    h.file(FileAnswer::SnippetsLoaded(vec![
        Snippet {
            name: "one".into(),
            sql: "SELECT 1".into(),
            connection: None,
            tags: vec!["t".into()],
        },
        Snippet {
            name: "two".into(),
            sql: "SELECT 2".into(),
            connection: None,
            tags: Vec::new(),
        },
    ]));
    h.keys("ctrl-p").type_str("manage snippets").keys("enter");
    assert!(matches!(
        h.state().overlays.last(),
        Some(Overlay::Snippets(_))
    ));

    // r: rename.
    h.keys("r");
    assert_eq!(h.dialog_title(), Some("Rename Snippet"));
    h.keys("end backspace").type_str("ce").keys("enter");
    let list = saved(&mut h);
    assert_eq!(list[0].name, "once");
    assert_eq!(list[0].tags, ["t"], "a rename keeps the rest");
    assert!(matches!(
        h.state().overlays.last(),
        Some(Overlay::Snippets(_))
    ));

    // d: delete, after a coral confirm that starts on Cancel.
    h.keys("down d");
    assert_eq!(h.dialog_title(), Some("Delete Snippet"));
    h.keys("enter");
    assert_eq!(h.state().snippets.len(), 2, "Cancel keeps it");
    h.keys("ctrl-p")
        .type_str("manage snippets")
        .keys("enter down d y");
    let list = saved(&mut h);
    assert_eq!(
        list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["once"]
    );

    // Enter: edit the SQL in the editor; ctrl-s there saves over it.
    h.keys("enter");
    assert_eq!(query(&h), "SELECT 1;");
    h.keys("left").type_str("0").keys("ctrl-s");
    assert_eq!(h.dialog_title(), Some("Save Snippet"));
    h.keys("enter");
    let list = saved(&mut h);
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].name.as_str(), list[0].sql.as_str()),
        ("once", "SELECT 10")
    );
}
