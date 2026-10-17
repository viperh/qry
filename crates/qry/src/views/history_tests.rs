//! Snapshots of the history dialog, the snippet dialogs and the palette's
//! snippets.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::{
    history::{Entry, History},
    snippets::Snippet,
};

use crate::{
    app::{FileAnswer, UiEvent, WallClock},
    config::Config,
    history_store::Loaded,
    testing::{AppHarness, buffer_lines, styled},
};

/// 2026-10-13T12:00:00Z.
const NOW: i64 = 1_791_892_800;

fn loaded(connection: &str, entries: &[(&str, i64, bool, Option<u64>, u64)]) -> Loaded {
    let mut history = History::default();
    for &(sql, ts, ok, rows, ms) in entries {
        history.push(Entry {
            ts: Some(ts),
            sql: sql.into(),
            ok,
            rows,
            ms,
            error: (!ok).then(|| "relation does not exist".into()),
        });
    }
    Loaded {
        connection: connection.into(),
        lines: history.len(),
        history,
    }
}

/// A harness at `w`×`h` with the scratch and `prod` histories loaded.
fn harness(w: u16, h: u16) -> AppHarness {
    let mut app = AppHarness::new(Config::embedded());
    app.send(UiEvent::Started);
    app.send(UiEvent::WallClock(WallClock {
        unix: NOW,
        offset: 0,
    }));
    app.resize(w, h);
    app.file(FileAnswer::HistoryLoaded(vec![
        loaded(
            "scratch",
            &[
                ("SELECT * FROM orders", NOW - 10 * 86_400, true, Some(7), 12),
                ("SELECT * FROM user_s", NOW - 86_400, false, None, 0),
                (
                    "SELECT u.name, count(*) FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.name ORDER BY 2 DESC",
                    NOW - 3 * 3600,
                    true,
                    Some(42),
                    31,
                ),
                ("SELECT *\nFROM users\nWHERE id = 3", NOW - 2 * 3600 - 48 * 60, true, Some(1), 4),
            ],
        ),
        loaded(
            "prod",
            &[("UPDATE users SET active = true", NOW - 600, true, Some(3), 9)],
        ),
    ]));
    app.keys("tab");
    app
}

#[test]
fn history_dialog_snapshots() {
    let mut h = harness(120, 40);
    h.keys("ctrl-r");
    insta::assert_snapshot!("history_dialog_120x40", h.render(120, 40));
    insta::assert_snapshot!(
        "history_dialog_120x40_styled",
        styled(&h.render_buffer(120, 40))
    );

    h.keys("tab");
    insta::assert_snapshot!("history_dialog_all_connections_120x40", h.render(120, 40));
    insta::assert_snapshot!(
        "history_dialog_all_connections_120x40_styled",
        styled(&h.render_buffer(120, 40))
    );

    let mut h = harness(80, 24);
    h.keys("ctrl-r").type_str("sel usr");
    insta::assert_snapshot!("history_dialog_filtered_80x24", h.render(80, 24));
    insta::assert_snapshot!(
        "history_dialog_filtered_80x24_styled",
        styled(&h.render_buffer(80, 24))
    );
}

#[test]
fn matches_are_amber_glyphs_green_and_coral() {
    let mut h = harness(120, 40);
    h.keys("ctrl-r").type_str("user_s");
    let buffer = h.render_buffer(120, 40);
    let theme = *h.theme();
    let lines = buffer_lines(&buffer);
    // The row, not the query line above it.
    let y = lines
        .iter()
        .position(|l| l.contains("user_s") && l.contains('✗'))
        .unwrap();
    let at = |needle: &str| {
        let x = lines[y][..lines[y].find(needle).unwrap()].chars().count();
        &buffer[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())]
    };
    assert_eq!(at("user_s").fg, theme.amber, "the matched u");
    assert_eq!(at("✗").fg, theme.coral);
    assert_eq!(at("user_s").bg, theme.purple_tint, "on the selected row");

    let mut h = harness(120, 40);
    h.keys("ctrl-r");
    let buffer = h.render_buffer(120, 40);
    let lines = buffer_lines(&buffer);
    let y = lines.iter().position(|l| l.contains("✓")).unwrap();
    let x = lines[y][..lines[y].find('✓').unwrap()].chars().count();
    assert_eq!(
        buffer[(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())].fg,
        theme.green
    );
}

#[test]
fn a_huge_statement_shows_its_first_line_only() {
    let mut h = harness(100, 30);
    let huge = format!("SELECT 'start'\n{}", "x".repeat(100_000));
    h.file(FileAnswer::HistoryLoaded(vec![loaded(
        "scratch",
        &[(&huge, NOW, true, Some(1), 1)],
    )]));
    h.keys("ctrl-r");
    let text = h.render(100, 30);
    assert!(text.contains("SELECT 'start'"));
    assert!(!text.contains("xxxx"));
    assert_eq!(
        h.state()
            .history
            .get("scratch")
            .unwrap()
            .get(0)
            .unwrap()
            .sql,
        huge,
        "stored in full"
    );
}

#[test]
fn renders_at_every_size_without_panic() {
    for (w, hgt) in [(10, 3), (20, 5), (40, 10), (80, 24), (200, 60)] {
        let mut h = harness(w, hgt);
        h.keys("ctrl-r tab");
        h.render(w, hgt);
    }
}

#[test]
fn snippet_save_dialog_80x24() {
    let mut h = harness(80, 24);
    h.type_str("SELECT * FROM users WHERE active")
        .keys("ctrl-s");
    h.type_str("active users");
    insta::assert_snapshot!("snippet_save_dialog_80x24", h.render(80, 24));
    insta::assert_snapshot!(
        "snippet_save_dialog_80x24_styled",
        styled(&h.render_buffer(80, 24))
    );
}

#[test]
fn palette_with_snippets_group_120x40() {
    let mut h = harness(120, 40);
    h.file(FileAnswer::SnippetsLoaded(vec![
        Snippet {
            name: "active users".into(),
            sql: "SELECT * FROM users WHERE active".into(),
            connection: None,
            tags: vec!["users".into()],
        },
        Snippet {
            name: "big orders".into(),
            sql: "SELECT * FROM orders WHERE total > 100".into(),
            connection: Some("scratch".into()),
            tags: Vec::new(),
        },
    ]));
    h.keys("ctrl-p").type_str("snippet");
    insta::assert_snapshot!("palette_with_snippets_group_120x40", h.render(120, 40));
    insta::assert_snapshot!(
        "palette_with_snippets_group_120x40_styled",
        styled(&h.render_buffer(120, 40))
    );
    // Manage snippets lists them too.
    h.keys("esc ctrl-p")
        .type_str("manage snippets")
        .keys("enter");
    let text = h.render(120, 40);
    assert!(
        text.contains("big orders") && text.contains("global"),
        "{text}"
    );
}
