//! Telling the user things: every notice goes into the history and the
//! toast queue, and the Messages dialog lists the history.

use crate::{
    action::Action,
    app::{
        effects::Effect,
        event::Timer,
        state::{MessagesState, Overlay, State},
    },
    notice::{Expiry, Notice},
};

/// Logs `notice` and queues its toast.
pub(crate) fn notify(state: &mut State, notice: Notice, fx: &mut Vec<Effect>) {
    state.messages.push(state.clock, notice.clone());
    let expiry = state.toasts.push(notice);
    schedule(expiry, fx);
}

fn schedule(expiry: Option<Expiry>, fx: &mut Vec<Effect>) {
    if let Some(Expiry { generation, after }) = expiry {
        fx.push(Effect::ScheduleTimer {
            timer: Timer::Toast(generation),
            after,
        });
    }
}

/// The visible toast's time ran out.
pub(super) fn expired(state: &mut State, generation: u64, fx: &mut Vec<Effect>) {
    let next = state.toasts.expire(generation);
    schedule(next, fx);
}

/// Esc with nothing else to close: the toast goes early.
pub(super) fn dismiss(state: &mut State, fx: &mut Vec<Effect>) {
    let next = state.toasts.dismiss();
    schedule(next, fx);
}

/// Opens the Messages dialog on the newest entry, in place of any other
/// overlay.
pub(super) fn show_messages(state: &mut State) {
    state.overlays.clear();
    state.awaiting = None;
    state
        .overlays
        .push(Overlay::Messages(MessagesState::default()));
}

fn messages_mut(state: &mut State) -> Option<&mut MessagesState> {
    match state.overlays.last_mut() {
        Some(Overlay::Messages(messages)) => Some(messages),
        _ => None,
    }
}

/// Whether the Messages dialog is on top.
pub(super) fn showing_messages(state: &State) -> bool {
    matches!(state.overlays.last(), Some(Overlay::Messages(_)))
}

/// What a `Help`-context key does in the Messages dialog: move the
/// selection, or close.
pub(super) fn act(state: &mut State, action: Action) {
    let last = state.messages.len().saturating_sub(1);
    let Some(messages) = messages_mut(state) else {
        return;
    };
    let page = messages.page.get().max(1);
    let selected = &mut messages.selected;
    match action {
        Action::Down => *selected = (*selected + 1).min(last),
        Action::Up => *selected = selected.saturating_sub(1),
        Action::PageDown => *selected = (*selected + page).min(last),
        Action::PageUp => *selected = selected.saturating_sub(page),
        Action::Top => *selected = 0,
        Action::Bottom => *selected = last,
        Action::Cancel | Action::Help | Action::ShowMessages => {
            state
                .overlays
                .retain(|o| !matches!(o, Overlay::Messages(_)));
        }
        _ => {}
    }
}

/// `y` copies the selected message, `c` clears the history. Returns
/// whether the key was one of them.
pub(super) fn shortcut(state: &mut State, c: char, fx: &mut Vec<Effect>) -> bool {
    let Some(selected) = messages_mut(state).map(|m| m.selected) else {
        return false;
    };
    match c {
        'y' => {
            if let Some((_, notice)) = state.messages.newest_first().nth(selected) {
                fx.push(Effect::CopyToClipboard(notice.text.clone()));
            }
            true
        }
        'c' => {
            state.messages.clear();
            if let Some(messages) = messages_mut(state) {
                messages.selected = 0;
            }
            true
        }
        _ => false,
    }
}
