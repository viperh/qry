//! The editor's drawing: highlighting, the gutter's markers, brackets,
//! errors and wide characters.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use ratatui::buffer::Buffer;

use super::*;
use crate::{
    app::{DbAnswer, SessionId, UiEvent},
    config::Config,
    connections::Driver,
    testing::{AppHarness, buffer_lines, grid, render_with, styled},
};

/// The editor focused on `text`, its session's driver `driver`, with the
/// cursor at the end.
fn editor(driver: Driver, text: &str) -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.state_mut().sessions.active_mut().driver = driver;
    h.keys("tab");
    h.send(UiEvent::Paste(text.into()));
    h
}

fn drawn(h: &AppHarness, w: u16, ht: u16) -> Buffer {
    render_with(w, ht, |frame| {
        let state = h.state();
        draw(state, &state.theme, frame, frame.area());
    })
}

/// The cell where `needle` first starts.
fn cell_of(buffer: &Buffer, needle: &str) -> (u16, u16) {
    let lines = buffer_lines(buffer);
    let y = lines.iter().position(|l| l.contains(needle)).unwrap();
    let x = lines[y][..lines[y].find(needle).unwrap()].chars().count();
    (u16::try_from(x).unwrap(), u16::try_from(y).unwrap())
}

#[test]
fn tokens_take_their_theme_colours() {
    let h = editor(Driver::Postgres, "select 'a', 42 -- note\nfrom t");
    let b = drawn(&h, 60, 6);
    let t = Theme::DARK;
    assert_eq!(b[cell_of(&b, "select")].fg, t.teal);
    assert_eq!(b[cell_of(&b, "'a'")].fg, t.green);
    assert_eq!(b[cell_of(&b, "42")].fg, t.purple);
    let note = &b[cell_of(&b, "-- note")];
    assert_eq!(note.fg, t.fg_dim);
    assert!(note.modifier.contains(Modifier::ITALIC));
}

#[test]
fn the_marker_follows_the_statement_under_the_cursor() {
    let mut h = editor(Driver::Sqlite, "select 1;\nselect\n  2;");
    let b = drawn(&h, 40, 4);
    let lines = buffer_lines(&b);
    assert!(!lines[0].starts_with('▌'));
    assert!(lines[1].starts_with('▌') && lines[2].starts_with('▌'));
    h.keys("ctrl-home");
    let lines = buffer_lines(&drawn(&h, 40, 4));
    assert!(lines[0].starts_with('▌') && !lines[1].starts_with('▌'));
}

#[test]
fn a_failed_statement_is_marked_and_its_error_underlined() {
    let mut h = editor(Driver::Sqlite, "select 1;\nselect * frm t;");
    h.keys("f8");
    h.db(DbAnswer::QueryDone(Arc::new(grid(1, 1))));
    h.db(DbAnswer::QueryFailed {
        message: "near \"frm\": syntax error".into(),
        position: Some(9),
    });
    let b = drawn(&h, 40, 3);
    let t = Theme::DARK;
    assert_eq!(b[(0, 1)].symbol(), "▌");
    assert_eq!(b[(0, 1)].fg, t.coral);
    let frm = &b[cell_of(&b, "frm")];
    assert!(frm.modifier.contains(Modifier::UNDERLINED));
    assert_eq!(frm.fg, t.coral);
    assert!(
        !b[cell_of(&b, " t;")]
            .modifier
            .contains(Modifier::UNDERLINED)
    );
}

#[test]
fn brackets_match_amber_and_unmatched_coral() {
    let mut h = editor(Driver::Sqlite, "select (1 + (2))");
    let b = drawn(&h, 40, 2);
    let t = Theme::DARK;
    // The cursor is after the last `)`, which matches the first `(`.
    let open = cell_of(&b, "(1");
    assert_eq!(b[open].fg, t.amber);
    h.send(UiEvent::Paste(")".into()));
    let b = drawn(&h, 40, 2);
    let x = buffer_lines(&b)[0].chars().position(|c| c == ')').unwrap() + 2;
    assert_eq!(
        b[(u16::try_from(x).unwrap(), 0)].fg,
        t.coral,
        "the extra `)`"
    );
}

