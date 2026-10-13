//! How each event changes the state, one file per concern. Every function
//! takes the state and the list of effects to add to.

mod db;
mod dialogs;
mod editor;
mod files;
pub(crate) mod help;
mod keys;
mod results;
mod sessions;
mod tree;

use super::{
    effects::Effect,
    event::{Timer, UiEvent},
    state::{Focus, SessionId, State, scratch_record},
};
use crate::{action::Action, connections::StoredConnection, db::DbCommand, keymap::KeyContext};

pub(super) fn handle(state: &mut State, event: UiEvent) -> Vec<Effect> {
    let mut fx = Vec::new();
    // Nothing reacts to a tick yet.
    state.dirty |= !matches!(event, UiEvent::Tick(_));
    match event {
        UiEvent::Started => start(state, &mut fx),
        UiEvent::Key(key) => keys::key(state, key, &mut fx),
        UiEvent::Resize(w, h) => state.size = (w, h),
        UiEvent::Tick(_) => {}
        UiEvent::Timer(Timer::KeySequence(n)) => keys::timed_out(state, n, &mut fx),
        // A closed session's worker may still answer; nobody is listening.
        UiEvent::Db(id, answer) if state.sessions.get(id).is_some() => {
            db::answer(state, id, answer, &mut fx);
        }
        UiEvent::Db(..) => {}
        UiEvent::File(answer) => files::answer(state, answer, &mut fx),
    }
    restyle(state);
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
        Action::Help => help::toggle(state),
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
        // The palette (T206), the jobs drawer (T305) and the hint bar
        // (T202) come later.
        Action::Palette | Action::ToggleJobs | Action::ToggleHints => {}
        action => match context {
            KeyContext::Tree => tree::run(state, action, fx),
            KeyContext::Editor => editor::run(state, action, fx),
            KeyContext::Results => results::run(state, action),
            KeyContext::Dialog => dialogs::act(state, action, fx),
            KeyContext::Help => help::act(state, action),
            KeyContext::Global | KeyContext::Inspector | KeyContext::Palette | KeyContext::Jobs => {
            }
        },
    }
}

/// Runs `sql` on the active session, unless there is nothing to run.
fn execute(state: &mut State, sql: String, fx: &mut Vec<Effect>) {
    if !sql.trim().is_empty() {
        let session = state.sessions.active_mut();
        session.running += 1;
        fx.push(Effect::Db(session.id, DbCommand::Query(sql)));
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

/// Styles the editor for the focus and the theme. The text area keeps its
/// own scroll position as it draws, so the view draws it as it is, and its
/// styling is set here.
pub(super) fn restyle(state: &mut State) {
    let focused = state.focus == Focus::Editor;
    let theme = state.theme;
    editor::restyle(state.editor_mut(), focused, &theme);
}
