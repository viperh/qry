//! Unit tests for `jobs.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;

const MS: Duration = Duration::from_millis(1);

fn state(jobs: &Jobs<u8>, id: JobId) -> JobState {
    jobs.get(id).unwrap().state.clone()
}

#[test]
fn fifo_per_session_parallel_across_sessions() {
    let mut jobs = Jobs::default();
    let a1 = jobs.enqueue(1, JobKind::Query, "select 1");
    let a2 = jobs.enqueue(1, JobKind::Query, "select 2");
    let b1 = jobs.enqueue(2, JobKind::Query, "select 3");

    assert_eq!(jobs.next_for(1), Some(a1));
    jobs.start(a1, MS);
    assert_eq!(jobs.next_for(1), None, "one at a time per session");
    // Starting the second out of turn is refused.
    jobs.start(a2, MS);
    assert_eq!(state(&jobs, a2), JobState::Queued);

    assert_eq!(jobs.next_for(2), Some(b1));
    jobs.start(b1, MS);
    assert_eq!(jobs.running_count(), 2, "sessions run side by side");

    jobs.finish(a1, 3, 5 * MS);
    assert_eq!(jobs.next_for(1), Some(a2));
    assert_eq!(state(&jobs, a1), JobState::Done { rows: 3 });
    assert_eq!(jobs.get(a1).unwrap().elapsed(Duration::MAX), Some(4 * MS));
}

#[test]
fn cancel_queued_removes_without_sending() {
    let mut jobs = Jobs::default();
    let running = jobs.enqueue(1, JobKind::Query, "a");
    let queued = jobs.enqueue(1, JobKind::Query, "b");
    jobs.start(running, MS);
    assert_eq!(jobs.cancel(queued, MS), Cancelled::Removed);
    assert!(jobs.get(queued).is_none());
    assert_eq!(jobs.cancel(running, 2 * MS), Cancelled::Running);
    assert_eq!(state(&jobs, running), JobState::Cancelled);
    assert_eq!(jobs.next_for(1), None);
}

#[test]
fn cancel_after_done_is_noop() {
    let mut jobs = Jobs::default();
    let id = jobs.enqueue(1, JobKind::Query, "a");
    jobs.start(id, MS);
    jobs.finish(id, 1, 2 * MS);
    assert_eq!(jobs.cancel(id, 3 * MS), Cancelled::Nothing);
    assert_eq!(state(&jobs, id), JobState::Done { rows: 1 });
    // A late failure does not change it either.
    jobs.fail(id, "late", 4 * MS);
    assert_eq!(state(&jobs, id), JobState::Done { rows: 1 });
}

#[test]
fn session_close_cancels_running_and_queued() {
    let mut jobs = Jobs::default();
    let a = jobs.enqueue(1, JobKind::Query, "a");
    let b = jobs.enqueue(1, JobKind::Query, "b");
    let other = jobs.enqueue(2, JobKind::Query, "c");
    jobs.start(a, MS);
    jobs.close_session(1, 2 * MS);
    assert_eq!(state(&jobs, a), JobState::Cancelled);
    assert_eq!(state(&jobs, b), JobState::Cancelled);
    assert_eq!(state(&jobs, other), JobState::Queued);
}

#[test]
fn finished_jobs_capped_at_50() {
    let mut jobs = Jobs::default();
    let waiting = jobs.enqueue(9, JobKind::Query, "waits");
    let first = jobs.enqueue(1, JobKind::Query, "0");
    jobs.start(first, MS);
    jobs.finish(first, 0, MS);
    for i in 1..60 {
        let id = jobs.enqueue(1, JobKind::Query, i.to_string());
        jobs.start(id, MS);
        jobs.finish(id, 0, MS);
    }
    assert_eq!(jobs.len(), FINISHED_KEPT + 1);
    assert!(jobs.get(first).is_none(), "the oldest went first");
    assert!(jobs.get(waiting).is_some(), "unfinished jobs are kept");
}

#[test]
fn progress_counts_rows_while_running() {
    let mut jobs = Jobs::default();
    let id = jobs.enqueue(1, JobKind::Query, "a");
    jobs.progress(id, 5);
    assert_eq!(state(&jobs, id), JobState::Queued);
    jobs.start(id, Duration::ZERO);
    jobs.progress(id, 12_000);
    assert_eq!(state(&jobs, id), JobState::Running { rows: 12_000 });
    assert_eq!(
        jobs.get(id).unwrap().elapsed(Duration::from_millis(2400)),
        Some(Duration::from_millis(2400))
    );
}

#[test]
fn format_elapsed_units() {
    assert_eq!(format_elapsed(Duration::from_micros(300)), "0 ms");
    assert_eq!(format_elapsed(Duration::from_millis(999)), "999 ms");
    assert_eq!(format_elapsed(Duration::from_millis(2400)), "2.4 s");
    assert_eq!(format_elapsed(Duration::from_secs(72)), "1 m 12 s");
}

#[test]
fn format_rows_thin_space_thousands() {
    assert_eq!(format_rows(0), "0");
    assert_eq!(format_rows(999), "999");
    assert_eq!(format_rows(1204), "1\u{2009}204");
    assert_eq!(format_rows(12_000_000), "12\u{2009}000\u{2009}000");
}

#[derive(Debug, Clone)]
enum Event {
    Enqueue(u8),
    Start(usize),
    Finish(usize),
    Fail(usize),
    Cancel(usize),
    Close(u8),
}

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![
        (0u8..3).prop_map(Event::Enqueue),
        (0usize..20).prop_map(Event::Start),
        (0usize..20).prop_map(Event::Finish),
        (0usize..20).prop_map(Event::Fail),
        (0usize..20).prop_map(Event::Cancel),
        (0u8..3).prop_map(Event::Close),
    ]
}

proptest! {
    #[test]
    fn prop_jobs_registry_invariants(events in prop::collection::vec(event(), 0..80)) {
        let mut jobs: Jobs<u8> = Jobs::default();
        let mut ids = Vec::new();
        let mut finished: Vec<(JobId, JobState)> = Vec::new();
        for (t, event) in events.into_iter().enumerate() {
            let now = Duration::from_millis(t as u64);
            let pick = |i: usize| ids.get(i % ids.len().max(1)).copied();
            match event {
                Event::Enqueue(s) => ids.push(jobs.enqueue(s, JobKind::Query, "x")),
                Event::Start(i) => if let Some(id) = pick(i) { jobs.start(id, now) },
                Event::Finish(i) => if let Some(id) = pick(i) { jobs.finish(id, 1, now) },
                Event::Fail(i) => if let Some(id) = pick(i) { jobs.fail(id, "e", now) },
                Event::Cancel(i) => if let Some(id) = pick(i) { jobs.cancel(id, now); },
                Event::Close(s) => jobs.close_session(s, now),
            }
            for s in 0..3 {
                let running = jobs.iter()
                    .filter(|j| j.session == s && matches!(j.state, JobState::Running { .. }))
                    .count();
                prop_assert!(running <= 1, "two jobs running in session {s}");
            }
            // A finished job never changes again (it may only be dropped).
            for (id, was) in &finished {
                if let Some(job) = jobs.get(*id) {
                    prop_assert_eq!(&job.state, was);
                }
            }
            for job in jobs.iter().filter(|j| j.state.is_finished()) {
                if !finished.iter().any(|(id, _)| *id == job.id) {
                    finished.push((job.id, job.state.clone()));
                }
            }
        }
    }
}
