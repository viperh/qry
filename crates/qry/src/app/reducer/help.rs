//! The help popup. Built from the config, so it always lists the keys that
//! are actually bound, including a user's own.

use crate::{
    action::Action,
    app::state::{HelpState, Overlay, State},
    config::{Config, KeySection},
    keymap::{HelpCommand, display_key},
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

pub(super) fn scroll(state: &mut State, command: HelpCommand) {
    if let Some(help) = state.help() {
        help.scroll(command);
    }
}

impl HelpState {
    pub(crate) fn from_config(config: &Config) -> Self {
        let panes = &config.panes;
        let sections = [
            ("Global", mode_bindings(config, KeySection::Home)),
            ("Connection tree", pane_bindings(panes.tree.describe())),
            ("Editor", pane_bindings(panes.editor.describe())),
            ("Results", pane_bindings(panes.results.describe())),
            // Every modal shares these keys, so they are listed once.
            ("Forms", pane_bindings(panes.form.describe())),
            (
                "New Connection",
                mode_bindings(config, KeySection::AddConnModal),
            ),
            ("Export", mode_bindings(config, KeySection::ExpoModal)),
            ("Password", mode_bindings(config, KeySection::PasswordModal)),
            ("Import", mode_bindings(config, KeySection::ImportModal)),
            (
                "Export Connections",
                mode_bindings(config, KeySection::ConnExportModal),
            ),
            ("Help", pane_bindings(panes.help.describe())),
        ];
        Self {
            sections: sections
                .into_iter()
                .filter(|(_, binds)| !binds.is_empty())
                .map(|(title, binds)| (title.to_string(), binds))
                .collect(),
            ..Self::default()
        }
    }

    /// Scrolls by a line or a page, as far as the last draw showed there is
    /// to see. The scroll position lives in a cell because the view keeps it
    /// within what fits.
    pub(crate) fn scroll(&self, command: HelpCommand) {
        let page = self.page.get().max(1);
        let scroll = self.scroll.get();
        let scroll = match command {
            HelpCommand::ScrollUp => scroll.saturating_sub(1),
            HelpCommand::ScrollDown => scroll + 1,
            HelpCommand::PageUp => scroll.saturating_sub(page),
            HelpCommand::PageDown => scroll + page,
            HelpCommand::Close => scroll,
        };
        self.scroll.set(scroll.min(self.max_scroll.get()));
    }
}

fn pane_bindings(binds: Vec<(String, &'static str)>) -> Vec<(String, String)> {
    binds
        .into_iter()
        .map(|(keys, desc)| (keys, desc.to_string()))
        .collect()
}

/// A section's bindings as help lines, keys for the same action on one
/// line, sorted by description.
fn mode_bindings(config: &Config, section: KeySection) -> Vec<(String, String)> {
    let Some(bindings) = config.keybindings.0.get(&section) else {
        return Vec::new();
    };
    let mut by_action: Vec<(String, Vec<String>)> = Vec::new();
    for (sequence, action) in bindings {
        let desc = describe_action(action);
        let keys = sequence
            .iter()
            .map(display_key)
            .collect::<Vec<_>>()
            .join(" ");
        match by_action.iter_mut().find(|(d, _)| *d == desc) {
            Some((_, all)) => all.push(keys),
            None => by_action.push((desc, vec![keys])),
        }
    }
    by_action.sort();
    by_action
        .into_iter()
        .map(|(desc, mut keys)| {
            keys.sort_by_key(|k| (k.len() == 1, k.clone()));
            (keys.join(" / "), desc)
        })
        .collect()
}

fn describe_action(action: &Action) -> String {
    match action {
        Action::Quit => "Quit".into(),
        Action::Help => "Show / hide this help".into(),
        Action::FocusNext => "Next pane".into(),
        Action::FocusPrev => "Previous pane".into(),
        Action::ChangeMode(KeySection::AddConnModal) => "New connection".into(),
        Action::ChangeMode(KeySection::Home) => "Close".into(),
        Action::ChangeMode(KeySection::ExpoModal) => "Export to file".into(),
        Action::ChangeMode(KeySection::PasswordModal) => "Ask for a password".into(),
        Action::ChangeMode(KeySection::ImportModal) => "Import connections".into(),
        Action::ChangeMode(KeySection::ConnExportModal) => "Export connections".into(),
        Action::Suspend => "Suspend".into(),
        Action::ClearScreen => "Redraw the screen".into(),
        Action::Execute(sql) => format!("Run `{sql}`"),
    }
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;
