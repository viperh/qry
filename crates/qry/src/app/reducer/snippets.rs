//! Saved snippets: saving the statement under the cursor (ctrl-s), the
//! palette's `snippet: …` items, and the Manage snippets list with rename,
//! delete and edit. The list goes to `<config dir>/snippets.json` on every
//! change.

use qry_core::{
    snippets::{Snippet, validate},
    sql::{Statement, split, statement_at},
};

use super::{history::insert_into_editor, notices::notify};
use crate::{
    action::Action,
    app::{
        dialogs::{
            DialogState,
            confirm::Confirm,
            snippetform::{SnippetForm, SnippetMode, SnippetSave},
        },
        effects::Effect,
        history::{EditingSnippet, SnippetList},
        state::{Overlay, State},
    },
    notice::Notice,
};

/// The statement under the cursor, as text.
fn current_statement(state: &State) -> Option<String> {
    let session = state.sessions.active();
    let all: Vec<Statement> = split(session.dialect(), &session.editor.query());
    let at = statement_at(&all, session.editor.cursor_offset())?;
    Some(all[at].text.clone())
}

/// ctrl-s: asks for a name for the statement under the cursor. A snippet
/// being edited comes back with its own name and scope, so saving
/// overwrites it.
pub(super) fn open_save(state: &mut State, fx: &mut Vec<Effect>) {
    let Some(sql) = current_statement(state) else {
        notify(state, Notice::warning("There is no statement to save"), fx);
        return;
    };
    let session = state.sessions.active();
    let here = session.connection_id.clone();
    let editing = state
        .editing_snippet
        .as_ref()
        .filter(|e| e.session == session.id);
    let (name, connection) = match editing {
        Some(e) => (e.name.clone(), e.connection.clone()),
        None => (String::new(), Some(here.clone())),
    };
    let mode = SnippetMode::Save {
        sql,
        replacing: editing.map(|e| (e.name.clone(), e.connection.clone())),
    };
    let form = SnippetForm::new(mode, &name, connection.as_deref(), &here);
    state.open_dialog(DialogState::Snippet(form));
    state.awaiting = None;
}

fn position(state: &State, name: &str, connection: Option<&str>) -> Option<usize> {
    state
        .snippets
        .iter()
        .position(|s| s.name == name && s.connection.as_deref() == connection)
}

/// The snippet dialog was submitted: saves or renames, or says why not.
/// `Ok(true)` when it came from the Manage list, to go back to it.
pub(super) fn submitted(
    state: &mut State,
    save: SnippetSave,
    fx: &mut Vec<Effect>,
) -> Result<bool, String> {
    let (snippet, replacing, from_list) = match save.mode {
        SnippetMode::Save { sql, replacing } => {
            let replacing = replacing
                .and_then(|(name, connection)| position(state, &name, connection.as_deref()));
            let tags = replacing
                .map(|i| state.snippets[i].tags.clone())
                .unwrap_or_default();
            let snippet = Snippet {
                name: save.name,
                sql,
                connection: save.connection,
                tags,
            };
            (snippet, replacing, false)
        }
        SnippetMode::Rename { name, connection } => {
            let Some(i) = position(state, &name, connection.as_deref()) else {
                return Err(format!("There is no snippet called \"{name}\" any more"));
            };
            let mut snippet = state.snippets[i].clone();
            snippet.name = save.name;
            snippet.connection = save.connection;
            (snippet, Some(i), true)
        }
    };
    validate(&state.snippets, &snippet, replacing).map_err(|e| e.to_string())?;
    let message = format!("Saved snippet \"{}\"", snippet.name);
    if let Some(e) = state.editing_snippet.as_mut()
        && replacing.is_some_and(|i| {
            state.snippets[i].name == e.name && state.snippets[i].connection == e.connection
        })
    {
        e.name = snippet.name.clone();
        e.connection = snippet.connection.clone();
    }
    match replacing {
        Some(i) => state.snippets[i] = snippet,
        None => state.snippets.push(snippet),
    }
    fx.push(Effect::SaveSnippets(state.snippets.clone()));
    notify(state, Notice::success(message), fx);
    Ok(from_list)
}

/// A confirmed delete.
pub(super) fn delete(
    state: &mut State,
    name: &str,
    connection: Option<&str>,
    fx: &mut Vec<Effect>,
) {
    if let Some(i) = position(state, name, connection) {
        state.snippets.remove(i);
        if state
            .editing_snippet
            .as_ref()
            .is_some_and(|e| e.name == name && e.connection.as_deref() == connection)
        {
            state.editing_snippet = None;
        }
        fx.push(Effect::SaveSnippets(state.snippets.clone()));
        notify(
            state,
            Notice::info(format!("Deleted snippet \"{name}\"")),
            fx,
        );
    }
}

