//! The command palette: gathering its items when it opens, its keys, and
//! running what was picked through the same paths as the keys do.

use super::{act, notices::notify, sessions};
use crate::{
    action::Action,
    app::{
        effects::Effect,
        state::{Overlay, State},
    },
    keymap::{KeyContext, display_chord},
    notice::Notice,
    palette::{ArgKind, PaletteItem, PaletteState, PaletteTarget, remember},
    theme::Theme,
};

/// Every item the palette offers now: the commands with a palette label,
/// the stored connections, the open sessions and the parametrised
/// commands. Each command's key is the one that runs it now.
pub(crate) fn items(state: &State) -> Vec<PaletteItem> {
    use strum::IntoEnumIterator;
    let keymap = &state.config.keymap;
    // Its own pane's key, else a global one, else the first pane's that
    // has one: Export result… is ctrl-e in the results.
    let key_for = |action: &Action| {
        [
            action.home_context(),
            KeyContext::Global,
            KeyContext::Tree,
            KeyContext::Editor,
            KeyContext::Results,
        ]
        .into_iter()
        .map(|context| keymap.chords_for(context, action))
        .find(|chords| !chords.is_empty())
        .map(|chords| display_chord(&chords))
    };
    let mut items: Vec<PaletteItem> = Action::iter()
        // With no result there is nothing to export.
        .filter(|action| *action != Action::ExportResults || super::export::available(state))
        .filter_map(|action| {
            Some(PaletteItem {
                label: action.palette_label()?.to_string(),
                category: action.palette_category(),
                key: key_for(&action),
                target: PaletteTarget::Action(action),
            })
        })
        .collect();
    items.extend(state.tree.connections.iter().map(|node| PaletteItem {
        label: format!("Open connection: {}", node.title()),
        category: "connection",
        key: None,
        target: PaletteTarget::OpenConnection(node.record.id.clone()),
    }));
    items.extend(state.sessions.iter().enumerate().map(|(i, session)| {
        let n = u8::try_from(i + 1).unwrap_or(u8::MAX);
        let action = Action::SwitchSession(n);
        PaletteItem {
            label: format!("Switch to session {}: {}", i + 1, session.title),
            category: "session",
            key: (n <= 9).then(|| key_for(&action)).flatten(),
            target: PaletteTarget::Action(action),
        }
    }));
    // The tables and views the active session has loaded.
    let session = state.sessions.active();
    let qualify = session.schema.catalogs().is_some_and(|c| c.len() > 1);
    items.extend(session.schema.loaded_objects().map(|object| {
        let kind = if object.kind.is_view() {
            "View"
        } else {
            "Table"
        };
        let name = if qualify {
            format!("{}.{}", object.catalog, object.name)
        } else {
            object.name.clone()
        };
        PaletteItem {
            label: format!("{kind}: {name}"),
            category: "table",
            key: None,
            target: PaletteTarget::Table {
                session: session.id,
                catalog: object.catalog.clone(),
                object: object.name.clone(),
            },
        }
    }));
    let here = &state.sessions.active().connection_id;
    items.extend(
        qry_core::snippets::for_connection(&state.snippets, here)
            .into_iter()
            .map(|(i, snippet)| PaletteItem {
                label: format!("snippet: {}", snippet.name),
                category: "snippets",
                key: None,
                target: PaletteTarget::Snippet(i),
            }),
    );
    items.extend(PaletteItem::arguments());
    items
}

/// Opens the palette, or closes it if it is open.
pub(super) fn toggle(state: &mut State) {
    if palette_mut(state).is_some() {
        close(state);
        return;
    }
    let palette = PaletteState::new(items(state), state.palette_recent.clone());
    state.overlays.retain(|o| matches!(o, Overlay::Dialog(_)));
    state.overlays.push(Overlay::Palette(palette));
}

fn close(state: &mut State) {
    state.overlays.retain(|o| !matches!(o, Overlay::Palette(_)));
}

fn palette_mut(state: &mut State) -> Option<&mut PaletteState> {
    match state.overlays.last_mut() {
        Some(Overlay::Palette(palette)) => Some(palette),
        _ => None,
    }
}

/// Types `c` into the query.
pub(super) fn type_char(state: &mut State, c: char) {
    if let Some(palette) = palette_mut(state) {
        palette.input.insert(c);
        palette.refilter();
    }
}

