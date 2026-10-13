//! Drawing code: free functions that read the [`State`] and a [`Theme`] and
//! own nothing. Where something scrolls, the view keeps the state's scroll
//! position in step with what fits as it draws; otherwise it changes nothing.

pub(crate) mod editor;
pub(crate) mod form;
pub(crate) mod help;
pub(crate) mod results;
pub(crate) mod status;
#[cfg(test)]
mod swatch;
pub(crate) mod tree;

use ratatui::{prelude::*, widgets::Block};

use crate::{
    app::state::{Overlay, State},
    theme::Theme,
};

/// Draws the main screen: the panes, and whatever is on top of them.
pub fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    // The app background and default text, under everything else.
    frame.render_widget(Block::new().style(theme.base()), area);
    let status_height = if status::is_empty(state) { 0 } else { 3 };

    let [all, statuspanel] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(status_height)]).areas(area);
    let [connpanel, rest] =
        Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)]).areas(all);
    let [querypane, resultspane] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Fill(1)]).areas(rest);

    tree::draw(state, theme, frame, connpanel);
    editor::draw(state, theme, frame, querypane);
    results::draw(state, theme, frame, resultspane);
    status::draw(state, theme, frame, statuspanel);

    for overlay in &state.overlays {
        match overlay {
            Overlay::Dialog(dialog) => form::draw(dialog.form(), theme, frame, area),
            Overlay::Help(help) => help::draw(help, theme, frame, area),
        }
    }
}

#[cfg(test)]
#[path = "views/views_tests.rs"]
mod views_tests;
