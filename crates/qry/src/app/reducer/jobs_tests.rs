//! Unit tests for `jobs.rs`, and for jobs as runs and cancels make them:
//! the drawer, `ctrl-c`, fetching every row and transactions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use qry_core::testing::{FakeDatabase, Reply, grid};

use super::*;
use crate::{
    app::{DbAnswer, SessionId, UiEvent, state::Focus},
    config::Config,
    db::DbCommand,
    testing::AppHarness,
};

/// The editor focused on `sql`, connected to a fake that answers with
/// `replies`.
fn editor(sql: &str, replies: Vec<Reply>) -> AppHarness {
    let mut db = FakeDatabase::default();
    for reply in replies {
        db = db.reply(reply);
    }
    let mut h = AppHarness::new(Config::embedded()).with_database(db);
    h.send(UiEvent::Started);
    // The connection's toast goes first.
    h.advance(60_000);
    h.keys("tab").type_str(sql);
    h.take_effects();
    h
}

fn states(h: &AppHarness) -> Vec<JobState> {
    h.state().jobs.iter().map(|j| j.state.clone()).collect()
}

#[test]
fn ctrl_c_cancels_focused_session_job() {
    let mut h = editor("select slow; select 2;", vec![Reply::Hang]);
    h.keys("f8").advance(1500);
    assert_eq!(
        states(&h),
        [JobState::Running { rows: 0 }, JobState::Queued]
    );
    assert!(
        h.render(120, 30).contains("running · 1."),
        "spinner with time"
    );
    h.take_effects();

    h.keys("ctrl-c");
    let effects = h.take_effects();
    assert!(
        effects.contains(&Effect::CancelQuery(SessionId::FIRST)),
        "{effects:?}"
    );
    // The running one is cancelled; the rest of the run never goes.
    assert_eq!(states(&h), [JobState::Cancelled]);
    assert!(h.render(120, 30).contains("query cancelled"));
    // The worker's failure for it marks nothing and opens no tab.
    assert!(h.state().sessions.active().failed.is_none());
    assert!(h.state().results().result.is_none());
    assert!(!h.state().sessions.active().busy());

    // With nothing running, ctrl-c copies again.
    h.keys("ctrl-c");
    assert!(
        !h.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::CancelQuery(_)))
    );
}

#[test]
fn a_late_result_of_a_cancelled_job_is_dropped() {
    // No database: the statement stays running, and nothing answers the
    // cancel, as with a driver that cannot cancel.
    let mut h = AppHarness::new(Config::embedded());
    h.send(UiEvent::Started);
    h.keys("tab").type_str("select 1").keys("f5");
    h.keys("ctrl-c");
    // A driver that could not cancel answers anyway.
    h.db(DbAnswer::QueryDone(std::sync::Arc::new(grid(2, 1))));
    assert!(h.state().results().result.is_none());
    assert_eq!(states(&h), [JobState::Cancelled]);
}

#[test]
fn finished_jobs_take_the_workers_time() {
    let mut h = editor(
        "select 1",
        vec![Reply::Delay(
            Duration::from_millis(2400),
            Box::new(Reply::Rows(grid(3, 1))),
        )],
    );
    h.keys("f5").advance(3000);
    let job = h.state().jobs.iter().next().unwrap();
    assert_eq!(job.state, JobState::Done { rows: 3 });
    assert_eq!(
        job.elapsed(Duration::MAX)
            .map(qry_core::jobs::format_elapsed),
        Some("2.4 s".into())
    );
}

/// Only the database effects of `effects`.
fn db_effects(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Db(..)))
        .collect()
}

#[test]
fn truncated_footer_a_fetches_every_row_after_confirming() {
    let mut h = editor("select * from t", vec![Reply::Rows(grid(1500, 1))]);
    h.keys("f5");
    let result = h.state().results().result.clone().unwrap();
    assert_eq!(result.rows.len(), 1000, "max_rows is 1000 by default");
    assert!(result.truncated);
    h.advance(60_000);
    assert!(
        h.render(120, 40)
            .contains("… more rows — showing the first 1")
    );

    h.state_mut().focus = Focus::Results;
    h.take_effects();
    h.keys("A");
    // No row count is known, so it asks first; nothing runs yet.
    assert_eq!(h.dialog_title(), Some("Fetch All Rows"));
    assert!(db_effects(&h.take_effects()).is_empty());
    insta::assert_snapshot!("confirm_fetch_all_80x24", h.render(80, 24));
    h.keys("enter");
    let effects = h.take_effects();
    assert_eq!(
        db_effects(&effects),
        [&Effect::Db(
            SessionId::FIRST,
            DbCommand::QueryAll("select * from t".into())
        )]
    );
}