/// What a `Palette` binding does.
pub(super) fn run_key(state: &mut State, action: Action, fx: &mut Vec<Effect>) {
    let Some(palette) = palette_mut(state) else {
        return;
    };
    let page = isize::try_from(crate::palette::MAX_ROWS).unwrap_or(1);
    let edited = match action {
        Action::Up => {
            palette.move_by(-1);
            false
        }
        Action::Down => {
            palette.move_by(1);
            false
        }
        Action::PageUp => {
            palette.move_by(-page);
            false
        }
        Action::PageDown => {
            palette.move_by(page);
            false
        }
        Action::NextField => {
            palette.fill();
            false
        }
        Action::Left => {
            palette.input.move_left();
            false
        }
        Action::Right => {
            palette.input.move_right();
            false
        }
        Action::LineStart => {
            palette.input.move_home();
            false
        }
        Action::LineEnd => {
            palette.input.move_end();
            false
        }
        Action::DeleteBack => {
            palette.input.delete_back();
            true
        }
        Action::DeleteForward => {
            palette.input.delete_forward();
            true
        }
        Action::Cancel | Action::Palette => {
            close(state);
            return;
        }
        Action::Submit => {
            submit(state, false, fx);
            return;
        }
        // A snippet is inserted and run; anything else runs as on Enter.
        Action::SubmitAndRun => {
            submit(state, true, fx);
            return;
        }
        _ => false,
    };
    if edited && let Some(palette) = palette_mut(state) {
        palette.refilter();
    }
}

/// What running an item comes to, once its argument is read.
enum Run {
    Action(Action),
    OpenConnection(String),
    Table(crate::app::state::SessionId, String, String),
    Snippet(usize),
    Export(qry_core::ExportConfig),
    Goto(usize),
    Theme(String),
}

/// Reads the selected item and its argument, or says what is wrong.
fn pick(palette: &PaletteState) -> Result<(String, Run), String> {
    let item = palette.selected_item().ok_or("no command matches")?;
    let label = item.label.clone();
    let run = match &item.target {
        PaletteTarget::Action(action) => Run::Action(*action),
        PaletteTarget::OpenConnection(id) => Run::OpenConnection(id.clone()),
        PaletteTarget::Table {
            session,
            catalog,
            object,
        } => Run::Table(*session, catalog.clone(), object.clone()),
        PaletteTarget::Snippet(i) => Run::Snippet(*i),
        PaletteTarget::Argument { hint, kind, .. } => {
            let arg = palette.argument().map(|(_, arg)| arg.trim()).unwrap_or("");
            if arg.is_empty() {
                return Err(format!("needs <{hint}>"));
            }
            match kind {
                ArgKind::Export(etype, separator) => {
                    Run::Export(qry_core::ExportConfig::new(arg, *separator, *etype))
                }
                ArgKind::Goto => match arg.parse::<usize>() {
                    Ok(row) if row > 0 => Run::Goto(row),
                    _ => return Err(format!("not a row number: {arg}")),
                },
                ArgKind::Theme => {
                    if !Theme::NAMES.contains(&arg) {
                        return Err(format!(
                            "no theme {arg}; there is {}",
                            Theme::NAMES.join(", ")
                        ));
                    }
                    Run::Theme(arg.to_string())
                }
            }
        }
    };
    Ok((label, run))
}

/// Runs the selected item and closes the palette, or keeps it open with
/// why it cannot. A snippet is inserted, and run too if `run`.
fn submit(state: &mut State, run_snippet: bool, fx: &mut Vec<Effect>) {
    let Some(palette) = palette_mut(state) else {
        return;
    };
    let (label, run) = match pick(palette) {
        Ok(picked) => picked,
        Err(error) => {
            palette.error = Some(error);
            return;
        }
    };
    remember(&mut state.palette_recent, &label);
    close(state);
    match run {
        // As its key would, in the pane it belongs to.
        Run::Action(action) => act(state, action.home_context(), action, fx),
        Run::OpenConnection(id) => {
            if let Some(i) = state.tree.position(&id) {
                let record = state.tree.connections[i].record.clone();
                sessions::open_or_switch(state, &record, fx);
            }
        }
        Run::Table(session, catalog, object) => {
            super::tree::select_object(state, session, &catalog, &object, true, fx);
        }
        Run::Snippet(i) => super::snippets::insert(state, i, run_snippet, fx),
        Run::Export(config) => super::export::quick(state, config, fx),
        Run::Goto(row) => {
            let results = state.results_mut();
            match results.result.as_ref().map(|r| r.rows.len()) {
                Some(rows) if rows > 0 => results.row = (row - 1).min(rows - 1),
                _ => notify(state, Notice::warning("There are no results to go to"), fx),
            }
        }
        Run::Theme(name) => notify(
            state,
            Notice::info(format!("Drawing with the {name} theme")),
            fx,
        ),
    }
}
