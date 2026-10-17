//! The jobs drawer's keys: moving through the jobs, cancelling one, and
//! going to the session of one.

use super::{run, sessions};
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{JobsDrawer, State},
    },
    theme::JobState,
};

/// `J` opens the drawer on the newest job, or closes it.
pub(super) fn toggle(state: &mut State) {
    state.drawer = match state.drawer {
        Some(_) => None,
        None => Some(JobsDrawer::default()),
    };
}

/// What a `Jobs` binding does.
pub(super) fn run(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let Some(drawer) = &mut state.drawer else {
        return;
    };
    let last = state.jobs.len().saturating_sub(1);
    match action {
        Action::Up => drawer.selected = drawer.selected.saturating_sub(1),
        Action::Down => drawer.selected = (drawer.selected + 1).min(last),
        Action::Top => drawer.selected = 0,
        Action::Bottom => drawer.selected = last,
        Action::CancelQuery => {
            let Some(job) = selected(state) else { return };
            match job.state {
                JobState::Running { .. } => _ = run::cancel_session(state, job.session, fx),
                JobState::Queued => run::drop_queued(state, job.id),
                _ => {}
            }
        }
        Action::Open => {
            let Some(job) = selected(state) else { return };
            if state.sessions.get(job.session).is_some() {
                sessions::switch(state, job.session);
            }
            state.drawer = None;
        }
        _ => {}
    }
}

/// The selected job, counted from the newest.
fn selected(state: &State) -> Option<qry_core::jobs::Job<crate::app::state::SessionId>> {
    let at = state.drawer?.selected;
    state.jobs.iter().rev().nth(at).cloned()
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod jobs_tests;