/// The file was read. Snippets saved before it came are kept, unless the
/// file has one of the same name and scope.
pub(super) fn loaded(state: &mut State, mut list: Vec<Snippet>) {
    for snippet in std::mem::take(&mut state.snippets) {
        if !list
            .iter()
            .any(|s| s.name == snippet.name && s.connection == snippet.connection)
        {
            list.push(snippet);
        }
    }
    state.snippets = list;
    refresh(state);
}

/// Puts snippet `i` in the editor as a new statement, and runs it if
/// `run`: what picking it in the palette does.
pub(super) fn insert(state: &mut State, i: usize, run: bool, fx: &mut Vec<Effect>) {
    if let Some(sql) = state.snippets.get(i).map(|s| s.sql.clone()) {
        insert_into_editor(state, &sql, run, fx);
    }
}

/// The list's rows: the active connection's snippets first, then the
/// global ones, then other connections', each by name.
fn rows(state: &State) -> Vec<usize> {
    let here = &state.sessions.active().connection_id;
    let mut rows: Vec<usize> = (0..state.snippets.len()).collect();
    rows.sort_by_key(|&i| {
        let s = &state.snippets[i];
        let rank = match &s.connection {
            Some(c) if c == here => 0,
            None => 1,
            Some(_) => 2,
        };
        (rank, s.name.to_lowercase())
    });
    rows
}

/// Opens Manage snippets.
pub(super) fn open_manage(state: &mut State) {
    let list = SnippetList {
        rows: rows(state),
        ..SnippetList::default()
    };
    state.overlays.retain(|o| matches!(o, Overlay::Dialog(_)));
    state.overlays.push(Overlay::Snippets(list));
}

/// Lists the snippets again after a change, keeping the selection's place.
fn refresh(state: &mut State) {
    let rows = rows(state);
    if let Some(Overlay::Snippets(list)) = state.overlays.last_mut() {
        list.rows = rows;
        list.selected = list.selected.min(list.rows.len().saturating_sub(1));
    }
}

fn list_mut(state: &mut State) -> Option<&mut SnippetList> {
    match state.overlays.last_mut() {
        Some(Overlay::Snippets(list)) => Some(list),
        _ => None,
    }
}

fn close(state: &mut State) {
    state
        .overlays
        .retain(|o| !matches!(o, Overlay::Snippets(_)));
}

/// The selected snippet's index into `State::snippets`.
fn selected(state: &mut State) -> Option<usize> {
    let list = list_mut(state)?;
    list.rows.get(list.selected).copied()
}

/// What a `Palette` binding does while the list is on top.
pub(super) fn run_key(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let Some(list) = list_mut(state) else {
        return;
    };
    let last = list.rows.len().saturating_sub(1);
    match action {
        Action::Up => list.selected = list.selected.saturating_sub(1),
        Action::Down => list.selected = (list.selected + 1).min(last),
        Action::PageUp => list.selected = list.selected.saturating_sub(crate::palette::MAX_ROWS),
        Action::PageDown => list.selected = (list.selected + crate::palette::MAX_ROWS).min(last),
        Action::Submit => edit(state),
        Action::SubmitAndRun => {
            if let Some(i) = selected(state) {
                close(state);
                insert(state, i, true, fx);
            }
        }
        Action::DeleteEntry => _ = shortcut(state, 'd'),
        Action::Cancel => close(state),
        _ => {}
    }
}

/// `r` renames the selected snippet, `d` deletes it (after a coral
/// confirm) and `e` edits its SQL. Returns whether the key was one of them.
pub(super) fn shortcut(state: &mut State, c: char) -> bool {
    let Some(i) = selected(state) else {
        return false;
    };
    let snippet = state.snippets[i].clone();
    match c {
        'r' => {
            close(state);
            let here = state.sessions.active().connection_id.clone();
            let mode = SnippetMode::Rename {
                name: snippet.name.clone(),
                connection: snippet.connection.clone(),
            };
            let form = SnippetForm::new(mode, &snippet.name, snippet.connection.as_deref(), &here);
            state.open_dialog(DialogState::Snippet(form));
            state.awaiting = None;
        }
        'd' => {
            close(state);
            let confirm = Confirm::delete_snippet(&snippet.name, snippet.connection);
            state.open_dialog(DialogState::Confirm(confirm));
            state.awaiting = None;
        }
        'e' => edit(state),
        _ => return false,
    }
    true
}

/// Puts the selected snippet's SQL in the editor to be changed; ctrl-s
/// there then saves over it.
fn edit(state: &mut State) {
    let Some(i) = selected(state) else {
        return;
    };
    close(state);
    let snippet = state.snippets[i].clone();
    state.editing_snippet = Some(EditingSnippet {
        session: state.sessions.active,
        name: snippet.name,
        connection: snippet.connection,
    });
    let mut none = Vec::new();
    insert_into_editor(state, &snippet.sql, false, &mut none);
}
