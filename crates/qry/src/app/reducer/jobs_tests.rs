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

#[test]
fn truncated_footer_a_fetches_every_row_not_load_more() {
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
    let effects = h.take_effects();
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Db(..)))
            .collect::<Vec<_>>(),
        [&Effect::Db(
            SessionId::FIRST,
            DbCommand::QueryAll("select * from t".into())
        )]
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
    h.keys("f5").advance(2400);
    // 120x20 is under the 80x24 minimum, so these are 24 rows tall.
    let text = h.render(120, 24);
    // Timed at the ticks, every 250 ms.
    assert!(text.contains("running · 2.2 s"), "{text}");
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
