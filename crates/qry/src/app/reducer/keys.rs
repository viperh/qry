//! Where a key goes.
//!
//! First the `keybindings` section for whatever is open is looked up, the
//! keys pressed since the last tick included for multi-key bindings. Then the
//! key goes to the help if it is open, else to the open modal, else to the
//! focused pane. The bound command, if any, is carried out last.

use crossterm::event::KeyEvent;

use super::{act, dialogs, editor, help, results, tree};
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{Focus, State},
    },
    keymap::{FormCommand, HelpCommand},
};

pub(super) fn key(state: &mut State, key: KeyEvent, fx: &mut Vec<Effect>) {
    let bound = binding(state, key);

    if state.help().is_some() {
        match state.config.panes.help.get(key) {
            Some(HelpCommand::Close) => help::close(state),
            Some(command) => help::scroll(state, command),
            None => {}
        }
    } else if state.dialog().is_some() {
        let command = state.config.panes.form.get(key);
        if command == Some(FormCommand::Submit) {
            dialogs::submit(state, fx);
        } else if let Some(dialog) = state.dialog_mut() {
            dialog.form_mut().handle_key(key, command);
        }
    } else {
        match state.focus {
            Focus::Tree => tree::key(state, key, fx),
            Focus::Editor => editor::key(state, key, fx),
            Focus::Results => results::key(state, key),
        }
    }

    if let Some(action) = bound {
        act(state, action, fx);
    }
}

/// The command the current section binds `key` to, counting the keys
/// pressed since the last tick for multi-key bindings.
fn binding(state: &mut State, key: KeyEvent) -> Option<Action> {
    let keymap = state.config.keybindings.0.get(&state.key_section())?;
    if let Some(action) = keymap.get(&vec![key]) {
        return Some(action.clone());
    }
    state.pending_keys.push(key);
    keymap.get(&state.pending_keys).cloned()
}
