//! The help popup. Built from the config, so it always lists the keys that
//! are actually bound, including a user's own.

use crate::{
    action::Action,
    app::state::{HelpState, Overlay, State},
    config::Config,
    keymap::{KeyContext, chord_to_string},
};

/// The contexts the help lists, with their titles. The inspector, the jobs
/// drawer and the palette are left out until there is one to open.
const SECTIONS: [(KeyContext, &str); 6] = [
    (KeyContext::Global, "Global"),
    (KeyContext::Tree, "Connection tree"),
    (KeyContext::Editor, "Editor"),
    (KeyContext::Results, "Results"),
    (KeyContext::Dialog, "Dialogs"),
    (KeyContext::Help, "Help"),
];

/// Shows the help from the top, or hides it.
pub(super) fn toggle(state: &mut State) {
    if state.help().is_some() {
        close(state);
    } else {
        state
            .overlays
            .push(Overlay::Help(HelpState::from_config(&state.config)));
    }
}

pub(super) fn close(state: &mut State) {
    state
        .overlays
        .retain(|overlay| !matches!(overlay, Overlay::Help(_)));
}

/// What a `Help` binding does: scroll, or close the help.
pub(super) fn act(state: &mut State, action: Action) {
    match action {
        Action::Cancel => close(state),
        action => {
            if let Some(help) = state.help() {
                help.scroll(action);
            }
        }
    }
}

impl HelpState {
    /// One section per context, one line per action in the actions' order,
    /// with every chord bound to it. A pane's binding and the `Global` one
    /// it shadows are both listed, each under its own context.
    pub(crate) fn from_config(config: &Config) -> Self {
        let sections = SECTIONS
            .into_iter()
            .map(|(context, title)| (title.to_string(), bindings(config, context)))
            .filter(|(_, binds)| !binds.is_empty())
            .collect();
        Self {
            sections,
            ..Self::default()
        }
    }

    /// Scrolls by a line or a page, or to either end, as far as the last
    /// draw showed there is to see. The scroll position lives in a cell
    /// because the view keeps it within what fits.
    pub(crate) fn scroll(&self, action: Action) {
        let page = self.page.get().max(1);
        let scroll = self.scroll.get();
        let scroll = match action {
            Action::Up => scroll.saturating_sub(1),
            Action::Down => scroll + 1,
            Action::PageUp => scroll.saturating_sub(page),
            Action::PageDown => scroll + page,
            Action::Top => 0,
            Action::Bottom => usize::MAX,
            _ => scroll,
        };
        self.scroll.set(scroll.min(self.max_scroll.get()));
    }
}

/// `context`'s bindings as help lines: an action's chords on one line,
/// plain and short ones first. Actions that read the same (the numbered
/// sessions) share a line, written as a range: `ctrl-1 … ctrl-9`.
fn bindings(config: &Config, context: KeyContext) -> Vec<(String, String)> {
    let mut actions: Vec<Action> = config
        .keymap
        .bindings(context)
        .map(|(_, action)| *action)
        .collect();
    actions.sort_by_key(Action::rank);
    actions.dedup();
    // The chords of each action, and the description they share.
    let mut lines: Vec<(Vec<Vec<String>>, &'static str)> = Vec::new();
    for action in actions {
        let chords = config.keymap.chords_for(context, &action);
        let keys = chords.iter().map(|chord| chord_to_string(chord)).collect();
        match lines.last_mut() {
            Some((all, desc)) if *desc == action.description() => all.push(keys),
            _ => lines.push((vec![keys], action.description())),
        }
    }
    lines
        .into_iter()
        .map(|(mut keys, desc)| {
            let keys = match keys.as_mut_slice() {
                [one] => one.join(" / "),
                [first, .., last] => format!("{} … {}", first.join(" / "), last.join(" / ")),
                [] => String::new(),
            };
            (keys, desc.to_string())
        })
        .collect()
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;