#[test]
fn a_cancelled_fetch_all_confirm_runs_nothing() {
    let mut h = editor("select * from t", vec![Reply::Rows(grid(1500, 1))]);
    h.keys("f5");
    h.state_mut().focus = Focus::Results;
    h.take_effects();
    h.keys("A esc");
    assert_eq!(h.dialog_title(), None);
    assert!(db_effects(&h.take_effects()).is_empty());
}

#[test]
fn truncated_footer_l_emits_fetch_more_not_rerun() {
    let mut h = editor("select * from t", vec![Reply::Rows(grid(2500, 1))]);
    h.keys("f5").advance(60_000);
    assert_eq!(
        h.state().sessions.active().cursor,
        Some(1),
        "kept for tab 1"
    );
    let text = h.render(120, 40);
    assert!(
        text.contains("showing the first 1\u{2009}000 · L more · A all"),
        "{text}"
    );

    h.state_mut().focus = Focus::Results;
    h.take_effects();
    h.keys("L");
    let effects = h.take_effects();
    assert_eq!(
        db_effects(&effects),
        [&Effect::Db(SessionId::FIRST, DbCommand::FetchMore)]
    );
    let job = h.state().jobs.iter().last().unwrap().clone();
    assert_eq!(job.kind, qry_core::jobs::JobKind::FetchMore);
    assert_eq!(job.state, JobState::Done { rows: 1000 });
    // Appended to the same tab, which still has more.
    let session = h.state().sessions.active();
    assert_eq!(session.tabs.len(), 1);
    let result = session.results().result.clone().unwrap();
    assert_eq!(result.rows.len(), 2000);
    assert_eq!(
        result.rows[1000],
        grid(2500, 1).rows[1000],
        "no row repeated"
    );
    assert!(result.truncated);
    h.advance(60_000);
    assert!(h.render(120, 40).contains("L more"));

    // The last 500, and then nothing more to load.
    h.keys("L");
    let result = h.state().results().result.clone().unwrap();
    assert_eq!(result.rows.len(), 2500);
    assert!(!result.truncated);
    assert_eq!(h.state().sessions.active().cursor, None);
    let text = h.render(120, 40);
    assert!(!text.contains("… more rows"), "{text}");
    assert!(text.contains("loaded 500 more rows"), "{text}");
}

#[test]
fn another_statement_drops_the_kept_cursor() {
    let mut h = editor(
        "select * from t",
        vec![Reply::Rows(grid(1500, 1)), Reply::Rows(grid(1, 1))],
    );
    h.keys("f5");
    assert_eq!(h.state().sessions.active().cursor, Some(1));
    h.keys("ctrl-a").type_str("select 1").keys("f5");
    assert_eq!(h.state().sessions.active().cursor, None);

    // Back on the first tab, which keeps its rows: `L` can only ask to
    // fetch everything again.
    h.state_mut().focus = Focus::Results;
    h.keys("[");
    assert!(!h.render(120, 40).contains("L more"));
    h.take_effects();
    h.keys("L");
    assert_eq!(h.dialog_title(), Some("Fetch All Rows"));
    assert!(db_effects(&h.take_effects()).is_empty());
}

#[test]
fn without_a_kept_cursor_l_falls_back_to_fetching_all() {
    let db = FakeDatabase::default()
        .without_streaming()
        .reply(Reply::Rows(grid(1500, 1)));
    let mut h = AppHarness::new(Config::embedded()).with_database(db);
    h.send(UiEvent::Started);
    h.advance(60_000);
    h.keys("tab").type_str("select * from t").keys("f5");
    h.advance(60_000);
    assert_eq!(h.state().sessions.active().cursor, None);
    let text = h.render(120, 40);
    assert!(
        text.contains("showing the first 1\u{2009}000 · A all"),
        "{text}"
    );
    h.state_mut().focus = Focus::Results;
    h.take_effects();
    h.keys("L enter");
    assert_eq!(
        db_effects(&h.take_effects()),
        [&Effect::Db(
            SessionId::FIRST,
            DbCommand::QueryAll("select * from t".into())
        )]
    );
}

