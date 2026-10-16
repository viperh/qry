//! How each event changes the state, one file per concern. Every function
//! takes the state and the list of effects to add to.

mod chrome;
mod db;
mod dialogs;
mod editor;
mod files;
pub(crate) mod help;
mod keys;
mod mouse;
pub(crate) mod notices;
pub(crate) mod palette;
mod results;
mod run;
mod sessions;
mod tree;

use super::{
    effects::Effect,
    event::{Timer, UiEvent},
    state,
    state::{Focus, SessionId, State, scratch_record},
};
use crate::{action::Action, connections::StoredConnection, keymap::KeyContext};

pub(super) fn handle(state: &mut State, event: UiEvent) -> Vec<Effect> {
    let mut fx = Vec::new();
    // Nothing on screen follows the tick or the clock.
    state.dirty |= !matches!(event, UiEvent::Tick(_) | UiEvent::Clock(_));
    // What cannot change the tree's rows leaves them be.
    let tree_may_change = !matches!(
        event,
        UiEvent::Tick(_)
            | UiEvent::Clock(_)
            | UiEvent::Timer(Timer::Toast(_) | Timer::Spinner(_) | Timer::Tree(_))
    );
    match event {
        UiEvent::Started => start(state, &mut fx),
        UiEvent::Key(key) => keys::key(state, key, &mut fx),
        UiEvent::Mouse(mouse) => mouse::mouse(state, mouse, &mut fx),
        UiEvent::Paste(text) => paste(state, &text),
        UiEvent::Resize(w, h) => state.size = (w, h),
        UiEvent::Tick(_) => {}
        UiEvent::Clock(now) => state.clock = now,
        UiEvent::Notice(notice) => notices::notify(state, notice, &mut fx),
        UiEvent::Timer(Timer::Toast(n)) => notices::expired(state, n, &mut fx),
        UiEvent::Timer(Timer::KeySequence(n)) => keys::timed_out(state, n, &mut fx),
        UiEvent::Timer(Timer::Spinner(n)) => dialogs::spin(state, n, &mut fx),
        UiEvent::Timer(Timer::Tree(n)) => tree::spin(state, n, &mut fx),
        // A closed session's worker may still answer; nobody is listening.
        UiEvent::Db(id, answer) if state.sessions.get(id).is_some() => {
            db::answer(state, id, answer, &mut fx);
        }
        UiEvent::Db(..) => {}
        UiEvent::File(answer) => files::answer(state, answer, &mut fx),
    }
    if tree_may_change {
        tree::after(state, &mut fx);
    }
    fx
}

/// Every run starts on an empty in-memory SQLite database, session 1. The
/// tree lists it like any other connection, but it is never saved. The
/// saved connections are read from disk alongside.
fn start(state: &mut State, fx: &mut Vec<Effect>) {
    fx.push(Effect::LoadConnections);
    let scratch = scratch_record();
    tree::attempt(state, &scratch, fx);
    if state.sessions.get(SessionId::FIRST).is_some() {
        fx.push(Effect::OpenSession(SessionId::FIRST, Box::new(scratch)));
    }
}

/// Carries out what a key is bound to. `context` is the innermost one the
/// key was looked up in: the actions that move, edit or pick act on it.
fn act(state: &mut State, context: KeyContext, action: Action, fx: &mut Vec<Effect>) {
    match action {
        Action::Quit => fx.push(Effect::Quit),
        // Only unix has job control to suspend to.
        Action::Suspend => {
            if cfg!(unix) {
                fx.push(Effect::Suspend);
            }
        }
        Action::ClearScreen => fx.push(Effect::ClearScreen),
        Action::Help if notices::showing_messages(state) => notices::act(state, action),
        Action::Help => help::toggle(state),
        Action::ShowMessages if notices::showing_messages(state) => notices::act(state, action),
        Action::ShowMessages => notices::show_messages(state),
        Action::DismissToast => notices::dismiss(state, fx),
        Action::FocusNext => state.focus = state.focus.next(),
        Action::FocusPrev => state.focus = state.focus.prev(),
        Action::FocusTree => state.focus = Focus::Tree,
        Action::FocusEditor => state.focus = Focus::Editor,
        Action::FocusResults => state.focus = Focus::Results,
        Action::NewConnection
        | Action::ImportConnections
        | Action::ExportConnections
        | Action::ExportResults => dialogs::open(state, action),
        Action::SwitchSession(_)
        | Action::NextSession
        | Action::PrevSession
        | Action::CloseSession => sessions::run(state, action, fx),
        Action::GrowEditor | Action::ShrinkEditor | Action::ToggleMaximise => {
            chrome::run(state, action);
        }
        Action::ToggleHints => state.chrome.hints = !state.chrome.hints,
        Action::ToggleInspector => state.chrome.inspector = !state.chrome.inspector,
        Action::Palette => palette::toggle(state),
        // The jobs drawer comes with T305.
        Action::ToggleJobs => {}
        action => match context {
            KeyContext::Tree => tree::run(state, action, fx),
            KeyContext::Editor => editor::run(state, action, fx),
            KeyContext::Results => results::run(state, action),
            KeyContext::Dialog => dialogs::act(state, action, fx),
            KeyContext::Help if notices::showing_messages(state) => notices::act(state, action),
            KeyContext::Help => help::act(state, action),
            KeyContext::Palette => palette::run_key(state, action, fx),
            KeyContext::Global | KeyContext::Inspector | KeyContext::Jobs => {}
        },
    }
}

/// What `sql` does to the transaction, judged by its last statement:
/// `Some(true)` begins one, `Some(false)` ends it, `None` leaves it be.
pub(crate) fn tx_change(sql: &str) -> Option<bool> {
    let last = sql
        .split(';')
        .map(str::trim)
        .rfind(|statement| !statement.is_empty())?
        .to_lowercase();
    let words: Vec<&str> = last.split_whitespace().take(2).collect();
    match words.as_slice() {
        ["begin", ..] | ["start", "transaction"] => Some(true),
        // Rolling back to a savepoint stays in the transaction.
        ["rollback", "to"] => None,
        ["commit" | "rollback" | "end" | "abort", ..] => Some(false),
        _ => None,
    }
}

/// Opens `record` as the New Connection modal submitted it, so the tree can
/// list it: the session already on it connects again, taking any edit on,
/// or a new one opens. Returns the session whose answer the modal waits for.
fn connect(state: &mut State, record: StoredConnection, fx: &mut Vec<Effect>) -> SessionId {
    tree::attempt(state, &record, fx);
    match state.sessions.for_connection(&record.id) {
        Some(id) => {
            sessions::switch(state, id);
            sessions::reconnect(state, id, &record, fx);
            id
        }
        None => sessions::open(state, &record, fx),
    }
}

/// Pasted text goes where typing would: the open dialog's text field, the
/// palette's query, or the editor.
fn paste(state: &mut State, text: &str) {
    match state.overlays.last() {
        Some(state::Overlay::Dialog(_)) => {
            if let Some(dialog) = state.dialog_mut()
                && dialog.form().is_typing()
            {
                for c in text.chars().filter(|c| !c.is_control()) {
                    dialog.form_mut().handle_key(
                        crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(c)),
                        None,
                    );
                }
            }
        }
        Some(state::Overlay::Palette(_)) => {
            for c in text.chars().filter(|c| !c.is_control()) {
                palette::type_char(state, c);
            }
        }
        Some(_) => {}
        None if state.focus == Focus::Editor => editor::paste(state.editor_mut(), text),
        None => {}
    }
}