#[test]
fn wide_characters_keep_the_gutter_aligned() {
    let h = editor(Driver::Sqlite, "select '日本語' as 名前;\nselect 1;");
    let lines = buffer_lines(&drawn(&h, 40, 3));
    let rule = |l: &str| l.chars().position(|c| c == '│');
    assert_eq!(rule(&lines[0]), rule(&lines[1]));
    assert!(
        lines[0].contains('日') && lines[0].contains('語'),
        "{lines:?}"
    );
}

#[test]
fn long_lines_scroll_to_keep_the_cursor_in_view() {
    let long = format!("select {}", "x".repeat(100));
    let h = editor(Driver::Sqlite, &long);
    let lines = buffer_lines(&drawn(&h, 40, 2));
    assert!(!lines[0].contains("select"), "{lines:?}");
    assert!(lines[0].trim_end().ends_with('x'));
}

#[test]
fn editor_snapshots() {
    let cases: [(&str, Driver, &str, u16, u16); 4] = [
        (
            "editor_highlight_postgres_80x12",
            Driver::Postgres,
            "-- active users\nSELECT id, name\n  FROM users\n WHERE created_at > now() - interval '7 days';\n\nSELECT count(*) FROM orders WHERE total > 100.5;\n/* nested /* ok */ comment */",
            80,
            12,
        ),
        (
            "editor_highlight_oracle_plsql_80x16",
            Driver::Oracle,
            "begin\n  insert into t values (q'[it's]');\n  commit;\nend;\n/\nselect sysdate from dual;",
            80,
            16,
        ),
        (
            "editor_bracket_match_60x6",
            Driver::Sqlite,
            "select count((1))",
            60,
            6,
        ),
        (
            "editor_wide_unicode_60x6",
            Driver::Sqlite,
            "select '日本語 🦀' as name;\nselect 1;",
            60,
            6,
        ),
    ];
    for (name, driver, text, w, ht) in cases {
        let h = editor(driver, text);
        insta::assert_snapshot!(name, buffer_lines(&drawn(&h, w, ht)).join("\n"));
        insta::assert_snapshot!(format!("{name}_styled"), styled(&drawn(&h, w, ht)));
    }

    let mut h = editor(Driver::Sqlite, "select 1;\n\nselect 2\n  from t;");
    h.keys("ctrl-home");
    insta::assert_snapshot!(
        "editor_current_statement_marker_80x12",
        styled(&drawn(&h, 80, 12))
    );
    h.state_mut().focus = crate::app::state::Focus::Tree;
    insta::assert_snapshot!("editor_unfocused_80x8", styled(&drawn(&h, 80, 8)));

    let mut h = editor(Driver::Sqlite, "select *\n  frm t;");
    h.keys("f5");
    h.send(UiEvent::Db(
        SessionId::FIRST,
        DbAnswer::QueryFailed {
            message: "syntax error".into(),
            position: Some(11),
        },
    ));
    insta::assert_snapshot!("editor_error_underline_80x8", styled(&drawn(&h, 80, 8)));
}

#[test]
fn run_all_two_tabs() {
    let db = qry_core::testing::FakeDatabase::default()
        .reply(qry_core::testing::Reply::Rows(grid(1, 1)))
        .reply(qry_core::testing::Reply::Rows(grid(1, 1)));
    let mut h = AppHarness::new(Config::embedded()).with_database(db.clone());
    h.send(UiEvent::Started);
    h.keys("tab").type_str("select 1; select 2;").keys("f8");
    assert_eq!(db.calls(), ["select 1", "select 2"]);
    let text = h.render(120, 40);
    assert!(
        text.contains(" 1 result · 1 ") && text.contains(" 2 result · 1 "),
        "{text}"
    );
    insta::assert_snapshot!("run_all_two_tabs_120x40", text);
}