#[test]
fn live_row_count_while_running() {
    let mut h = editor("select * from events", vec![Reply::Hang]);
    h.keys("f5").advance(1000);
    h.db(DbAnswer::Progress { rows: 12_000 });
    h.advance(1250);
    assert_eq!(
        h.state().jobs.iter().last().unwrap().state,
        JobState::Running { rows: 12_000 }
    );
    let text = h.render(120, 24);
    // The results title and the status line both say it.
    assert_eq!(
        text.matches("running · 2.2 s · 12\u{2009}000 rows").count(),
        2,
        "{text}"
    );
}

#[test]
fn a_query_stopped_mid_stream_keeps_its_rows() {
    let mut h = editor("select * from events", vec![Reply::Hang]);
    h.keys("f5").advance(500);
    h.db(DbAnswer::QueryStopped {
        result: std::sync::Arc::new(grid(5000, 2)),
        message: "could not read block 7".into(),
    });
    let job = h.state().jobs.iter().last().unwrap().clone();
    assert_eq!(job.state, JobState::Failed("could not read block 7".into()));
    assert_eq!(
        h.state().results().result.as_ref().unwrap().rows.len(),
        5000
    );
    h.advance(60_000);
    let buffer = h.render_buffer(120, 24);
    let text = crate::testing::buffer_lines(&buffer).join("\n");
    assert!(
        text.contains("stopped after 5\u{2009}000 rows: could not read block 7"),
        "{text}"
    );
    insta::assert_snapshot!("results_failed_mid_stream_120x24", text);
    insta::assert_snapshot!(
        "results_failed_mid_stream_120x24_styled",
        crate::testing::styled(&buffer)
    );
}

#[test]
fn toggle_autocommit_from_the_palette() {
    let mut h = editor("select 1", Vec::new());
    h.keys("ctrl-p").type_str("Toggle autocommit").keys("enter");
    assert!(h.take_effects().contains(&Effect::Db(
        SessionId::FIRST,
        DbCommand::SetAutocommit(false)
    )));
    let session = h.state().sessions.active();
    assert!(!session.autocommit && session.tx_open());
    let buffer = h.render_buffer(120, 30);
    assert!(crate::testing::buffer_lines(&buffer)[28].contains(" TX "));
    assert!(h.render(120, 30).contains("autocommit off"));

    // Quitting now asks, as with an open transaction.
    h.keys("ctrl-q");
    assert_eq!(h.dialog_title(), Some("Open Transaction"));
    h.keys("esc");

    h.keys("ctrl-p").type_str("Toggle autocommit").keys("enter");
    assert!(h.take_effects().contains(&Effect::Db(
        SessionId::FIRST,
        DbCommand::SetAutocommit(true)
    )));
    assert!(h.state().sessions.active().autocommit);
    let buffer = h.render_buffer(120, 30);
    assert!(crate::testing::buffer_lines(&buffer)[28].contains("auto"));
}

#[test]
fn a_failed_autocommit_change_says_why_and_keeps_the_state() {
    let mut h = editor("select 1", Vec::new());
    h.db(DbAnswer::Autocommit(Err("not supported".into())));
    assert!(h.state().sessions.active().autocommit);
    assert!(
        h.render(120, 30)
            .contains("could not change autocommit: not supported")
    );
}

#[test]
fn drawer_toggle_and_height_formula() {
    let mut h = editor(
        "select 1; select 2; select 3;",
        (0..12).map(|_| Reply::Affected(0)).collect(),
    );
    h.state_mut().focus = Focus::Results;
    h.keys("J");
    assert!(h.state().drawer.is_some());
    assert_eq!(h.contexts()[0], crate::keymap::KeyContext::Jobs);
    assert_eq!(h.state().drawer_rows(), 2, "a title and an empty line");
    h.keys("J");
    assert!(h.state().drawer.is_none());

    h.state_mut().focus = Focus::Editor;
    h.keys("f8");
    for _ in 0..3 {
        h.keys("f8");
    }
    assert_eq!(h.state().jobs.len(), 12);
    h.state_mut().focus = Focus::Results;
    h.keys("J");
    assert_eq!(h.state().drawer_rows(), 8, "1 + min(n, 7)");
    h.keys("esc");
    assert!(h.state().drawer.is_none());
}

#[test]
fn drawer_c_cancels_selected() {
    let mut h = editor("select 1; select 2; select 3;", vec![Reply::Hang]);
    h.keys("f8");
    h.state_mut().focus = Focus::Results;
    h.keys("J");
    // Newest first: the last queued one is selected.
    h.keys("c");
    assert_eq!(
        states(&h),
        [JobState::Running { rows: 0 }, JobState::Queued],
        "a queued job is dropped, never sent"
    );
    h.take_effects();
    h.keys("j c");
    assert_eq!(states(&h), [JobState::Cancelled]);
    assert!(
        h.take_effects()
            .contains(&Effect::CancelQuery(SessionId::FIRST))
    );
}

