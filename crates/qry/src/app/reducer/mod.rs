//! How each event changes the state, one file per concern. Every function
//! takes the state and the list of effects to add to.

mod db;
mod dialogs;
mod editor;
mod files;
pub(crate) mod help;
mod keys;
mod results;
mod tree;

use super::{
    effects::Effect,
    event::{Timer, UiEvent},
    state::{Focus, State},
};
use crate::{
    action::Action,
    connections::{Driver, Secret, StoredConnection},
    db::DbCommand,
    keymap::KeyContext,
};

/// The id of the in-memory database every run starts on. It is never saved,
/// so a fixed id cannot clash with a stored one's keychain entry.
pub(crate) const SCRATCH_ID: &str = "scratch";

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
        // Only the one session exists; anything else is a stray.
        UiEvent::Db(id, answer) if id == state.session => db::answer(state, answer, &mut fx),
        UiEvent::Db(..) => {}
        UiEvent::File(answer) => files::answer(state, answer, &mut fx),
    }
    restyle(state);
    fx
}

/// Every run starts on an empty in-memory SQLite database. The tree lists it
/// like any other connection, but it is never saved. The saved connections
/// are read from disk alongside.
fn start(state: &mut State, fx: &mut Vec<Effect>) {
    fx.push(Effect::LoadConnections);
    let mut scratch = StoredConnection::with_id(SCRATCH_ID, "scratch", Driver::Sqlite);
    scratch.path = Some(":memory:".to_string());
    scratch.secret = Secret::None;
    scratch.ephemeral = true;
    tree::attempt(state, &scratch, fx);
    fx.push(Effect::OpenSession(state.session, Box::new(scratch)));
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
        // The palette (T206), sessions (T103), the jobs drawer (T305) and
        // the hint bar (T202) come later.
        Action::Palette
        | Action::SwitchSession(_)
        | Action::NextSession
        | Action::PrevSession
        | Action::CloseSession
        | Action::ToggleJobs
        | Action::ToggleHints => {}
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

/// Runs `sql` on the session, unless there is nothing to run.
fn execute(state: &State, sql: String, fx: &mut Vec<Effect>) {
    if !sql.trim().is_empty() {
        fx.push(Effect::Db(state.session, DbCommand::Query(sql)));
    }
}

/// Opens `record` on the session. Every connection passes through here,
/// whether it came from the New Connection modal or from the tree, so the
/// tree can list it.
fn connect(state: &mut State, record: StoredConnection, fx: &mut Vec<Effect>) {
    fx.push(Effect::Db(
        state.session,
        DbCommand::Connect(Box::new(record.clone())),
    ));
    tree::attempt(state, &record, fx);
}

/// Styles the editor for the focus and the theme. The text area keeps its
/// own scroll position as it draws, so the view draws it as it is, and its
/// styling is set here.
pub(super) fn restyle(state: &mut State) {
    editor::restyle(
        &mut state.editor,
        state.focus == Focus::Editor,
        &state.theme,
    );
}
