//! Where a key goes.
//!
//! The key is looked up in the keymap with the state's context stack (see
//! [`State::contexts`]), together with the keys of a sequence typed before
//! it. A binding it completes is carried out. One it only starts waits, in
//! `State::pending_keys`, for the next key or for [`SEQUENCE_TIMEOUT`]. A key
//! that is bound to nothing goes to the focused pane or the open dialog as
//! typing.
//!
//! A dialog's text field is the exception: a plain character is typed there
//! before anything is looked up, so Space types a space rather than picking
//! a choice.

use std::time::Duration;

use crossterm::event::KeyEvent;

use super::act;
use crate::{
    app::{
        dialogs::textinput::typed_char,
        effects::Effect,
        event::Timer,
        state::{Overlay, State},
    },
    keymap::{KeyContext, Lookup, normalise},
};

/// How long a half-typed sequence such as `g g` waits for its next key.
pub(crate) const SEQUENCE_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) fn key(state: &mut State, key: KeyEvent, fx: &mut Vec<Effect>) {
    if typing_into_dialog(state, key) {
        if let Some(dialog) = state.dialog_mut() {
            dialog.form_mut().handle_key(key, None);
        }
        return;
    }

    let contexts = state.contexts();
    state.pending_keys.push(normalise(key));
    match state.config.keymap.resolve(&contexts, &state.pending_keys) {
        Lookup::Match(action) => {
            state.pending_keys.clear();
            act(state, contexts[0], action, fx);
        }
        Lookup::Prefix => {
            state.key_timer += 1;
            fx.push(Effect::ScheduleTimer {
                timer: Timer::KeySequence(state.key_timer),
                after: SEQUENCE_TIMEOUT,
            });
        }
        // The sequence broke off: what the keys before this one are bound
        // to runs, and this key starts afresh.
        Lookup::None if state.pending_keys.len() > 1 => {
            state.pending_keys.pop();
            finish_sequence(state, fx);
            self::key(state, key, fx);
        }
        Lookup::None => {
            state.pending_keys.clear();
            unbound(state, contexts[0], key);
        }
    }
}

/// The wait for the rest of a sequence ran out: whatever the keys typed so
/// far are bound to runs, as a binding that starts a longer one only does
/// then. A timer from an earlier wait changes nothing.
pub(super) fn timed_out(state: &mut State, timer: u64, fx: &mut Vec<Effect>) {
    if timer == state.key_timer {
        finish_sequence(state, fx);
    }
}

fn finish_sequence(state: &mut State, fx: &mut Vec<Effect>) {
    let keys = std::mem::take(&mut state.pending_keys);
    if keys.is_empty() {
        return;
    }
    let contexts = state.contexts();
    if let Some(action) = state.config.keymap.exact(&contexts, &keys) {
        act(state, contexts[0], action, fx);
    }
}

/// Whether `key` is a plain character for the open dialog's focused text
/// field, with nothing on top of the dialog.
fn typing_into_dialog(state: &State, key: KeyEvent) -> bool {
    let on_top = matches!(state.overlays.last(), Some(Overlay::Dialog(_)));
    on_top
        && typed_char(key).is_some()
        && state
            .dialog()
            .is_some_and(|dialog| dialog.form().is_typing())
}

/// A key bound to nothing: the editor types it. (A dialog's text field
/// took its characters before the lookup.)
fn unbound(state: &mut State, context: KeyContext, key: KeyEvent) {
    if context == KeyContext::Editor
        && let Some(c) = typed_char(key)
    {
        state.editor.textarea.insert_char(c);
    }
}
