//! Drawing code: free functions that read the [`State`] and a [`Theme`] and
//! own nothing. Where something scrolls, the view keeps the state's scroll
//! position in step with what fits as it draws; otherwise it changes nothing.

pub(crate) mod editor;
pub(crate) mod form;
pub(crate) mod help;
pub(crate) mod hints;
pub(crate) mod layout;
pub(crate) mod pane_title;
pub(crate) mod results;
pub(crate) mod status_line;
#[cfg(test)]
mod swatch;
pub(crate) mod top_bar;
pub(crate) mod tree;

use ratatui::{prelude::*, widgets::Block};

use crate::{
    app::state::{Focus, Overlay, State},
    theme::Theme,
};

/// Draws the main screen: the chrome, the panes, and whatever is on top of
/// them. Below [`layout::MIN_SIZE`] only a message saying so is drawn.
pub fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let Some(screen) = layout::compute_layout(&state.layout_input(area)) else {
        layout::render_too_small(frame, area, theme);
        return;
    };
    // The app background and default text, under everything else.
    frame.render_widget(Block::new().style(theme.base()), area);

    top_bar::draw(state, theme, frame, screen.top_bar);
    tree::draw(state, theme, frame, screen.tree);
    let divider = |focused: bool| {
        if focused {
            theme.divider_focused()
        } else {
            theme.divider()
        }
    };
    layout::render_vertical_divider(
        frame,
        screen.tree_divider,
        divider(state.focus == Focus::Tree),
    );
    if !screen.editor_title.is_empty() {
        let detail = editor::position(state);
        pane_title::draw(
            "QUERY",
            &detail,
            state.focus == Focus::Editor,
            theme,
            frame,
            screen.editor_title,
        );
        editor::draw(state, theme, frame, screen.editor);
    }
    if !screen.results_title.is_empty() {
        let detail = results::summary(state);
        pane_title::draw(
            "RESULTS",
            &detail,
            state.focus == Focus::Results,
            theme,
            frame,
            screen.results_title,
        );
        results::draw(state, theme, frame, screen.results);
    }
    if let (Some(divider_area), Some(inspector)) = (screen.inspector_divider, screen.inspector) {
        layout::render_vertical_divider(frame, divider_area, theme.divider());
        // T304 fills it.
        let [title, _] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inspector);
        pane_title::draw("INSPECTOR", "", false, theme, frame, title);
    }
    status_line::draw(state, theme, frame, screen.status);
    if let Some(hints) = screen.hints {
        hints::draw(state, theme, frame, hints);
    }

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
