//! The help overlay's state. Its content comes from `help_model::build`
//! over the live keymap, so it always lists the keys that are actually
//! bound, including a user's own.

use crate::{
    action::Action,
    app::state::{HelpState, Overlay, State},
    config::Config,
    help_model,
};

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
    /// Everything the help lists, from the keymap as it is now, from the
    /// top.
    pub(crate) fn from_config(config: &Config) -> Self {
        let keymap = &config.keymap;
        Self {
            groups: help_model::build(keymap),
            footer: help_model::footer(keymap),
            keys: help_model::help_keys(keymap),
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

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;
