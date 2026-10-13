//! The reducer: `UiEvent → App::handle → Vec<Effect>`.
//!
//! [`App`] owns the whole UI [`State`] as plain data, and [`App::handle`] is
//! the one place it changes. `handle` is synchronous and deterministic: it
//! never reads the clock (time arrives inside events) and performs no I/O.
//! What should happen outside is returned as [`Effect`]s, which the
//! [runtime](crate::runtime) carries out; their outcomes come back as
//! [`UiEvent`]s.
//!
//! This module must not import the async runtime, the filesystem, the
//! network, the clock or the keychain; `purity_tests` enforces it.

pub(crate) mod dialogs;
pub mod effects;
pub mod event;
mod reducer;
pub mod state;

use std::sync::Arc;

pub use self::{
    effects::Effect,
    event::{DbAnswer, FileAnswer, Timer, UiEvent},
    state::{SessionId, SessionStatus, State, StatusCode},
};
use crate::{config::Config, theme::Theme};

/// The app: its state, and the reducer that changes it.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub(crate) state: State,
}

impl App {
    /// An app with this config and theme, before [`UiEvent::Started`].
    /// `seed` makes the connection ids it mints differ between runs; see
    /// [`connections::id_seed`](crate::connections::id_seed).
    pub fn new(config: Config, theme: Theme, seed: u64) -> Self {
        let mut state = State {
            config: Arc::new(config),
            theme,
            ids: state::IdGen::new(seed),
            ..State::default()
        };
        reducer::restyle(&mut state);
        Self { state }
    }

    /// Reacts to one event: changes the state and says what should happen
    /// outside. The only entry point for input.
    pub fn handle(&mut self, event: UiEvent) -> Vec<Effect> {
        reducer::handle(&mut self.state, event)
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Draws with `theme` from now on.
    pub fn set_theme(&mut self, theme: Theme) {
        self.state.theme = theme;
        self.state.dirty = true;
        reducer::restyle(&mut self.state);
    }

    /// Whether anything changed since the last call, for the runtime to know
    /// whether to draw. Clears the flag.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.state.dirty)
    }
}

#[cfg(test)]
impl App {
    /// The built-in config and the dark theme, before `Started`, as most
    /// unit tests want it.
    pub(crate) fn embedded() -> Self {
        Self::new(Config::embedded(), Theme::DARK, 0)
    }
}

#[cfg(test)]
#[path = "reducer_tests.rs"]
mod reducer_tests;

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod overlay_tests;

#[cfg(test)]
#[path = "screen_tests.rs"]
mod screen_tests;

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod sessions_tests;

#[cfg(test)]
#[path = "purity_tests.rs"]
mod purity_tests;

#[cfg(test)]
#[path = "determinism_tests.rs"]
mod determinism_tests;
