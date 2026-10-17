//! Query jobs: every statement the user starts, from queued to done, and
//! how their times and row counts read.
//!
//! [`Jobs`] is pure bookkeeping. Times are what the caller says "now" is,
//! as time since some start of its own, so a virtual clock drives it in
//! tests. A session runs one job at a time; the caller asks
//! [`Jobs::next_for`] which to send next.

use std::{collections::VecDeque, time::Duration};

/// How many finished jobs are kept; older ones are dropped.
pub const FINISHED_KEPT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Query,
    FetchMore,
    Export,
    Schema,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running { rows: u64 },
    Done { rows: u64 },
    Failed(String),
    Cancelled,
}

impl JobState {
    /// Done, failed or cancelled: nothing changes it any more.
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed(_) | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job<S> {
    pub id: JobId,
    pub session: S,
    pub kind: JobKind,
    pub sql: String,
    pub started: Option<Duration>,
    pub finished: Option<Duration>,
    pub state: JobState,
}

impl<S> Job<S> {
    /// How long it ran: until it finished, or until `now`.
    pub fn elapsed(&self, now: Duration) -> Option<Duration> {
        let started = self.started?;
        Some(self.finished.unwrap_or(now).saturating_sub(started))
    }
}

/// What [`Jobs::cancel`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cancelled {
    /// It was waiting, and is gone without ever being sent.
    Removed,
    /// It was running: the caller has to stop it on the server.
    Running,
    /// It had already finished, or is unknown.
    Nothing,
}

/// The jobs of every session, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jobs<S> {
    jobs: VecDeque<Job<S>>,
    made: u64,
}

impl<S> Default for Jobs<S> {
    fn default() -> Self {
        Self {
            jobs: VecDeque::new(),
            made: 0,
        }
    }
}

impl<S: Copy + Eq> Jobs<S> {
    /// Queues a job for `session`.
    pub fn enqueue(&mut self, session: S, kind: JobKind, sql: impl Into<String>) -> JobId {
        self.made += 1;
        let id = JobId(self.made);
        self.jobs.push_back(Job {
            id,
            session,
            kind,
            sql: sql.into(),
            started: None,
            finished: None,
            state: JobState::Queued,
        });
        id
    }

    pub fn get(&self, id: JobId) -> Option<&Job<S>> {
        self.jobs.iter().find(|job| job.id == id)
    }

    fn get_mut(&mut self, id: JobId) -> Option<&mut Job<S>> {
        self.jobs.iter_mut().find(|job| job.id == id)
    }

    /// Every job, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Job<S>> + ExactSizeIterator {
        self.jobs.iter()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The job `session` is running, if any.
    pub fn running(&self, session: S) -> Option<&Job<S>> {
        self.jobs
            .iter()
            .find(|job| job.session == session && matches!(job.state, JobState::Running { .. }))
    }

    /// How many jobs are running, in every session.
    pub fn running_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| matches!(job.state, JobState::Running { .. }))
            .count()
    }

    /// The job `session` should send next: its oldest queued one, unless
    /// one is running.
    pub fn next_for(&self, session: S) -> Option<JobId> {
        if self.running(session).is_some() {
            return None;
        }
        self.jobs
            .iter()
            .find(|job| job.session == session && job.state == JobState::Queued)
            .map(|job| job.id)
    }

    /// Job `id` was sent at `now`. Ignored unless it is queued and its
    /// session runs nothing else.
    pub fn start(&mut self, id: JobId, now: Duration) {
        let Some(session) = self.get(id).map(|job| job.session) else {
            return;
        };
        if self.running(session).is_some() {
            return;
        }
        if let Some(job) = self.get_mut(id)
            && job.state == JobState::Queued
        {
            job.state = JobState::Running { rows: 0 };
            job.started = Some(now);
        }
    }

    /// Job `id` has read `rows` rows so far.
    pub fn progress(&mut self, id: JobId, rows: u64) {
        if let Some(job) = self.get_mut(id)
            && let JobState::Running { rows: seen } = &mut job.state
        {
            *seen = rows;
        }
    }

    /// Job `id` finished with `rows` rows.
    pub fn finish(&mut self, id: JobId, rows: u64, now: Duration) {
        self.end(id, JobState::Done { rows }, now);
    }

    /// Job `id` failed with `message`.
    pub fn fail(&mut self, id: JobId, message: impl Into<String>, now: Duration) {
        self.end(id, JobState::Failed(message.into()), now);
    }

    fn end(&mut self, id: JobId, state: JobState, now: Duration) {
        if let Some(job) = self.get_mut(id)
            && !job.state.is_finished()
        {
            job.started.get_or_insert(now);
            job.state = state;
            job.finished = Some(now);
        }
        self.trim();
    }

    /// Cancels job `id`: a queued one is removed, a running one marked
    /// cancelled for the caller to stop, a finished one left alone.
    pub fn cancel(&mut self, id: JobId, now: Duration) -> Cancelled {
        let Some(job) = self.get(id) else {
            return Cancelled::Nothing;
        };
        match job.state {
            JobState::Queued => {
                self.jobs.retain(|job| job.id != id);
                Cancelled::Removed
            }
            JobState::Running { .. } => {
                self.end(id, JobState::Cancelled, now);
                Cancelled::Running
            }
            _ => Cancelled::Nothing,
        }
    }

    /// Session `session` closed: its running and queued jobs are cancelled.
    pub fn close_session(&mut self, session: S, now: Duration) {
        let open: Vec<JobId> = self
            .jobs
            .iter()
            .filter(|job| job.session == session && !job.state.is_finished())
            .map(|job| job.id)
            .collect();
        for id in open {
            self.end(id, JobState::Cancelled, now);
        }
    }

    /// Drops the oldest finished jobs past [`FINISHED_KEPT`].
    fn trim(&mut self) {
        let mut finished = self.jobs.iter().filter(|j| j.state.is_finished()).count();
        while finished > FINISHED_KEPT {
            if let Some(at) = self.jobs.iter().position(|j| j.state.is_finished()) {
                self.jobs.remove(at);
            }
            finished -= 1;
        }
    }
}

/// How long something took, in the unit that reads best: `0 ms`, `38 ms`,
/// `2.4 s`, `1 m 12 s`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let ms = elapsed.as_millis();
    match ms {
        0..1000 => format!("{ms} ms"),
        1000..60_000 => format!("{:.1} s", elapsed.as_secs_f64()),
        _ => {
            let secs = elapsed.as_secs();
            format!("{} m {} s", secs / 60, secs % 60)
        }
    }
}

/// The thousands separator: a thin space.
pub const THIN_SPACE: char = '\u{2009}';

/// `n` with a thin space between each three digits: `1 204`.
pub fn format_rows(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(THIN_SPACE);
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod jobs_tests;