#[test]
fn tx_indicator_after_begin_commit_rollback() {
    let mut h = editor(
        "begin",
        vec![
            Reply::Affected(0),
            Reply::Affected(0),
            Reply::Affected(0),
            Reply::Affected(0),
        ],
    );
    h.keys("f5");
    assert!(h.state().sessions.active().in_transaction);
    assert!(h.render(120, 30).contains(" TX "));
    h.keys("ctrl-p").type_str("Commit").keys("enter");
    assert!(!h.state().sessions.active().in_transaction);
    assert!(h.render(120, 30).contains("auto"));

    h.keys("f5");
    assert!(h.state().sessions.active().in_transaction);
    h.keys("ctrl-p").type_str("Rollback").keys("enter");
    assert!(!h.state().sessions.active().in_transaction);
}

#[test]
fn quit_with_open_tx_shows_confirm() {
    let mut h = editor("begin", vec![Reply::Affected(0), Reply::Affected(0)]);
    h.keys("f5").take_effects();
    h.keys("ctrl-q");
    assert_eq!(h.dialog_title(), Some("Open Transaction"));
    assert!(!h.take_effects().contains(&Effect::Quit));
    insta::assert_snapshot!("confirm_open_tx_on_quit_80x24", h.render(80, 24));

    // Roll back & quit (Cancel is the default): the rollback runs, then
    // qry quits.
    h.keys("right enter");
    let effects = h.take_effects();
    let db = effects
        .iter()
        .position(|e| matches!(e, Effect::Db(_, DbCommand::Query(sql)) if sql == "ROLLBACK"));
    let quit = effects.iter().position(|e| *e == Effect::Quit);
    assert!(db.is_some() && quit > db, "{effects:?}");
}

#[test]
fn quitting_without_a_transaction_asks_nothing() {
    let mut h = editor("select 1", Vec::new());
    h.keys("ctrl-q");
    assert!(h.take_effects().contains(&Effect::Quit));
}

/// The running title, the truncated footer, the drawer and the TX pill.
#[test]
fn job_snapshots() {
    let mut h = editor("select * from events", vec![Reply::Hang]);
    h.keys("f5").advance(1200);
    h.db(DbAnswer::Progress { rows: 12_000 });
    h.advance(1200);
    // 120x20 is under the 80x24 minimum, so these are 24 rows tall.
    let text = h.render(120, 24);
    // Timed at the ticks, every 250 ms.
    assert!(
        text.contains("running · 2.2 s · 12\u{2009}000 rows"),
        "{text}"
    );
    insta::assert_snapshot!("results_running_spinner_120x24", text);

    let mut h = editor("select * from t", vec![Reply::Rows(grid(1500, 2))]);
    h.keys("f5").advance(60_000);
    insta::assert_snapshot!("results_truncated_footer_120x24", h.render(120, 24));

    // Done, failed, cancelled, running and queued, in one session.
    let mut h = editor(
        "update users set x = 1; select * from nope;",
        vec![
            Reply::Affected(3),
            Reply::Error("relation \"nope\" does not exist".into()),
            Reply::Hang,
            Reply::Hang,
        ],
    );
    h.keys("f8");
    h.keys("ctrl-a").type_str("select * from huge;").keys("f8");
    h.advance(9300).keys("ctrl-c");
    h.keys("ctrl-a")
        .type_str("select * from events where a > 1; select 2;")
        .keys("f8");
    h.advance(4100);
    h.state_mut().focus = Focus::Results;
    h.keys("J").advance(60_000);
    let buffer = h.render_buffer(120, 40);
    insta::assert_snapshot!(
        "jobs_drawer_mixed_states_120x40",
        crate::testing::buffer_lines(&buffer).join("\n")
    );
    insta::assert_snapshot!(
        "jobs_drawer_mixed_states_120x40_styled",
        crate::testing::styled(&buffer)
    );

    let mut h = editor("begin", vec![Reply::Affected(0)]);
    h.keys("f5").advance(60_000);
    let buffer = h.render_buffer(120, 30);
    let status = crate::testing::buffer_lines(&buffer)[28].clone();
    assert!(status.contains(" TX "), "{status}");
    insta::assert_snapshot!("status_tx_pill_120x2", status);
}
